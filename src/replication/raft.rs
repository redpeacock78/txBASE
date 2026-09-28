mod command;

use std::io::Cursor;

pub use command::{
    MAX_RAFT_CLIENT_ID_BYTES, RAFT_COMMAND_VERSION, RaftCommand, RaftCommandPrecondition,
    RaftRejection, RaftResponse, RaftResponseResult,
};

openraft::declare_raft_types!(
    pub TypeConfig:
        D = RaftCommand,
        R = RaftResponse,
);
