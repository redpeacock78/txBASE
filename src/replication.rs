//! Deterministic single-authority replication entries.
//!
//! This module deliberately stops before transport and consensus. A
//! [`ReplicationLog`] gives the local authority and its follower fixture the
//! same versioned entry, ordering, schema, and duplicate-delivery contract.

use crate::catalog::{Catalog, CatalogError, CatalogTransactionError};
use crate::xbase::{MAX_OPERATION_BATCH, OperationIr, OperationMethod};
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::fmt::{self, Display, Formatter};

pub const REPLICATION_ENTRY_VERSION: u16 = 1;
pub const REPLICATION_LOG_VERSION: u16 = 1;
pub const REPLICATION_SIDECAR_NAME: &str = ".txbase.replication";
const REPLICATION_SIDECAR_MAGIC: &[u8; 4] = b"TXRP";
const REPLICATION_SIDECAR_VERSION: u8 = 1;

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
        serde_json::to_vec(self).map_err(|error| ReplicationError::Serialization(error.to_string()))
    }

    pub fn from_json(bytes: &[u8]) -> Result<Self, ReplicationError> {
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
        })
    }

    pub fn term(&self) -> u64 {
        self.term
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
        self.ensure_catalog_position(current_transaction_id(catalog)?)?;
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
            None => Self::new(term)?,
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
        self.apply_new(catalog, entry.clone())?;
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
                    let actual = current_transaction_id(catalog)?;
                    if actual != entry.transaction_id {
                        return Err(ReplicationError::CatalogStateMismatch {
                            expected: entry.transaction_id,
                            actual,
                        });
                    }
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
        let transaction_id = catalog
            .commit_operations_with_sidecar(
                &entry.operations,
                REPLICATION_SIDECAR_NAME,
                expected_sidecar,
                next_log.to_sidecar_bytes()?,
            )
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
}

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
