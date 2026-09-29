use super::TypeConfig;
#[cfg(test)]
use super::network_faults::{AppendDelayHandle, FaultController};
use crate::replication::{
    MAX_REPLICATION_SNAPSHOT_BYTES, ReplicationHttpClient, ReplicationHttpError,
    ReplicationRetryPolicy,
};
use openraft::BasicNode;
#[cfg(test)]
use openraft::EntryPayload;
use openraft::error::{InstallSnapshotError, NetworkError, RPCError, RaftError, RemoteError};
use openraft::network::{RPCOption, RaftNetwork, RaftNetworkFactory};
use openraft::raft::{
    AppendEntriesRequest, AppendEntriesResponse, InstallSnapshotRequest, InstallSnapshotResponse,
    VoteRequest, VoteResponse,
};
use openraft::{Snapshot, Vote};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::io;
use std::sync::{Arc, RwLock};
use std::time::Duration;

pub(crate) const RAFT_RPC_VERSION: u16 = 1;
pub(crate) const RAFT_VOTE_PATH: &str = "/raft/v1/vote";
pub(crate) const RAFT_APPEND_PATH: &str = "/raft/v1/append";
pub(crate) const RAFT_SNAPSHOT_PATH: &str = "/raft/v1/snapshot";
pub(crate) const RAFT_ADD_LEARNER_PATH: &str = "/raft/v1/learner";
pub(crate) const RAFT_PREPARE_LEARNER_PATH: &str = "/raft/v1/learner/prepare";
pub(crate) const RAFT_MEMBERSHIP_PATH: &str = "/raft/v1/membership";
pub const MAX_RAFT_RPC_BYTES: usize = crate::MAX_JSON_INPUT_BYTES * 2;

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct RpcRequest<T> {
    version: u16,
    cluster_id: String,
    sender_id: u64,
    genesis_fingerprint: Vec<u8>,
    payload: T,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RpcRequestWire<T> {
    pub(crate) version: u16,
    pub(crate) cluster_id: String,
    pub(crate) sender_id: u64,
    pub(crate) genesis_fingerprint: Vec<u8>,
    pub(crate) payload: T,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RpcReply<T, E> {
    version: u16,
    cluster_id: String,
    node_id: u64,
    genesis_fingerprint: Vec<u8>,
    result: Result<T, E>,
}

pub(crate) fn reply<T, E>(
    cluster_id: &str,
    node_id: u64,
    genesis_fingerprint: &[u8],
    result: Result<T, E>,
) -> RpcReply<T, E> {
    RpcReply {
        version: RAFT_RPC_VERSION,
        cluster_id: cluster_id.to_owned(),
        node_id,
        genesis_fingerprint: genesis_fingerprint.to_vec(),
        result,
    }
}

#[derive(Clone)]
pub struct RaftHttpNetworkFactory {
    cluster_id: String,
    sender_id: u64,
    genesis_fingerprint: Arc<RwLock<Vec<u8>>>,
    bearer_token: String,
    #[cfg(test)]
    faults: Arc<FaultController>,
}

impl RaftHttpNetworkFactory {
    fn new_network(&self, target: u64, address: &str) -> RaftHttpNetwork {
        let client = ReplicationHttpClient::new(address)
            .and_then(|client| client.with_bearer_token(self.bearer_token.clone()))
            .map_err(|error| error.to_string());
        RaftHttpNetwork {
            target_id: target,
            cluster_id: self.cluster_id.clone(),
            sender_id: self.sender_id,
            genesis_fingerprint: Arc::clone(&self.genesis_fingerprint),
            client,
            #[cfg(test)]
            faults: Arc::clone(&self.faults),
        }
    }

    #[cfg(test)]
    pub(crate) fn set_peer_blocked(&self, target: u64, blocked: bool) -> Result<(), String> {
        self.faults.set_peer_blocked(target, blocked)
    }

    #[cfg(test)]
    pub(crate) fn delay_next_append_entries(
        &self,
        target: u64,
    ) -> Result<AppendDelayHandle, String> {
        self.faults.delay_next_append_entries(target)
    }

    #[cfg(test)]
    pub(crate) fn delay_next_uniform_membership_append(
        &self,
        target: u64,
    ) -> Result<AppendDelayHandle, String> {
        self.faults.delay_next_uniform_membership_append(target)
    }

    pub(crate) fn prepare_learner(
        &self,
        target_id: u64,
        address: &str,
        timeout: Duration,
    ) -> Result<(), (u16, String)> {
        let fingerprint = self
            .genesis_fingerprint
            .read()
            .map_err(|error| (503, format!("Raft fingerprint lock poisoned: {error}")))?
            .clone();
        let request = RpcRequest {
            version: RAFT_RPC_VERSION,
            cluster_id: self.cluster_id.clone(),
            sender_id: self.sender_id,
            genesis_fingerprint: fingerprint.clone(),
            payload: (),
        };
        let bytes = serde_json::to_vec(&request).map_err(|error| (500, error.to_string()))?;
        let retry = ReplicationRetryPolicy::new(1, Duration::ZERO, Duration::ZERO)
            .map_err(http_error_status)?;
        let client = ReplicationHttpClient::new(address)
            .and_then(|client| client.with_bearer_token(self.bearer_token.clone()))
            .and_then(|client| client.with_timeout(timeout))
            .map_err(http_error_status)?
            .with_retry_policy(retry);
        let response = client
            .post_json_bytes(RAFT_PREPARE_LEARNER_PATH, &bytes, MAX_RAFT_RPC_BYTES)
            .map_err(http_error_status)?;
        let reply: RpcReply<(), String> = serde_json::from_slice(&response).map_err(|error| {
            (
                502,
                format!("invalid learner preparation response: {error}"),
            )
        })?;
        if reply.version != RAFT_RPC_VERSION
            || reply.cluster_id != self.cluster_id
            || reply.node_id != target_id
            || reply.genesis_fingerprint != fingerprint
        {
            return Err((
                502,
                "learner preparation response identity does not match".into(),
            ));
        }
        reply.result.map_err(|error| (409, error))
    }

    pub(crate) fn transfer_snapshot(
        &self,
        target_id: u64,
        address: &str,
        vote: Vote<u64>,
        snapshot: Snapshot<TypeConfig>,
        timeout: Duration,
    ) -> Result<(), String> {
        let network = self.new_network(target_id, address);
        let meta = snapshot.meta;
        let bytes = (*snapshot.snapshot).into_inner();
        if bytes.is_empty() || bytes.len() > MAX_REPLICATION_SNAPSHOT_BYTES {
            return Err(format!(
                "Raft snapshot size must be between 1 and {MAX_REPLICATION_SNAPSHOT_BYTES} bytes"
            ));
        }

        let chunk_size = (MAX_RAFT_RPC_BYTES / 8).max(1);
        for (index, chunk) in bytes.chunks(chunk_size).enumerate() {
            let offset = index * chunk_size;
            let request: InstallSnapshotRequest<TypeConfig> = InstallSnapshotRequest {
                vote,
                meta: meta.clone(),
                offset: offset as u64,
                data: chunk.to_vec(),
                done: offset + chunk.len() == bytes.len(),
            };
            let result = network
                .rpc::<_, InstallSnapshotResponse<u64>, RaftError<u64, InstallSnapshotError>>(
                    RAFT_SNAPSHOT_PATH,
                    request,
                    timeout,
                )
                .map_err(|error| format!("learner snapshot RPC failed: {error}"))?;
            let response = result.map_err(|error| format!("learner rejected snapshot: {error}"))?;
            if response.vote != vote {
                return Err(format!(
                    "learner snapshot response vote {} does not match leader vote {vote}",
                    response.vote
                ));
            }
        }
        Ok(())
    }

    pub fn new(
        cluster_id: impl Into<String>,
        sender_id: u64,
        genesis_fingerprint: Vec<u8>,
        bearer_token: impl Into<String>,
    ) -> Result<Self, String> {
        Self::new_with_fingerprint(
            cluster_id,
            sender_id,
            Arc::new(RwLock::new(genesis_fingerprint)),
            bearer_token,
        )
    }

    pub(crate) fn new_with_fingerprint(
        cluster_id: impl Into<String>,
        sender_id: u64,
        genesis_fingerprint: Arc<RwLock<Vec<u8>>>,
        bearer_token: impl Into<String>,
    ) -> Result<Self, String> {
        let cluster_id = cluster_id.into();
        if !is_valid_cluster_id(&cluster_id) {
            return Err(
                "Raft cluster ID must be 1 to 128 ASCII letters, digits, '.', '_', or '-'".into(),
            );
        }
        if sender_id == 0 {
            return Err("Raft node ID must be positive".into());
        }
        if genesis_fingerprint
            .read()
            .map_err(|error| format!("Raft fingerprint lock poisoned: {error}"))?
            .len()
            != 32
        {
            return Err("Raft genesis fingerprint must be a SHA-256 digest".into());
        }
        Ok(Self {
            cluster_id,
            sender_id,
            genesis_fingerprint,
            bearer_token: bearer_token.into(),
            #[cfg(test)]
            faults: Arc::new(FaultController::default()),
        })
    }
}

fn http_error_status(error: ReplicationHttpError) -> (u16, String) {
    let status = match &error {
        ReplicationHttpError::HttpStatus { status, .. } => *status,
        _ => 503,
    };
    (status, error.to_string())
}

pub(crate) fn is_valid_cluster_id(cluster_id: &str) -> bool {
    !cluster_id.is_empty()
        && cluster_id.len() <= 128
        && cluster_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

#[derive(Clone)]
pub struct RaftHttpNetwork {
    target_id: u64,
    cluster_id: String,
    sender_id: u64,
    genesis_fingerprint: Arc<RwLock<Vec<u8>>>,
    client: Result<ReplicationHttpClient, String>,
    #[cfg(test)]
    faults: Arc<FaultController>,
}

impl RaftHttpNetwork {
    #[cfg(test)]
    fn check_network(&self, operation: &str) -> Result<(), String> {
        self.faults.check_network(self.target_id, operation)
    }

    fn rpc<Q, T, E>(
        &self,
        path: &str,
        payload: Q,
        timeout: Duration,
    ) -> Result<Result<T, E>, String>
    where
        Q: Serialize,
        T: DeserializeOwned,
        E: DeserializeOwned,
    {
        let genesis_fingerprint = self
            .genesis_fingerprint
            .read()
            .map_err(|error| format!("Raft fingerprint lock poisoned: {error}"))?
            .clone();
        let request = RpcRequest {
            version: RAFT_RPC_VERSION,
            cluster_id: self.cluster_id.clone(),
            sender_id: self.sender_id,
            genesis_fingerprint: genesis_fingerprint.clone(),
            payload,
        };
        let bytes = serde_json::to_vec(&request).map_err(|error| error.to_string())?;
        if bytes.len() > MAX_RAFT_RPC_BYTES {
            return Err(format!("Raft RPC exceeds {MAX_RAFT_RPC_BYTES} bytes"));
        }
        let retry = ReplicationRetryPolicy::new(1, Duration::ZERO, Duration::ZERO)
            .map_err(|error| error.to_string())?;
        let client = self
            .client
            .as_ref()
            .map_err(Clone::clone)?
            .clone()
            .with_timeout(timeout)
            .map_err(|error| error.to_string())?
            .with_retry_policy(retry);
        let bytes = client
            .post_json_bytes(path, &bytes, MAX_RAFT_RPC_BYTES)
            .map_err(|error| error.to_string())?;
        let reply: RpcReply<T, E> = serde_json::from_slice(&bytes)
            .map_err(|error| format!("invalid Raft RPC response: {error}"))?;
        if reply.version != RAFT_RPC_VERSION
            || reply.cluster_id != self.cluster_id
            || reply.node_id != self.target_id
            || reply.genesis_fingerprint != genesis_fingerprint
        {
            return Err("Raft RPC response identity or version does not match".into());
        }
        Ok(reply.result)
    }
}

impl RaftNetworkFactory<TypeConfig> for RaftHttpNetworkFactory {
    type Network = RaftHttpNetwork;

    async fn new_client(&mut self, target: u64, node: &BasicNode) -> Self::Network {
        self.new_network(target, &node.addr)
    }
}

impl RaftNetwork<TypeConfig> for RaftHttpNetwork {
    async fn append_entries(
        &mut self,
        rpc: AppendEntriesRequest<TypeConfig>,
        option: RPCOption,
    ) -> Result<AppendEntriesResponse<u64>, RPCError<u64, BasicNode, RaftError<u64>>> {
        #[cfg(test)]
        let has_uniform_membership = rpc.entries.iter().any(|entry| {
            matches!(
                &entry.payload,
                EntryPayload::Membership(membership)
                    if membership.get_joint_config().len() == 1
            )
        });
        #[cfg(test)]
        let delay = if rpc.entries.is_empty() {
            None
        } else {
            match self
                .faults
                .take_append_delay(self.target_id, has_uniform_membership)
            {
                Ok(delay) => delay,
                Err(error) => return map_rpc_result(self.target_id, Err(error)),
            }
        };
        #[cfg(test)]
        if delay.is_none() {
            if let Err(error) = self.check_network("append-entries") {
                return map_rpc_result(self.target_id, Err(error));
            }
        }
        let network = self.clone();
        let result = tokio::task::spawn_blocking(move || {
            #[cfg(test)]
            let can_send = match delay.as_ref() {
                Some(delay) => delay.pause(),
                None => true,
            };
            #[cfg(test)]
            if !can_send {
                // A cancelled test RPC must not reach its peer.
                let error = "test network cancelled AppendEntries RPC".to_owned();
                if let Some(delay) = delay {
                    delay.complete(Err(error.clone()));
                }
                return Err(error);
            }
            let result =
                network.rpc::<_, _, RaftError<u64>>(RAFT_APPEND_PATH, rpc, option.hard_ttl());
            #[cfg(test)]
            if let Some(delay) = delay {
                delay.complete(result.as_ref().map(|_| ()).map_err(Clone::clone));
            }
            result
        })
        .await
        .map_err(|error| error.to_string())
        .and_then(|result| result);
        map_rpc_result(self.target_id, result)
    }

    async fn install_snapshot(
        &mut self,
        rpc: InstallSnapshotRequest<TypeConfig>,
        option: RPCOption,
    ) -> Result<
        InstallSnapshotResponse<u64>,
        RPCError<u64, BasicNode, RaftError<u64, InstallSnapshotError>>,
    > {
        #[cfg(test)]
        if let Err(error) = self.check_network("snapshot") {
            return map_rpc_result(self.target_id, Err(error));
        }
        let network = self.clone();
        let result = tokio::task::spawn_blocking(move || {
            network.rpc::<_, _, RaftError<u64, InstallSnapshotError>>(
                RAFT_SNAPSHOT_PATH,
                rpc,
                option.hard_ttl(),
            )
        })
        .await
        .map_err(|error| error.to_string())
        .and_then(|result| result);
        map_rpc_result(self.target_id, result)
    }

    async fn vote(
        &mut self,
        rpc: VoteRequest<u64>,
        option: RPCOption,
    ) -> Result<VoteResponse<u64>, RPCError<u64, BasicNode, RaftError<u64>>> {
        #[cfg(test)]
        if let Err(error) = self.check_network("vote") {
            return map_rpc_result(self.target_id, Err(error));
        }
        let network = self.clone();
        let result = tokio::task::spawn_blocking(move || {
            network.rpc::<_, _, RaftError<u64>>(RAFT_VOTE_PATH, rpc, option.hard_ttl())
        })
        .await
        .map_err(|error| error.to_string())
        .and_then(|result| result);
        map_rpc_result(self.target_id, result)
    }
}

// OpenRaft's Network trait requires RPCError to be returned by value.
#[allow(clippy::result_large_err)]
fn map_rpc_result<T, E>(
    target_id: u64,
    result: Result<Result<T, E>, String>,
) -> Result<T, RPCError<u64, BasicNode, E>>
where
    E: std::error::Error,
{
    match result {
        Ok(Ok(response)) => Ok(response),
        Ok(Err(error)) => Err(RPCError::RemoteError(RemoteError::new(target_id, error))),
        Err(message) => Err(RPCError::Network(NetworkError::new(&io::Error::other(
            message,
        )))),
    }
}

#[cfg(test)]
#[path = "network_tests.rs"]
mod tests;
