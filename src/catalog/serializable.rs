use super::{Catalog, CatalogError, CatalogTransactionError};
use crate::dbf::{DbfTable, TableLock};
use crate::xbase::OperationIr;
use std::collections::{BTreeMap, BTreeSet};

/// A coarse-grained serializable transaction over the catalog table set.
///
/// The catalog write lock and every discovered table lock remain held until
/// `commit`, `rollback`, or drop. Mutations are applied to private copies and
/// committed as one catalog journal transaction. A table-set change observed at
/// commit is rejected instead of being mixed into the transaction.
pub struct CatalogTransaction {
    catalog: Catalog,
    before: BTreeMap<String, DbfTable>,
    tables: BTreeMap<String, DbfTable>,
    touched: BTreeSet<String>,
    expected_table_set: BTreeSet<(String, String)>,
    catalog_lock: super::transaction::CatalogWriteLock,
    table_locks: Vec<TableLock>,
}

impl Catalog {
    /// Opens a serializable transaction and locks the current catalog image.
    pub fn begin_serializable(&self) -> Result<CatalogTransaction, CatalogError> {
        if self.is_historical() {
            return Err(CatalogError::Invalid(
                "historical catalog snapshots are read-only".into(),
            ));
        }

        let catalog_lock = super::transaction::write_lock(&self.root)?;
        let mut catalog = self.clone();
        catalog.tables = super::discovery::discover_tables(&self.root)?;
        let expected_table_set = table_set(&catalog.tables);
        let entries = catalog.tables.values().cloned().collect::<Vec<_>>();
        let mut table_locks = Vec::with_capacity(entries.len());
        for entry in &entries {
            table_locks.push(TableLock::acquire(entry.path())?);
        }

        let mut before = BTreeMap::new();
        for entry in &entries {
            crate::dbf::recover_path_with_lock_held(entry.path()).map_err(|source| {
                CatalogError::Table {
                    name: entry.name().to_owned(),
                    source,
                }
            })?;
            let table = crate::dbf::load_path_with_lock_held(entry.path()).map_err(|source| {
                CatalogError::Table {
                    name: entry.name().to_owned(),
                    source,
                }
            })?;
            before.insert(entry.name().to_owned(), table);
        }

        Ok(CatalogTransaction {
            catalog,
            tables: before.clone(),
            before,
            touched: BTreeSet::new(),
            expected_table_set,
            catalog_lock,
            table_locks,
        })
    }
}

impl CatalogTransaction {
    /// Returns table names in deterministic catalog order.
    pub fn table_names(&self) -> Vec<String> {
        self.tables.keys().cloned().collect()
    }

    /// Returns a read-only copy of a table in the private transaction image.
    pub fn open_table(&self, name: &str) -> Result<DbfTable, CatalogError> {
        let mut table = self
            .tables
            .get(name)
            .cloned()
            .ok_or_else(|| CatalogError::Invalid(format!("table not found: {name}")))?;
        table.mark_read_only();
        Ok(table)
    }

    /// Applies one named record mutation to the private transaction image.
    pub fn apply(&mut self, operation: &OperationIr) -> Result<(), CatalogTransactionError> {
        super::transaction::apply_operation_to_tables(
            &self.catalog,
            &mut self.tables,
            &mut self.touched,
            operation,
        )
    }

    /// Commits all applied mutations as one catalog journal transaction.
    pub fn commit(self) -> Result<u64, CatalogTransactionError> {
        let Self {
            catalog,
            before,
            tables,
            touched,
            expected_table_set,
            catalog_lock,
            table_locks,
        } = self;
        let _catalog_lock = catalog_lock;
        let _table_locks = table_locks;
        let current_table_set = super::discovery::discover_tables(&catalog.root)
            .map(|tables| table_set(&tables))
            .map_err(CatalogTransactionError::Catalog)?;
        if current_table_set != expected_table_set {
            return Err(CatalogTransactionError::TableSetChanged);
        }
        catalog.commit_loaded_tables_locked(before, tables, touched, true, Vec::new())
    }

    /// Discards the private image and releases all held locks.
    pub fn rollback(self) {}
}

fn table_set(tables: &BTreeMap<String, super::CatalogTable>) -> BTreeSet<(String, String)> {
    tables
        .values()
        .map(|table| (table.name().to_owned(), table.file_name().to_owned()))
        .collect()
}
