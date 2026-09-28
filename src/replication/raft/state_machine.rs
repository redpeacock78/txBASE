use super::{RaftResponse, RaftResponseResult, TypeConfig};
use crate::catalog::Catalog;
use crate::replication::MAX_REPLICATION_SNAPSHOT_BYTES;
use openraft::storage::RaftStateMachine;
use openraft::{
    BasicNode, Entry, ErrorSubject, ErrorVerb, LogId, Snapshot, SnapshotMeta, StorageError,
    StoredMembership,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Cursor;
use std::sync::{Arc, Mutex};

mod apply;
use apply::apply_entries;

pub const RAFT_STATE_SIDECAR_NAME: &str = ".txbase.raft-state";
pub(super) const RAFT_SNAPSHOT_SIDECAR_NAME: &str = ".txbase.raft-snapshot";
const STATE_MAGIC: &[u8; 4] = b"TXRA";
const STATE_VERSION: u8 = 1;

pub(super) type SharedCatalog = Arc<Mutex<Catalog>>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ClientResult {
    sequence: u64,
    fingerprint: Vec<u8>,
    response: RaftResponse,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RaftApplicationState {
    pub(super) last_applied: Option<LogId<u64>>,
    pub(super) last_membership: StoredMembership<u64, BasicNode>,
    pub(super) catalog_transaction_id: u64,
    genesis: bool,
    clients: BTreeMap<String, ClientResult>,
}

impl Default for RaftApplicationState {
    fn default() -> Self {
        Self {
            last_applied: None,
            last_membership: StoredMembership::default(),
            catalog_transaction_id: 0,
            genesis: true,
            clients: BTreeMap::new(),
        }
    }
}

impl RaftApplicationState {
    pub(super) fn to_bytes(&self) -> Result<Vec<u8>, String> {
        self.validate()?;
        let payload = serde_json::to_vec(self).map_err(|error| error.to_string())?;
        let mut bytes = Vec::with_capacity(5 + payload.len());
        bytes.extend_from_slice(STATE_MAGIC);
        bytes.push(STATE_VERSION);
        bytes.extend_from_slice(&payload);
        // ponytail: retain each client's latest result until a client-retirement protocol exists; fail closed at the shared snapshot ceiling.
        if bytes.len() > MAX_REPLICATION_SNAPSHOT_BYTES {
            return Err(format!(
                "Raft application state exceeds {MAX_REPLICATION_SNAPSHOT_BYTES} bytes"
            ));
        }
        Ok(bytes)
    }

    pub(super) fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_REPLICATION_SNAPSHOT_BYTES {
            return Err(format!(
                "Raft application state exceeds {MAX_REPLICATION_SNAPSHOT_BYTES} bytes"
            ));
        }
        let Some(body) = bytes.strip_prefix(STATE_MAGIC) else {
            return Err("Raft application state header is invalid".into());
        };
        let Some((&version, payload)) = body.split_first() else {
            return Err("Raft application state header is truncated".into());
        };
        if version != STATE_VERSION {
            return Err(format!(
                "unsupported Raft application state version: {version}"
            ));
        }
        let state: Self = serde_json::from_slice(payload).map_err(|error| error.to_string())?;
        state.validate()?;
        Ok(state)
    }

    fn validate(&self) -> Result<(), String> {
        if self.genesis != self.last_applied.is_none() {
            return Err("Raft genesis marker disagrees with the applied log position".into());
        }
        if let Some(membership_log_id) = self.last_membership.log_id() {
            let Some(last_applied) = self.last_applied else {
                return Err("Raft membership has no applied log position".into());
            };
            if membership_log_id.index > last_applied.index {
                return Err("Raft membership is newer than the applied log position".into());
            }
        }
        for (client_id, result) in &self.clients {
            if !valid_client_id(client_id)
                || result.sequence == 0
                || result.fingerprint.len() != 32
                || result.response.sequence != result.sequence
            {
                return Err("Raft client deduplication state is invalid".into());
            }
            if let RaftResponseResult::Applied { transaction_id } = &result.response.result {
                if *transaction_id > self.catalog_transaction_id {
                    return Err("Raft client response is newer than the catalog".into());
                }
            }
        }
        Ok(())
    }
}

pub struct RaftCatalogStateMachine {
    pub(super) catalog: SharedCatalog,
}

impl RaftCatalogStateMachine {
    pub fn open(catalog: Catalog) -> Result<Self, String> {
        if catalog.is_historical() {
            return Err("Raft state machine requires a writable catalog".into());
        }
        let transaction_id = catalog
            .transaction_id()
            .map_err(|error| error.to_string())?
            .unwrap_or(0);
        match catalog
            .read_sidecar_bytes(RAFT_STATE_SIDECAR_NAME)
            .map_err(|error| error.to_string())?
        {
            Some(bytes) => {
                let state = RaftApplicationState::from_bytes(&bytes)?;
                if state.catalog_transaction_id != transaction_id {
                    return Err(format!(
                        "Raft application state is at catalog transaction {}, catalog is at {transaction_id}",
                        state.catalog_transaction_id
                    ));
                }
            }
            None if transaction_id == 0 && catalog.tables().next().is_none() => {
                return Self::initialize(catalog);
            }
            None => {
                return Err(
                    "catalog has data but no Raft application state; refusing to initialize".into(),
                );
            }
        }
        Ok(Self {
            catalog: Arc::new(Mutex::new(catalog)),
        })
    }

    /// Initializes a prepared catalog as a cluster's immutable genesis image.
    pub fn initialize(catalog: Catalog) -> Result<Self, String> {
        if catalog.is_historical() {
            return Err("Raft state machine requires a writable catalog".into());
        }
        if catalog
            .read_sidecar_bytes(RAFT_STATE_SIDECAR_NAME)
            .map_err(|error| error.to_string())?
            .is_some()
        {
            return Err("Raft application state is already initialized".into());
        }
        let transaction_id = catalog
            .transaction_id()
            .map_err(|error| error.to_string())?
            .unwrap_or(0);
        if transaction_id == 0 && catalog.tables().next().is_some() {
            return Err("a non-empty genesis catalog must have a committed snapshot".into());
        }
        let state = RaftApplicationState {
            catalog_transaction_id: transaction_id,
            ..RaftApplicationState::default()
        };
        let bytes = state.to_bytes()?;
        catalog
            .update_sidecars_at_transaction(
                Some(transaction_id),
                vec![(RAFT_STATE_SIDECAR_NAME.into(), None, Some(bytes))],
            )
            .map_err(|error| error.to_string())?;
        Ok(Self {
            catalog: Arc::new(Mutex::new(catalog)),
        })
    }
}

pub(super) fn lock_catalog(
    catalog: &SharedCatalog,
) -> Result<std::sync::MutexGuard<'_, Catalog>, String> {
    catalog
        .lock()
        .map_err(|error| format!("catalog mutex poisoned: {error}"))
}

pub(super) fn read_state(catalog: &Catalog) -> Result<(RaftApplicationState, Vec<u8>), String> {
    let bytes = catalog
        .read_sidecar_bytes(RAFT_STATE_SIDECAR_NAME)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Raft application state sidecar is missing".to_owned())?;
    let state = RaftApplicationState::from_bytes(&bytes)?;
    let transaction_id = catalog
        .transaction_id()
        .map_err(|error| error.to_string())?
        .unwrap_or(0);
    if state.catalog_transaction_id != transaction_id {
        return Err(format!(
            "Raft state catalog transaction {} does not match catalog transaction {transaction_id}",
            state.catalog_transaction_id
        ));
    }
    Ok((state, bytes))
}

pub(super) fn save_state(
    catalog: &Catalog,
    expected_bytes: &[u8],
    state: &RaftApplicationState,
) -> Result<Vec<u8>, String> {
    let bytes = state.to_bytes()?;
    catalog
        .update_sidecars_at_transaction(
            Some(state.catalog_transaction_id),
            vec![(
                RAFT_STATE_SIDECAR_NAME.into(),
                Some(expected_bytes.to_vec()),
                Some(bytes.clone()),
            )],
        )
        .map_err(|error| error.to_string())?;
    Ok(bytes)
}

pub(super) async fn run_blocking<T, F>(
    subject: ErrorSubject<u64>,
    verb: ErrorVerb,
    operation: F,
) -> Result<T, Box<StorageError<u64>>>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    tokio::task::spawn_blocking(operation)
        .await
        .map_err(|error| Box::new(storage_error(subject.clone(), verb, error.to_string())))?
        .map_err(|error| Box::new(storage_error(subject, verb, error)))
}

fn storage_error(
    subject: ErrorSubject<u64>,
    verb: ErrorVerb,
    message: String,
) -> StorageError<u64> {
    StorageError::from_io_error(subject, verb, std::io::Error::other(message))
}

fn valid_client_id(client_id: &str) -> bool {
    !client_id.is_empty()
        && client_id.len() <= super::MAX_RAFT_CLIENT_ID_BYTES
        && client_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

impl RaftStateMachine<TypeConfig> for RaftCatalogStateMachine {
    type SnapshotBuilder = super::snapshot::CatalogSnapshotBuilder;

    async fn applied_state(
        &mut self,
    ) -> Result<(Option<LogId<u64>>, StoredMembership<u64, BasicNode>), StorageError<u64>> {
        let catalog = self.catalog.clone();
        run_blocking(ErrorSubject::StateMachine, ErrorVerb::Read, move || {
            let catalog = lock_catalog(&catalog)?;
            let (state, _) = read_state(&catalog)?;
            Ok((state.last_applied, state.last_membership))
        })
        .await
        .map_err(|error| *error)
    }

    async fn apply<I>(&mut self, entries: I) -> Result<Vec<Option<RaftResponse>>, StorageError<u64>>
    where
        I: IntoIterator<Item = Entry<TypeConfig>> + Send,
        I::IntoIter: Send,
    {
        let entries = entries.into_iter().collect::<Vec<_>>();
        let catalog = self.catalog.clone();
        run_blocking(ErrorSubject::StateMachine, ErrorVerb::Write, move || {
            let mut catalog = lock_catalog(&catalog)?;
            apply_entries(&mut catalog, entries)
        })
        .await
        .map_err(|error| *error)
    }

    async fn get_snapshot_builder(&mut self) -> Self::SnapshotBuilder {
        super::snapshot::CatalogSnapshotBuilder {
            catalog: self.catalog.clone(),
        }
    }

    async fn begin_receiving_snapshot(
        &mut self,
    ) -> Result<Box<Cursor<Vec<u8>>>, StorageError<u64>> {
        Ok(Box::new(Cursor::new(Vec::new())))
    }

    async fn install_snapshot(
        &mut self,
        meta: &SnapshotMeta<u64, BasicNode>,
        snapshot: Box<Cursor<Vec<u8>>>,
    ) -> Result<(), StorageError<u64>> {
        let catalog = self.catalog.clone();
        let meta = meta.clone();
        let bytes = snapshot.into_inner();
        run_blocking(
            ErrorSubject::Snapshot(Some(meta.signature())),
            ErrorVerb::Write,
            move || {
                let mut catalog = lock_catalog(&catalog)?;
                super::snapshot::install_snapshot(&mut catalog, &meta, &bytes)
            },
        )
        .await
        .map_err(|error| *error)
    }

    async fn get_current_snapshot(
        &mut self,
    ) -> Result<Option<Snapshot<TypeConfig>>, StorageError<u64>> {
        let catalog = self.catalog.clone();
        run_blocking(ErrorSubject::StateMachine, ErrorVerb::Read, move || {
            let catalog = lock_catalog(&catalog)?;
            super::snapshot::current_snapshot(&catalog)
        })
        .await
        .map_err(|error| *error)
    }
}
