mod config;
mod learner;
mod peer;
mod read_token;

use super::{CatalogRaftConfig, HttpResponse, error, json_response};
use crate::catalog::Catalog;
use crate::replication::raft::{
    self, RaftCatalogStateMachine, RaftCommand, RaftCommandPrecondition, RaftHttpNetworkFactory,
    RaftLogStore, RaftRejection, RaftResponseResult, TypeConfig,
};
use openraft::{BasicNode, Raft};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tiny_http::Request;
use tokio::runtime::{Builder, Runtime};

type Node = Raft<TypeConfig>;
pub(super) const RPC_TIMEOUT: Duration = Duration::from_secs(10);
pub(super) const SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(60);
pub(super) const MIN_READ_TOKEN_HEADER: &str = "X-TXBASE-Raft-Min-Read-Token";
pub(super) const READ_TOKEN_HEADER: &str = "X-TXBASE-Raft-Read-Token";

pub(super) enum ReadPositionError {
    InvalidToken,
    ClusterMismatch,
    Unavailable(String),
}

#[derive(Clone)]
pub(super) struct RaftRuntime {
    pub(super) node: Arc<Node>,
    pub(super) runtime: Arc<Runtime>,
    pub(super) node_id: u64,
    cluster_id: String,
    initial_members: BTreeSet<u64>,
    genesis_fingerprint: Arc<RwLock<Vec<u8>>>,
    local_genesis_fingerprint: Vec<u8>,
    state_machine: RaftCatalogStateMachine,
    log_store: RaftLogStore,
    network_factory: RaftHttpNetworkFactory,
    token: String,
    peer_tls: bool,
    membership_change_lock: Arc<tokio::sync::Mutex<()>>,
}

impl RaftRuntime {
    pub(super) fn start(
        catalog_root: &Path,
        config: CatalogRaftConfig,
        token: Option<String>,
    ) -> Result<Self, String> {
        Self::start_with_payload_entry_limit(catalog_root, config, token, 1)
    }

    #[cfg(test)]
    pub(super) fn start_with_max_payload_entries(
        catalog_root: &Path,
        config: CatalogRaftConfig,
        token: Option<String>,
        max_payload_entries: u64,
    ) -> Result<Self, String> {
        Self::start_with_payload_entry_limit(catalog_root, config, token, max_payload_entries)
    }

    fn start_with_payload_entry_limit(
        catalog_root: &Path,
        config: CatalogRaftConfig,
        token: Option<String>,
        max_payload_entries: u64,
    ) -> Result<Self, String> {
        let token =
            token.ok_or_else(|| "Raft peer RPC requires TXBASE_REPLICATION_TOKEN".to_owned())?;
        let peer_tls = config::validate_config(catalog_root, &config, &token)?;
        config::bind_node_identity(&config.node_directory, config.node_id, &config.cluster_id)?;

        let log_store = RaftLogStore::open(&config.node_directory)
            .map_err(|error| format!("cannot open Raft node directory: {error}"))?;
        let catalog = Catalog::from_path(catalog_root)
            .map_err(|error| format!("cannot open Raft catalog: {error}"))?;
        let state = match catalog
            .read_sidecar_bytes(raft::RAFT_STATE_SIDECAR_NAME)
            .map_err(|error| error.to_string())?
        {
            Some(_) => RaftCatalogStateMachine::open(catalog),
            None if config.bootstrap || config.initialize_catalog => {
                RaftCatalogStateMachine::initialize(catalog)
            }
            None => RaftCatalogStateMachine::open(catalog),
        }
        .map_err(|error| format!("cannot open Raft state machine: {error}"))?;
        let fingerprint_catalog = Catalog::from_path(catalog_root)
            .map_err(|error| format!("cannot reopen Raft catalog: {error}"))?;
        let genesis_fingerprint = raft::raft_genesis_fingerprint(&fingerprint_catalog)?;
        let local_genesis_fingerprint = genesis_fingerprint.clone();

        let genesis_fingerprint = Arc::new(RwLock::new(genesis_fingerprint.clone()));
        let mut network = RaftHttpNetworkFactory::new_with_fingerprint(
            config.cluster_id.clone(),
            config.node_id,
            Arc::clone(&genesis_fingerprint),
            token.clone(),
        )?;
        if config.tls_client_ca.is_some() {
            network = network.with_client_certificate_files(
                config
                    .tls_certificate
                    .as_deref()
                    .expect("validated TLS certificate"),
                config
                    .tls_private_key
                    .as_deref()
                    .expect("validated TLS private key"),
            );
        }
        let network_factory = network.clone();
        let join_state_machine = state.clone();
        let join_log_store = log_store.clone();
        let raft_config = openraft::Config {
            cluster_name: config.cluster_id.clone(),
            max_payload_entries,
            ..openraft::Config::default()
        };
        let raft_config = Arc::new(
            raft_config
                .validate()
                .map_err(|error| format!("invalid OpenRaft configuration: {error}"))?,
        );
        let runtime = Arc::new(
            Builder::new_multi_thread()
                .enable_all()
                .build()
                .map_err(|error| format!("cannot create Tokio runtime: {error}"))?,
        );
        let members = config
            .initial_members
            .iter()
            .map(|(id, address)| (*id, BasicNode::new(address)))
            .collect::<BTreeMap<_, _>>();
        let node = runtime.block_on(async move {
            let node = Node::new(config.node_id, raft_config, network, log_store, state)
                .await
                .map_err(|error| format!("cannot start OpenRaft node: {error}"))?;
            if config.bootstrap
                && !node
                    .is_initialized()
                    .await
                    .map_err(|error| error.to_string())?
            {
                node.initialize(members)
                    .await
                    .map_err(|error| format!("cannot initialize Raft cluster: {error}"))?;
            }
            Ok::<_, String>(node)
        })?;

        Ok(Self {
            node: Arc::new(node),
            runtime,
            node_id: config.node_id,
            cluster_id: config.cluster_id,
            initial_members: config.initial_members.keys().copied().collect(),
            genesis_fingerprint,
            local_genesis_fingerprint,
            state_machine: join_state_machine,
            log_store: join_log_store,
            network_factory,
            token,
            peer_tls,
            membership_change_lock: Arc::new(tokio::sync::Mutex::new(())),
        })
    }

    pub(super) fn linearizable_read(&self) -> Result<(), String> {
        self.runtime
            .block_on(async {
                tokio::time::timeout(RPC_TIMEOUT, self.node.ensure_linearizable()).await
            })
            .map_err(|_| "Raft read barrier timed out".to_owned())?
            .map(|_| ())
            .map_err(|error| format!("Raft read barrier failed: {error}"))
    }

    pub(super) fn prepare_read(&self, token: Option<&str>) -> Result<(), ReadPositionError> {
        let Some(token) = token else {
            return self
                .linearizable_read()
                .map_err(ReadPositionError::Unavailable);
        };
        let index = read_token::parse(token, &self.cluster_id).map_err(|error| match error {
            read_token::Error::Invalid => ReadPositionError::InvalidToken,
            read_token::Error::ClusterMismatch => ReadPositionError::ClusterMismatch,
        })?;
        self.runtime
            .block_on(
                self.node
                    .wait(Some(RPC_TIMEOUT))
                    .applied_index_at_least(Some(index), "Raft read token"),
            )
            .map(|_| ())
            .map_err(|error| {
                ReadPositionError::Unavailable(format!(
                    "timed out waiting for Raft read token: {error}"
                ))
            })
    }

    pub(super) fn current_read_token(&self) -> Result<String, ReadPositionError> {
        let log_id = self
            .state_machine
            .last_applied_log_id()
            .map_err(ReadPositionError::Unavailable)?
            .ok_or_else(|| {
                ReadPositionError::Unavailable(
                    "Raft has no applied log position for a read token".into(),
                )
            })?;
        Ok(read_token::encode(&self.cluster_id, log_id.index))
    }

    #[cfg(test)]
    pub(super) fn shutdown(&self) -> Result<(), String> {
        self.runtime
            .block_on(self.node.shutdown())
            .map_err(|error| format!("cannot stop OpenRaft node: {error}"))
    }

    #[cfg(test)]
    pub(super) fn set_peer_blocked(&self, peer_id: u64, blocked: bool) -> Result<(), String> {
        self.network_factory.set_peer_blocked(peer_id, blocked)
    }

    #[cfg(test)]
    pub(super) fn delay_next_append_entries(
        &self,
        peer_id: u64,
    ) -> Result<raft::AppendDelayHandle, String> {
        self.network_factory.delay_next_append_entries(peer_id)
    }

    #[cfg(test)]
    pub(super) fn delay_append_entries_at(
        &self,
        peer_id: u64,
        log_index: u64,
    ) -> Result<raft::AppendDelayHandle, String> {
        self.network_factory
            .delay_append_entries_at(peer_id, log_index)
    }

    #[cfg(test)]
    pub(super) fn delay_next_uniform_membership_append(
        &self,
        peer_id: u64,
    ) -> Result<raft::AppendDelayHandle, String> {
        self.network_factory
            .delay_next_uniform_membership_append(peer_id)
    }

    pub(super) fn propose_request(
        &self,
        request: &Request,
        catalog: &Catalog,
        steps: Vec<crate::xbase::TransactionStep>,
        precondition: Option<RaftCommandPrecondition>,
        request_fingerprint: Option<Vec<u8>>,
    ) -> Result<u64, HttpResponse> {
        let (client_id, sequence) = client_request_identity(request)?;
        let (_, expected_catalog_tag) =
            catalog.schema_representation().map_err(|catalog_error| {
                json_response(
                    500,
                    error("catalog_error", &catalog_error.to_string()),
                    false,
                )
            })?;
        let command = RaftCommand::new(
            client_id,
            sequence,
            expected_catalog_tag,
            precondition,
            steps,
        )
        .map_err(|message| json_response(422, error("invalid_transaction", &message), false))?;
        let command = match request_fingerprint {
            Some(fingerprint) => {
                command
                    .with_request_fingerprint(fingerprint)
                    .map_err(|message| {
                        json_response(422, error("invalid_transaction", &message), false)
                    })?
            }
            None => command,
        };
        let response = self
            .runtime
            .block_on(async {
                tokio::time::timeout(RPC_TIMEOUT, self.node.client_write(command)).await
            })
            .map_err(|_| {
                json_response(
                    503,
                    error(
                        "raft_unavailable",
                        "Raft write timed out; retry with the same client ID and sequence",
                    ),
                    false,
                )
            })?
            .map_err(|raft_error| {
                json_response(
                    503,
                    error("raft_unavailable", &raft_error.to_string()),
                    false,
                )
            })?
            .data
            .ok_or_else(|| {
                json_response(
                    500,
                    error("raft_error", "Raft returned no application response"),
                    false,
                )
            })?;
        #[cfg(test)]
        crate::test_support::crash_at("raft_response_received");
        match response.result {
            RaftResponseResult::Applied { transaction_id } => Ok(transaction_id),
            RaftResponseResult::Rejected { reason } => Err(rejection_response(reason)),
        }
    }

    pub(super) fn replayed_transaction(
        &self,
        request: &Request,
        catalog: &Catalog,
        fingerprint: &[u8],
    ) -> Result<Option<u64>, HttpResponse> {
        let (client_id, sequence) = client_request_identity(request)?;
        self.linearizable_read()
            .map_err(|message| json_response(503, error("raft_unavailable", &message), false))?;
        let previous = raft::raft_client_result(catalog, &client_id)
            .map_err(|message| json_response(500, error("raft_state_error", &message), false))?;
        let Some((previous_sequence, previous_fingerprint, response)) = previous else {
            return if sequence == 1 {
                Ok(None)
            } else {
                Err(rejection_response(RaftRejection::ClientSequenceGap))
            };
        };
        if sequence == previous_sequence {
            if fingerprint != previous_fingerprint {
                return Err(rejection_response(RaftRejection::ClientSequenceConflict));
            }
            return match response {
                RaftResponseResult::Applied { transaction_id } => Ok(Some(transaction_id)),
                RaftResponseResult::Rejected { reason } => Err(rejection_response(reason)),
            };
        }
        if sequence < previous_sequence {
            return Err(rejection_response(RaftRejection::ClientSequenceTooOld));
        }
        if previous_sequence.checked_add(1) == Some(sequence) {
            Ok(None)
        } else {
            Err(rejection_response(RaftRejection::ClientSequenceGap))
        }
    }
}

fn client_request_identity(request: &Request) -> Result<(String, u64), HttpResponse> {
    let client_id = super::request_header(request, "X-Txbase-Client-Id")
        .ok_or_else(|| {
            json_response(
                400,
                error(
                    "missing_client_id",
                    "Raft writes require X-Txbase-Client-Id",
                ),
                false,
            )
        })?
        .to_owned();
    let sequence = super::request_header(request, "X-Txbase-Client-Sequence")
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|sequence| *sequence > 0)
        .ok_or_else(|| {
            json_response(
                400,
                error(
                    "invalid_client_sequence",
                    "Raft writes require a positive X-Txbase-Client-Sequence",
                ),
                false,
            )
        })?;
    Ok((client_id, sequence))
}

fn rejection_response(reason: RaftRejection) -> HttpResponse {
    match reason {
        RaftRejection::CatalogChanged => json_response(
            409,
            error(
                "catalog_changed",
                "the catalog changed before the Raft command committed",
            ),
            false,
        ),
        RaftRejection::PreconditionFailed => json_response(
            412,
            error(
                "precondition_failed",
                "If-Match or If-None-Match does not permit the current representation",
            ),
            false,
        ),
        RaftRejection::ClientSequenceConflict => json_response(
            409,
            error(
                "client_sequence_conflict",
                "the client sequence was already used by a different command",
            ),
            false,
        ),
        RaftRejection::ClientSequenceTooOld => json_response(
            409,
            error(
                "client_sequence_too_old",
                "the client sequence is older than the latest applied command",
            ),
            false,
        ),
        RaftRejection::ClientSequenceGap => json_response(
            409,
            error(
                "client_sequence_gap",
                "client commands must use consecutive sequence numbers",
            ),
            false,
        ),
        RaftRejection::InvalidTransaction => json_response(
            422,
            error(
                "invalid_transaction",
                "the Raft state machine rejected the transaction",
            ),
            false,
        ),
    }
}
