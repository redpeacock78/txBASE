use crate::catalog::{CatalogError, CatalogTransactionError};
use std::error::Error;
use std::fmt::{self, Display, Formatter};

#[derive(Debug)]
pub enum ReplicationError {
    Invalid(String),
    Serialization(String),
    Catalog(CatalogError),
    Commit(CatalogTransactionError),
    TermMismatch {
        expected: u64,
        actual: u64,
    },
    IndexGap {
        expected: u64,
        actual: u64,
    },
    TransactionGap {
        expected: u64,
        actual: u64,
    },
    CatalogStateMismatch {
        expected: u64,
        actual: u64,
    },
    SchemaMismatch {
        expected: String,
        actual: String,
    },
    HistoryUnavailable {
        index: u64,
        base_index: u64,
    },
    ConflictingDuplicate {
        index: u64,
    },
    SidecarStateMismatch,
    ReadUnavailable {
        requested: u64,
        applied: u64,
    },
    ReadHistoryUnavailable {
        requested: u64,
        base_transaction_id: u64,
    },
    SnapshotUnavailable {
        requested: u64,
        applied: u64,
    },
    EntriesUnavailable {
        requested: u64,
        applied: u64,
    },
    ProgressUnavailable {
        requested: u64,
        applied: u64,
    },
    ProgressRegression {
        follower_id: String,
        previous_index: u64,
        requested_index: u64,
    },
    NoFollowerProgress,
    CompactionNotAcknowledged {
        requested: u64,
        acknowledged: u64,
    },
    SnapshotStale {
        requested: u64,
        current: u64,
    },
    SnapshotConflict {
        transaction_id: u64,
    },
    SnapshotInstallRace {
        expected: u64,
        actual: u64,
    },
}

impl Display for ReplicationError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(message) => write!(formatter, "invalid replication state: {message}"),
            Self::Serialization(message) => {
                write!(formatter, "replication serialization error: {message}")
            }
            Self::Catalog(error) => write!(formatter, "replication catalog read error: {error}"),
            Self::Commit(error) => write!(formatter, "replication catalog commit error: {error}"),
            Self::TermMismatch { expected, actual } => {
                write!(
                    formatter,
                    "replication term mismatch: expected {expected}, got {actual}"
                )
            }
            Self::IndexGap { expected, actual } => {
                write!(
                    formatter,
                    "replication index gap: expected {expected}, got {actual}"
                )
            }
            Self::TransactionGap { expected, actual } => write!(
                formatter,
                "replication transaction gap: expected {expected}, got {actual}"
            ),
            Self::CatalogStateMismatch { expected, actual } => write!(
                formatter,
                "replication catalog position mismatch: expected {expected}, got {actual}"
            ),
            Self::SchemaMismatch { expected, actual } => write!(
                formatter,
                "replication schema mismatch: expected {expected}, got {actual}"
            ),
            Self::HistoryUnavailable { index, base_index } => write!(
                formatter,
                "replication history for index {index} is unavailable before base index {base_index}"
            ),
            Self::ConflictingDuplicate { index } => {
                write!(
                    formatter,
                    "conflicting duplicate replication entry at index {index}"
                )
            }
            Self::SidecarStateMismatch => {
                write!(
                    formatter,
                    "replication sidecar does not match the in-memory log"
                )
            }
            Self::ReadUnavailable { requested, applied } => write!(
                formatter,
                "follower read transaction {requested} is beyond applied transaction {applied}"
            ),
            Self::ReadHistoryUnavailable {
                requested,
                base_transaction_id,
            } => write!(
                formatter,
                "follower read transaction {requested} is unavailable at log base transaction {base_transaction_id}"
            ),
            Self::SnapshotUnavailable { requested, applied } => write!(
                formatter,
                "replication snapshot index {requested} is beyond applied index {applied}"
            ),
            Self::EntriesUnavailable { requested, applied } => write!(
                formatter,
                "replication entry index {requested} is beyond applied index {applied}"
            ),
            Self::ProgressUnavailable { requested, applied } => write!(
                formatter,
                "replication progress index {requested} is beyond applied index {applied}"
            ),
            Self::ProgressRegression {
                follower_id,
                previous_index,
                requested_index,
            } => write!(
                formatter,
                "replication progress for follower {follower_id} regressed from index {previous_index} to {requested_index}"
            ),
            Self::NoFollowerProgress => write!(
                formatter,
                "replication compaction requires progress from at least one follower"
            ),
            Self::CompactionNotAcknowledged {
                requested,
                acknowledged,
            } => write!(
                formatter,
                "replication compaction index {requested} exceeds the lowest acknowledged follower index {acknowledged}"
            ),
            Self::SnapshotStale { requested, current } => write!(
                formatter,
                "replication snapshot transaction {requested} is not newer than catalog transaction {current}"
            ),
            Self::SnapshotConflict { transaction_id } => write!(
                formatter,
                "replication snapshot conflicts with catalog transaction {transaction_id}"
            ),
            Self::SnapshotInstallRace { expected, actual } => write!(
                formatter,
                "replication snapshot install raced with catalog transaction {expected} becoming {actual}"
            ),
        }
    }
}

impl Error for ReplicationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Catalog(error) => Some(error),
            Self::Commit(error) => Some(error),
            _ => None,
        }
    }
}
