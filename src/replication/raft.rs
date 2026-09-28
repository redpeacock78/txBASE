mod command;
mod log_store;
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
pub use snapshot::CatalogSnapshotBuilder;
pub use state_machine::{RAFT_STATE_SIDECAR_NAME, RaftCatalogStateMachine};

openraft::declare_raft_types!(
    pub TypeConfig:
        D = RaftCommand,
        R = Option<RaftResponse>,
);
