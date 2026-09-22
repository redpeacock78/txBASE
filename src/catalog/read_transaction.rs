use super::{Catalog, CatalogError, CatalogTable};
use crate::dbf::{DbfTable, TableReadLock};
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
struct SnapshotTable {
    metadata: CatalogTable,
    table: DbfTable,
}

/// A stable, read-only in-memory image of all tables known to a catalog.
///
/// `begin_read` recovers and loads every current table while holding the
/// catalog read lock and all table read locks. The locks are released after
/// the copies are complete, so a caller can retain this value without
/// blocking later writers.
#[derive(Debug, Clone)]
pub struct CatalogReadTransaction {
    tables: BTreeMap<String, SnapshotTable>,
    transaction_id: Option<u64>,
}

impl Catalog {
    /// Captures one consistent read image of every discovered table.
    pub fn begin_read(&self) -> Result<CatalogReadTransaction, CatalogError> {
        let _catalog_lock = super::transaction::read_lock(&self.root)?;
        let transaction_id = match &self.historical_snapshot {
            Some(snapshot) => Some(snapshot.transaction_id),
            None => super::journal::read_transaction_id_locked(&self.root)?,
        };
        let entries = self.tables.values().cloned().collect::<Vec<_>>();

        if self.historical_snapshot.is_none() {
            for entry in &entries {
                DbfTable::from_path(entry.path()).map_err(|source| CatalogError::Table {
                    name: entry.name().to_owned(),
                    source,
                })?;
            }
        }

        let mut table_locks = Vec::new();
        if self.historical_snapshot.is_none() {
            for entry in &entries {
                table_locks.push(TableReadLock::acquire(entry.path())?);
            }
        }

        let mut tables = BTreeMap::new();
        for entry in entries {
            let mut table = if self.historical_snapshot.is_some() {
                self.open_table_unlocked(entry.name())?
            } else {
                crate::dbf::load_path_with_lock_held(entry.path()).map_err(|source| {
                    CatalogError::Table {
                        name: entry.name().to_owned(),
                        source,
                    }
                })?
            };
            table.mark_read_only();
            tables.insert(
                entry.name().to_owned(),
                SnapshotTable {
                    metadata: entry,
                    table,
                },
            );
        }
        drop(table_locks);

        Ok(CatalogReadTransaction {
            tables,
            transaction_id,
        })
    }
}

impl CatalogReadTransaction {
    /// Returns the catalog commit ID captured at begin time, if one exists.
    pub fn transaction_id(&self) -> Option<u64> {
        self.transaction_id
    }

    /// Returns table names in the same deterministic order as the catalog.
    pub fn table_names(&self) -> Vec<String> {
        self.tables.keys().cloned().collect()
    }

    /// Returns the captured path for a table without reading the filesystem.
    pub fn table_path(&self, name: &str) -> Option<&std::path::Path> {
        self.tables.get(name).map(|table| table.metadata.path())
    }

    /// Returns an independent read-only copy of a captured table.
    pub fn open_table(&self, name: &str) -> Result<DbfTable, CatalogError> {
        self.tables
            .get(name)
            .map(|table| table.table.clone())
            .ok_or_else(|| CatalogError::Invalid(format!("table not found: {name}")))
    }

    /// Returns schema metadata from the captured image without touching disk.
    pub fn schema_json(&self) -> Value {
        let tables = self
            .tables
            .values()
            .map(|entry| {
                json!({
                    "name": entry.metadata.name(),
                    "file": entry.metadata.file_name(),
                    "schema": entry.table.schema_json(),
                })
            })
            .collect::<Vec<_>>();
        json!({
            "format": "txbase-catalog",
            "transaction_id": self.transaction_id,
            "tables": tables,
        })
    }

    /// Executes the existing bounded join contract against this stable image.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn execute_join(
        &self,
        request: &crate::query::join::JoinRequest,
    ) -> Result<Vec<Value>, crate::query::join::JoinError> {
        crate::query::join::execute_read_transaction(self, request)
    }
}
