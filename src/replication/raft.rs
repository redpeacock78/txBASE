mod command;
mod log_store;
mod network;
mod snapshot;
mod state_machine;
#[cfg(test)]
mod state_machine_tests;

use std::io::Cursor;

pub use command::{
    MAX_RAFT_CLIENT_ID_BYTES, RAFT_COMMAND_VERSION, RaftCommand, RaftCommandPrecondition,
    RaftRejection, RaftResponse, RaftResponseResult,
};
pub use log_store::RaftLogStore;
pub use network::{MAX_RAFT_RPC_BYTES, RaftHttpNetworkFactory};
pub(crate) use network::{
    RAFT_ADD_LEARNER_PATH, RAFT_APPEND_PATH, RAFT_MEMBERSHIP_PATH, RAFT_RPC_VERSION,
    RAFT_SNAPSHOT_PATH, RAFT_VOTE_PATH, RpcRequestWire, reply as raft_rpc_reply,
};
pub use snapshot::CatalogSnapshotBuilder;
pub(crate) use state_machine::client_result as raft_client_result;
pub(crate) use state_machine::genesis_fingerprint as raft_genesis_fingerprint;
pub use state_machine::{RAFT_STATE_SIDECAR_NAME, RaftCatalogStateMachine};

openraft::declare_raft_types!(
    pub TypeConfig:
        D = RaftCommand,
        R = Option<RaftResponse>,
);
