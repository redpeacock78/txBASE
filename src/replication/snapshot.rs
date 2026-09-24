use super::{ApplyOutcome, REPLICATION_SIDECAR_NAME, ReplicationError, ReplicationLog};
use crate::catalog::{Catalog, CatalogTransactionError};
use serde::{Deserialize, Serialize};

pub const REPLICATION_SNAPSHOT_VERSION: u16 = 1;
pub const MAX_REPLICATION_SNAPSHOT_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplicationSnapshot {
    pub version: u16,
    pub term: u64,
    pub last_index: u64,
    pub last_transaction_id: u64,
    pub schema_tag: String,
    pub catalog: Vec<u8>,
}

impl ReplicationSnapshot {
    pub fn new(
        term: u64,
        last_index: u64,
        last_transaction_id: u64,
        schema_tag: String,
        catalog: Vec<u8>,
    ) -> Result<Self, ReplicationError> {
        let snapshot = Self {
            version: REPLICATION_SNAPSHOT_VERSION,
            term,
            last_index,
            last_transaction_id,
            schema_tag,
            catalog,
        };
        snapshot.validate()?;
        Ok(snapshot)
    }

    pub fn validate(&self) -> Result<(), ReplicationError> {
        if self.version != REPLICATION_SNAPSHOT_VERSION {
            return Err(ReplicationError::Invalid(format!(
                "unsupported replication snapshot version: {}",
                self.version
            )));
        }
        if self.term == 0 || self.last_index == 0 || self.last_transaction_id == 0 {
            return Err(ReplicationError::Invalid(
                "snapshot term, last_index, and last_transaction_id must be positive".into(),
            ));
        }
        if self.schema_tag.trim().is_empty() {
            return Err(ReplicationError::Invalid(
                "snapshot schema_tag must not be empty".into(),
            ));
        }
        if self.catalog.is_empty() {
            return Err(ReplicationError::Invalid(
                "snapshot catalog payload must not be empty".into(),
            ));
        }
        if self.catalog.len() > MAX_REPLICATION_SNAPSHOT_BYTES {
            return Err(ReplicationError::Invalid(format!(
                "snapshot catalog payload exceeds {MAX_REPLICATION_SNAPSHOT_BYTES} bytes"
            )));
        }
        let (transaction_id, schema_tag) =
            Catalog::snapshot_metadata(&self.catalog).map_err(ReplicationError::Catalog)?;
        if transaction_id != self.last_transaction_id {
            return Err(ReplicationError::TransactionGap {
                expected: self.last_transaction_id,
                actual: transaction_id,
            });
        }
        if schema_tag != self.schema_tag {
            return Err(ReplicationError::SchemaMismatch {
                expected: schema_tag,
                actual: self.schema_tag.clone(),
            });
        }
        Ok(())
    }

    pub fn to_json(&self) -> Result<Vec<u8>, ReplicationError> {
        self.validate()?;
        serde_json::to_vec(self).map_err(|error| ReplicationError::Serialization(error.to_string()))
    }

    pub fn from_json(bytes: &[u8]) -> Result<Self, ReplicationError> {
        if bytes.len() > MAX_REPLICATION_SNAPSHOT_BYTES {
            return Err(ReplicationError::Invalid(format!(
                "snapshot JSON exceeds {MAX_REPLICATION_SNAPSHOT_BYTES} bytes"
            )));
        }
        let snapshot: Self = serde_json::from_slice(bytes)
            .map_err(|error| ReplicationError::Serialization(error.to_string()))?;
        snapshot.validate()?;
        Ok(snapshot)
    }
}

impl ReplicationLog {
    /// Captures the current applied catalog image as a transport-independent snapshot.
    pub fn snapshot(&self, catalog: &Catalog) -> Result<ReplicationSnapshot, ReplicationError> {
        let (transaction_id, schema_tag, catalog_bytes) = catalog
            .export_current_snapshot()
            .map_err(ReplicationError::Catalog)?;
        self.ensure_catalog_position(transaction_id)?;
        ReplicationSnapshot::new(
            self.term(),
            self.last_index(),
            transaction_id,
            schema_tag,
            catalog_bytes,
        )
    }

    /// Installs a retained catalog image and compacts the local log to its base position.
    pub fn install_snapshot(
        &mut self,
        catalog: &mut Catalog,
        snapshot: ReplicationSnapshot,
    ) -> Result<ApplyOutcome, ReplicationError> {
        snapshot.validate()?;
        if snapshot.term != self.term() {
            return Err(ReplicationError::TermMismatch {
                expected: self.term(),
                actual: snapshot.term,
            });
        }
        let current = super::current_transaction_id(catalog)?;
        if current > snapshot.last_transaction_id {
            return Err(ReplicationError::SnapshotStale {
                requested: snapshot.last_transaction_id,
                current,
            });
        }
        let next_log = ReplicationLog::with_position(
            snapshot.term,
            snapshot.last_index,
            snapshot.last_transaction_id,
        )?;
        let sidecar_after = next_log.to_sidecar_bytes()?;
        let actual_sidecar = catalog
            .read_sidecar_bytes(REPLICATION_SIDECAR_NAME)
            .map_err(ReplicationError::Catalog)?;

        if current == snapshot.last_transaction_id {
            let current_snapshot = catalog
                .export_snapshot_at(current)
                .map_err(ReplicationError::Catalog)?;
            if current_snapshot != snapshot.catalog {
                return Err(ReplicationError::SnapshotConflict {
                    transaction_id: current,
                });
            }
            if actual_sidecar.as_deref() != Some(sidecar_after.as_slice()) {
                return Err(ReplicationError::SidecarStateMismatch);
            }
            *self = next_log;
            return Ok(ApplyOutcome::SnapshotDuplicate {
                index: snapshot.last_index,
                transaction_id: snapshot.last_transaction_id,
            });
        }

        let transaction_id = match catalog.install_snapshot_with_sidecar(
            &snapshot.catalog,
            current,
            REPLICATION_SIDECAR_NAME,
            actual_sidecar,
            sidecar_after,
        ) {
            Ok(transaction_id) => transaction_id,
            Err(CatalogTransactionError::TransactionPreconditionFailed { expected, actual }) => {
                return Err(ReplicationError::SnapshotInstallRace { expected, actual });
            }
            Err(error) => return Err(ReplicationError::Commit(error)),
        };
        if transaction_id != snapshot.last_transaction_id {
            return Err(ReplicationError::Invalid(format!(
                "catalog installed transaction {transaction_id}, expected {}",
                snapshot.last_transaction_id
            )));
        }
        *self = next_log;
        Ok(ApplyOutcome::SnapshotInstalled {
            index: snapshot.last_index,
            transaction_id,
        })
    }
}
