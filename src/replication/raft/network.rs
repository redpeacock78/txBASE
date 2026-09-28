use super::TypeConfig;
use crate::replication::{ReplicationHttpClient, ReplicationRetryPolicy};
use openraft::BasicNode;
use openraft::error::{InstallSnapshotError, NetworkError, RPCError, RaftError, RemoteError};
use openraft::network::{RPCOption, RaftNetwork, RaftNetworkFactory};
use openraft::raft::{
    AppendEntriesRequest, AppendEntriesResponse, InstallSnapshotRequest, InstallSnapshotResponse,
    VoteRequest, VoteResponse,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::io;
use std::time::Duration;

pub(crate) const RAFT_RPC_VERSION: u16 = 1;
pub(crate) const RAFT_VOTE_PATH: &str = "/raft/v1/vote";
pub(crate) const RAFT_APPEND_PATH: &str = "/raft/v1/append";
pub(crate) const RAFT_SNAPSHOT_PATH: &str = "/raft/v1/snapshot";
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
    genesis_fingerprint: Vec<u8>,
    bearer_token: String,
}

impl RaftHttpNetworkFactory {
    pub fn new(
        cluster_id: impl Into<String>,
        sender_id: u64,
        genesis_fingerprint: Vec<u8>,
        bearer_token: impl Into<String>,
    ) -> Result<Self, String> {
        let cluster_id = cluster_id.into();
        if cluster_id.is_empty()
            || cluster_id.len() > 128
            || !cluster_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        {
            return Err(
                "Raft cluster ID must be 1 to 128 ASCII letters, digits, '.', '_', or '-'".into(),
            );
        }
        if sender_id == 0 {
            return Err("Raft node ID must be positive".into());
        }
        if genesis_fingerprint.len() != 32 {
            return Err("Raft genesis fingerprint must be a SHA-256 digest".into());
        }
        Ok(Self {
            cluster_id,
            sender_id,
            genesis_fingerprint,
            bearer_token: bearer_token.into(),
        })
    }
}

#[derive(Clone)]
pub struct RaftHttpNetwork {
    target_id: u64,
    cluster_id: String,
    sender_id: u64,
    genesis_fingerprint: Vec<u8>,
    client: Result<ReplicationHttpClient, String>,
}

impl RaftHttpNetwork {
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
        let request = RpcRequest {
            version: RAFT_RPC_VERSION,
            cluster_id: self.cluster_id.clone(),
            sender_id: self.sender_id,
            genesis_fingerprint: self.genesis_fingerprint.clone(),
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
            || reply.genesis_fingerprint != self.genesis_fingerprint
        {
            return Err("Raft RPC response identity or version does not match".into());
        }
        Ok(reply.result)
    }
}

impl RaftNetworkFactory<TypeConfig> for RaftHttpNetworkFactory {
    type Network = RaftHttpNetwork;

    async fn new_client(&mut self, target: u64, node: &BasicNode) -> Self::Network {
        let client = ReplicationHttpClient::new(&node.addr)
            .and_then(|client| client.with_bearer_token(self.bearer_token.clone()))
            .map_err(|error| error.to_string());
        RaftHttpNetwork {
            target_id: target,
            cluster_id: self.cluster_id.clone(),
            sender_id: self.sender_id,
            genesis_fingerprint: self.genesis_fingerprint.clone(),
            client,
        }
    }
}

impl RaftNetwork<TypeConfig> for RaftHttpNetwork {
    async fn append_entries(
        &mut self,
        rpc: AppendEntriesRequest<TypeConfig>,
        option: RPCOption,
    ) -> Result<AppendEntriesResponse<u64>, RPCError<u64, BasicNode, RaftError<u64>>> {
        let network = self.clone();
        let result = tokio::task::spawn_blocking(move || {
            network.rpc::<_, _, RaftError<u64>>(RAFT_APPEND_PATH, rpc, option.hard_ttl())
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
