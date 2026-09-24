//! Deterministic single-authority replication entries.
//!
//! This module deliberately stops before transport and consensus. A
//! [`ReplicationLog`] gives the local authority and its follower fixture the
//! same versioned entry, ordering, schema, and duplicate-delivery contract.

use crate::catalog::Catalog;
use crate::xbase::{MAX_OPERATION_BATCH, OperationIr, OperationMethod};
use serde::{Deserialize, Serialize};

mod error;
mod progress;
mod snapshot;
mod transport;
pub use error::ReplicationError;
pub use progress::{
    MAX_REPLICATION_FOLLOWERS, MAX_REPLICATION_PROGRESS_BYTES, REPLICATION_PROGRESS_VERSION,
    ReplicationProgress, ReplicationProgressOutcome,
};
pub use snapshot::{MAX_REPLICATION_SNAPSHOT_BYTES, ReplicationSnapshot};
pub use transport::{
    MAX_REPLICATION_ENTRY_BATCH, MAX_REPLICATION_ENTRY_BATCH_BYTES, ReplicationEntryBatch,
};

pub const REPLICATION_ENTRY_VERSION: u16 = 1;
pub const REPLICATION_LOG_VERSION: u16 = 1;
pub const REPLICATION_TRANSPORT_VERSION: u16 = 1;
pub const REPLICATION_SIDECAR_NAME: &str = ".txbase.replication";
pub const MAX_REPLICATION_ENTRY_BYTES: usize = crate::MAX_JSON_INPUT_BYTES;
const REPLICATION_SIDECAR_MAGIC: &[u8; 4] = b"TXRP";
const REPLICATION_SIDECAR_VERSION: u8 = 1;

enum CommitPreconditions<'a> {
    Catalog {
        if_match: Option<&'a str>,
        if_none_match: Option<&'a str>,
    },
    Table {
        name: &'a str,
        if_match: Option<&'a str>,
        if_none_match: Option<&'a str>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplicationEntry {
    pub version: u16,
    pub term: u64,
    pub index: u64,
    pub transaction_id: u64,
    pub schema_tag: String,
    pub operations: Vec<OperationIr>,
}

impl ReplicationEntry {
    pub fn new(
        term: u64,
        index: u64,
        transaction_id: u64,
        schema_tag: String,
        operations: Vec<OperationIr>,
    ) -> Result<Self, ReplicationError> {
        let entry = Self {
            version: REPLICATION_ENTRY_VERSION,
            term,
            index,
            transaction_id,
            schema_tag,
            operations,
        };
        entry.validate()?;
        Ok(entry)
    }

    pub fn validate(&self) -> Result<(), ReplicationError> {
        if self.version != REPLICATION_ENTRY_VERSION {
            return Err(ReplicationError::Invalid(format!(
                "unsupported replication entry version: {}",
                self.version
            )));
        }
        if self.term == 0 || self.index == 0 || self.transaction_id == 0 {
            return Err(ReplicationError::Invalid(
                "term, index, and transaction_id must be positive".into(),
            ));
        }
        if self.schema_tag.trim().is_empty() {
            return Err(ReplicationError::Invalid(
                "schema_tag must not be empty".into(),
            ));
        }
        if self.operations.is_empty() {
            return Err(ReplicationError::Invalid(
                "operations must not be empty".into(),
            ));
        }
        if self.operations.len() > MAX_OPERATION_BATCH {
            return Err(ReplicationError::Invalid(format!(
                "operation count exceeds {MAX_OPERATION_BATCH}"
            )));
        }
        for operation in &self.operations {
            if matches!(
                operation.method,
                OperationMethod::Get | OperationMethod::Query
            ) {
                return Err(ReplicationError::Invalid(format!(
                    "{} is a read operation",
                    operation.method.as_str()
                )));
            }
        }
        Ok(())
    }

    pub fn to_json(&self) -> Result<Vec<u8>, ReplicationError> {
        self.validate()?;
        let bytes = serde_json::to_vec(self)
            .map_err(|error| ReplicationError::Serialization(error.to_string()))?;
        if bytes.len() > MAX_REPLICATION_ENTRY_BYTES {
            return Err(ReplicationError::Invalid(format!(
                "replication entry JSON exceeds {MAX_REPLICATION_ENTRY_BYTES} bytes"
            )));
        }
        Ok(bytes)
    }

    pub fn from_json(bytes: &[u8]) -> Result<Self, ReplicationError> {
        if bytes.len() > MAX_REPLICATION_ENTRY_BYTES {
            return Err(ReplicationError::Invalid(format!(
                "replication entry JSON exceeds {MAX_REPLICATION_ENTRY_BYTES} bytes"
            )));
        }
        let entry: Self = serde_json::from_slice(bytes)
            .map_err(|error| ReplicationError::Serialization(error.to_string()))?;
        entry.validate()?;
        Ok(entry)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplicationLog {
    pub format_version: u16,
    term: u64,
    base_index: u64,
    base_transaction_id: u64,
    entries: Vec<ReplicationEntry>,
    #[serde(skip, default)]
    follower_watermarks: progress::FollowerWatermarks,
}

impl ReplicationLog {
    pub fn new(term: u64) -> Result<Self, ReplicationError> {
        Self::with_position(term, 0, 0)
    }

    pub fn with_position(
        term: u64,
        base_index: u64,
        base_transaction_id: u64,
    ) -> Result<Self, ReplicationError> {
        if term == 0 {
            return Err(ReplicationError::Invalid("term must be positive".into()));
        }
        Ok(Self {
            format_version: REPLICATION_LOG_VERSION,
            term,
            base_index,
            base_transaction_id,
            entries: Vec::new(),
            follower_watermarks: progress::FollowerWatermarks::default(),
        })
    }

    pub fn term(&self) -> u64 {
        self.term
    }

    pub fn base_index(&self) -> u64 {
        self.base_index
    }

    pub fn base_transaction_id(&self) -> u64 {
        self.base_transaction_id
    }

    pub fn last_index(&self) -> u64 {
        self.base_index + self.entries.len() as u64
    }

    pub fn last_transaction_id(&self) -> u64 {
        self.base_transaction_id + self.entries.len() as u64
    }

    pub fn entries(&self) -> &[ReplicationEntry] {
        &self.entries
    }

    /// Records a validated, monotonic applied position for one follower.
    pub fn acknowledge_follower(
        &mut self,
        catalog: &Catalog,
        progress: ReplicationProgress,
    ) -> Result<ReplicationProgressOutcome, ReplicationError> {
        let mut watermarks = std::mem::take(&mut self.follower_watermarks);
        let outcome = watermarks.acknowledge(self, catalog, progress);
        self.follower_watermarks = watermarks;
        outcome
    }

    /// Returns the lowest applied index reported by all registered followers.
    pub fn safe_compaction_index(&self) -> Option<u64> {
        self.follower_watermarks.safe_compaction_index()
    }

    pub fn follower_count(&self) -> usize {
        self.follower_watermarks.follower_count()
    }

    /// Builds the progress acknowledgement for this log's applied position.
    pub fn progress_for(
        &self,
        catalog: &Catalog,
        follower_id: String,
    ) -> Result<ReplicationProgress, ReplicationError> {
        self.ensure_catalog_position(current_transaction_id(catalog)?)?;
        let (_, schema_tag) = catalog
            .schema_representation()
            .map_err(ReplicationError::Catalog)?;
        ReplicationProgress::new(
            follower_id,
            self.term(),
            self.last_index(),
            self.last_transaction_id(),
            schema_tag,
        )
    }

    /// Reads a retained catalog snapshot at an already applied transaction.
    ///
    /// This is a local historical follower-read primitive. It does not claim
    /// quorum, leases, linearizability, or network freshness.
    pub fn read_at(
        &self,
        catalog: &Catalog,
        transaction_id: u64,
    ) -> Result<Catalog, ReplicationError> {
        if transaction_id == 0 {
            return Err(ReplicationError::Invalid(
                "follower read transaction_id must be positive".into(),
            ));
        }
        self.ensure_catalog_position(current_transaction_id(catalog)?)?;
        if transaction_id < self.base_transaction_id {
            return Err(ReplicationError::ReadHistoryUnavailable {
                requested: transaction_id,
                base_transaction_id: self.base_transaction_id,
            });
        }
        if transaction_id > self.last_transaction_id() {
            return Err(ReplicationError::ReadUnavailable {
                requested: transaction_id,
                applied: self.last_transaction_id(),
            });
        }
        Catalog::from_path_at(catalog.root(), transaction_id).map_err(ReplicationError::Catalog)
    }

    /// Reads the latest catalog image covered by this local replication log.
    pub fn read_applied(&self, catalog: &Catalog) -> Result<Catalog, ReplicationError> {
        let transaction_id = current_transaction_id(catalog)?;
        if transaction_id == 0 {
            return Err(ReplicationError::ReadUnavailable {
                requested: 1,
                applied: 0,
            });
        }
        self.read_at(catalog, transaction_id)
    }

    /// Opens the durable replication log associated with a catalog.
    ///
    /// A missing sidecar bootstraps a log at the requested term. An existing
    /// sidecar must use that term and must end at the catalog transaction
    /// position before it can accept another entry.
    pub fn open(catalog: &Catalog, term: u64) -> Result<Self, ReplicationError> {
        let bytes = catalog
            .read_sidecar_bytes(REPLICATION_SIDECAR_NAME)
            .map_err(ReplicationError::Catalog)?;
        let log = match bytes {
            Some(bytes) => Self::from_sidecar_bytes(&bytes)?,
            None => {
                let transaction_id = current_transaction_id(catalog)?;
                if transaction_id == 0 {
                    Self::new(term)?
                } else {
                    Self::with_position(term, transaction_id, transaction_id)?
                }
            }
        };
        if log.term != term {
            return Err(ReplicationError::TermMismatch {
                expected: term,
                actual: log.term,
            });
        }
        log.ensure_catalog_position(current_transaction_id(catalog)?)?;
        Ok(log)
    }

    /// Encodes the journaled TXRP sidecar format.
    pub fn to_sidecar_bytes(&self) -> Result<Vec<u8>, ReplicationError> {
        let mut bytes = REPLICATION_SIDECAR_MAGIC.to_vec();
        bytes.push(REPLICATION_SIDECAR_VERSION);
        bytes.extend_from_slice(&self.to_json()?);
        Ok(bytes)
    }

    /// Decodes and validates a journaled TXRP sidecar.
    pub fn from_sidecar_bytes(bytes: &[u8]) -> Result<Self, ReplicationError> {
        let Some(body) = bytes.strip_prefix(REPLICATION_SIDECAR_MAGIC) else {
            return Err(ReplicationError::Invalid(
                "replication sidecar header is invalid".into(),
            ));
        };
        let Some((&version, payload)) = body.split_first() else {
            return Err(ReplicationError::Invalid(
                "replication sidecar header is truncated".into(),
            ));
        };
        if version != REPLICATION_SIDECAR_VERSION {
            return Err(ReplicationError::Invalid(format!(
                "unsupported replication sidecar version: {version}"
            )));
        }
        Self::from_json(payload)
    }

    /// Proposes and commits one entry through the single local authority.
    pub fn propose(
        &mut self,
        catalog: &Catalog,
        operations: Vec<OperationIr>,
    ) -> Result<ReplicationEntry, ReplicationError> {
        self.propose_with_catalog_preconditions(catalog, operations, None, None)
    }

    /// Proposes one entry while checking the catalog representation preconditions.
    pub fn propose_with_catalog_preconditions(
        &mut self,
        catalog: &Catalog,
        operations: Vec<OperationIr>,
        if_match: Option<&str>,
        if_none_match: Option<&str>,
    ) -> Result<ReplicationEntry, ReplicationError> {
        let current_transaction_id = current_transaction_id(catalog)?;
        self.ensure_catalog_position(current_transaction_id)?;
        let (_, schema_tag) = catalog
            .schema_representation()
            .map_err(ReplicationError::Catalog)?;
        let entry = ReplicationEntry::new(
            self.term,
            next(self.last_index(), "index")?,
            next(self.last_transaction_id(), "transaction_id")?,
            schema_tag,
            operations,
        )?;
        self.apply_new_with_catalog_preconditions(catalog, entry.clone(), if_match, if_none_match)?;
        Ok(entry)
    }

    /// Proposes one entry while checking the representation of one named table.
    pub fn propose_with_table_preconditions(
        &mut self,
        catalog: &Catalog,
        table_name: &str,
        operations: Vec<OperationIr>,
        if_match: Option<&str>,
        if_none_match: Option<&str>,
    ) -> Result<ReplicationEntry, ReplicationError> {
        let current_transaction_id = current_transaction_id(catalog)?;
        self.ensure_catalog_position(current_transaction_id)?;
        let (_, schema_tag) = catalog
            .schema_representation()
            .map_err(ReplicationError::Catalog)?;
        let entry = ReplicationEntry::new(
            self.term,
            next(self.last_index(), "index")?,
            next(self.last_transaction_id(), "transaction_id")?,
            schema_tag,
            operations,
        )?;
        self.apply_new_with_table_preconditions(
            catalog,
            table_name,
            entry.clone(),
            if_match,
            if_none_match,
        )?;
        Ok(entry)
    }

    /// Delivers one entry to this log and catalog.
    ///
    /// The same entry delivered again is acknowledged without a second
    /// catalog commit. Missing entries and conflicting duplicates are errors.
    pub fn receive(
        &mut self,
        catalog: &Catalog,
        entry: ReplicationEntry,
    ) -> Result<ApplyOutcome, ReplicationError> {
        entry.validate()?;
        if entry.term != self.term {
            return Err(ReplicationError::TermMismatch {
                expected: self.term,
                actual: entry.term,
            });
        }

        if entry.index <= self.base_index {
            return Err(ReplicationError::HistoryUnavailable {
                index: entry.index,
                base_index: self.base_index,
            });
        }

        let offset = entry.index - self.base_index - 1;
        if let Ok(offset) = usize::try_from(offset) {
            if let Some(existing) = self.entries.get(offset) {
                if existing == &entry {
                    self.ensure_catalog_position(current_transaction_id(catalog)?)?;
                    return Ok(ApplyOutcome::Duplicate {
                        index: entry.index,
                        transaction_id: entry.transaction_id,
                    });
                }
                return Err(ReplicationError::ConflictingDuplicate { index: entry.index });
            }
        }

        let expected_index = next(self.last_index(), "index")?;
        if entry.index != expected_index {
            return Err(ReplicationError::IndexGap {
                expected: expected_index,
                actual: entry.index,
            });
        }
        let expected_transaction_id = next(self.last_transaction_id(), "transaction_id")?;
        if entry.transaction_id != expected_transaction_id {
            return Err(ReplicationError::TransactionGap {
                expected: expected_transaction_id,
                actual: entry.transaction_id,
            });
        }
        self.apply_new(catalog, entry)
    }

    /// Delivers one validated transport page in entry order.
    ///
    /// A page is not a transaction boundary: each entry remains independently
    /// atomic, and a failure may leave an already applied prefix in place.
    pub fn receive_batch(
        &mut self,
        catalog: &Catalog,
        batch: ReplicationEntryBatch,
    ) -> Result<Vec<ApplyOutcome>, ReplicationError> {
        batch.validate()?;
        if batch.term != self.term {
            return Err(ReplicationError::TermMismatch {
                expected: self.term,
                actual: batch.term,
            });
        }
        batch
            .entries
            .into_iter()
            .map(|entry| self.receive(catalog, entry))
            .collect()
    }

    pub fn to_json(&self) -> Result<Vec<u8>, ReplicationError> {
        self.validate()?;
        serde_json::to_vec(self).map_err(|error| ReplicationError::Serialization(error.to_string()))
    }

    pub fn from_json(bytes: &[u8]) -> Result<Self, ReplicationError> {
        let log: Self = serde_json::from_slice(bytes)
            .map_err(|error| ReplicationError::Serialization(error.to_string()))?;
        log.validate()?;
        Ok(log)
    }

    fn apply_new(
        &mut self,
        catalog: &Catalog,
        entry: ReplicationEntry,
    ) -> Result<ApplyOutcome, ReplicationError> {
        self.apply_new_with_preconditions(catalog, entry, None)
    }

    fn apply_new_with_catalog_preconditions(
        &mut self,
        catalog: &Catalog,
        entry: ReplicationEntry,
        if_match: Option<&str>,
        if_none_match: Option<&str>,
    ) -> Result<ApplyOutcome, ReplicationError> {
        self.apply_new_with_preconditions(
            catalog,
            entry,
            Some(CommitPreconditions::Catalog {
                if_match,
                if_none_match,
            }),
        )
    }

    fn apply_new_with_table_preconditions(
        &mut self,
        catalog: &Catalog,
        table_name: &str,
        entry: ReplicationEntry,
        if_match: Option<&str>,
        if_none_match: Option<&str>,
    ) -> Result<ApplyOutcome, ReplicationError> {
        self.apply_new_with_preconditions(
            catalog,
            entry,
            Some(CommitPreconditions::Table {
                name: table_name,
                if_match,
                if_none_match,
            }),
        )
    }

    fn apply_new_with_preconditions(
        &mut self,
        catalog: &Catalog,
        entry: ReplicationEntry,
        preconditions: Option<CommitPreconditions<'_>>,
    ) -> Result<ApplyOutcome, ReplicationError> {
        let expected_index = next(self.last_index(), "index")?;
        if entry.index != expected_index {
            return Err(ReplicationError::IndexGap {
                expected: expected_index,
                actual: entry.index,
            });
        }
        let expected_transaction_id = next(self.last_transaction_id(), "transaction_id")?;
        if entry.transaction_id != expected_transaction_id {
            return Err(ReplicationError::TransactionGap {
                expected: expected_transaction_id,
                actual: entry.transaction_id,
            });
        }
        self.ensure_catalog_position(current_transaction_id(catalog)?)?;
        let (_, schema_tag) = catalog
            .schema_representation()
            .map_err(ReplicationError::Catalog)?;
        if entry.schema_tag != schema_tag {
            return Err(ReplicationError::SchemaMismatch {
                expected: schema_tag,
                actual: entry.schema_tag,
            });
        }
        let actual_sidecar = catalog
            .read_sidecar_bytes(REPLICATION_SIDECAR_NAME)
            .map_err(ReplicationError::Catalog)?;
        let expected_sidecar = match actual_sidecar {
            Some(bytes) => {
                if bytes != self.to_sidecar_bytes()? {
                    return Err(ReplicationError::SidecarStateMismatch);
                }
                Some(bytes)
            }
            None => None,
        };
        let mut next_log = self.clone();
        next_log.entries.push(entry.clone());
        let transaction_id = match preconditions {
            Some(CommitPreconditions::Catalog {
                if_match,
                if_none_match,
            }) => catalog.commit_operations_with_preconditions_and_sidecar(
                &entry.operations,
                if_match,
                if_none_match,
                REPLICATION_SIDECAR_NAME,
                expected_sidecar,
                next_log.to_sidecar_bytes()?,
            ),
            Some(CommitPreconditions::Table {
                name,
                if_match,
                if_none_match,
            }) => catalog.commit_operations_with_sidecar_and_table_preconditions(
                &entry.operations,
                name,
                (if_match, if_none_match),
                REPLICATION_SIDECAR_NAME,
                expected_sidecar,
                next_log.to_sidecar_bytes()?,
            ),
            None => catalog.commit_operations_with_sidecar(
                &entry.operations,
                REPLICATION_SIDECAR_NAME,
                expected_sidecar,
                next_log.to_sidecar_bytes()?,
            ),
        }
        .map_err(ReplicationError::Commit)?;
        if transaction_id != entry.transaction_id {
            return Err(ReplicationError::Invalid(format!(
                "catalog committed transaction {transaction_id}, expected {}",
                entry.transaction_id
            )));
        }
        let outcome = ApplyOutcome::Applied {
            index: entry.index,
            transaction_id,
        };
        self.entries.push(entry);
        Ok(outcome)
    }

    fn ensure_catalog_position(&self, actual: u64) -> Result<(), ReplicationError> {
        let expected = self.last_transaction_id();
        if actual != expected {
            return Err(ReplicationError::CatalogStateMismatch { expected, actual });
        }
        Ok(())
    }

    fn validate(&self) -> Result<(), ReplicationError> {
        if self.format_version != REPLICATION_LOG_VERSION {
            return Err(ReplicationError::Invalid(format!(
                "unsupported replication log version: {}",
                self.format_version
            )));
        }
        if self.term == 0 {
            return Err(ReplicationError::Invalid("term must be positive".into()));
        }
        let mut expected_index = self.base_index;
        let mut expected_transaction_id = self.base_transaction_id;
        for entry in &self.entries {
            entry.validate()?;
            expected_index = next(expected_index, "index")?;
            expected_transaction_id = next(expected_transaction_id, "transaction_id")?;
            if entry.term != self.term {
                return Err(ReplicationError::TermMismatch {
                    expected: self.term,
                    actual: entry.term,
                });
            }
            if entry.index != expected_index {
                return Err(ReplicationError::IndexGap {
                    expected: expected_index,
                    actual: entry.index,
                });
            }
            if entry.transaction_id != expected_transaction_id {
                return Err(ReplicationError::TransactionGap {
                    expected: expected_transaction_id,
                    actual: entry.transaction_id,
                });
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyOutcome {
    Applied { index: u64, transaction_id: u64 },
    Duplicate { index: u64, transaction_id: u64 },
    SnapshotInstalled { index: u64, transaction_id: u64 },
    SnapshotDuplicate { index: u64, transaction_id: u64 },
}

fn current_transaction_id(catalog: &Catalog) -> Result<u64, ReplicationError> {
    catalog
        .transaction_id()
        .map(|transaction_id| transaction_id.unwrap_or(0))
        .map_err(ReplicationError::Catalog)
}

fn next(value: u64, name: &str) -> Result<u64, ReplicationError> {
    value
        .checked_add(1)
        .ok_or_else(|| ReplicationError::Invalid(format!("{name} is exhausted")))
}

#[cfg(test)]
#[path = "replication_tests.rs"]
mod tests;
