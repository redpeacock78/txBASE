use super::{Catalog, CatalogError, CatalogTransactionError};
use crate::ConstraintMode;
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
    deferred_constraints: BTreeMap<String, BTreeSet<String>>,
    aborted: bool,
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
        let deferred_constraints = super::constraints::initial_deferred_constraints(&before)?;

        Ok(CatalogTransaction {
            catalog,
            tables: before.clone(),
            before,
            touched: BTreeSet::new(),
            deferred_constraints,
            aborted: false,
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
        if self.aborted {
            return Err(CatalogError::Invalid(
                "catalog transaction is aborted after a failed operation".into(),
            ));
        }
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
        if self.aborted {
            return Err(CatalogTransactionError::Invalid(
                "catalog transaction is aborted after a failed operation".into(),
            ));
        }
        let result = super::transaction::apply_operation_to_tables(
            &self.catalog,
            &mut self.tables,
            &mut self.touched,
            operation,
            &self.deferred_constraints,
        );
        if result.is_err() {
            self.aborted = true;
        }
        result
    }

    /// Changes the timing of named deferrable constraints in one table.
    ///
    /// A transition to `Immediate` validates the current transaction image
    /// before changing the mode.
    pub fn set_constraints(
        &mut self,
        table_name: &str,
        names: &[&str],
        mode: ConstraintMode,
    ) -> Result<(), CatalogTransactionError> {
        if self.aborted {
            return Err(CatalogTransactionError::Invalid(
                "catalog transaction is aborted after a failed operation".into(),
            ));
        }
        if names.is_empty() {
            return self.abort_with(CatalogTransactionError::Invalid(
                "constraint name list must not be empty".into(),
            ));
        }
        let targets = names
            .iter()
            .map(|name| (table_name.to_owned(), (*name).to_owned()))
            .collect::<Vec<_>>();
        self.change_constraint_modes(&targets, mode)
    }

    /// Changes the timing of every deferrable constraint in the catalog.
    pub fn set_all_constraints(
        &mut self,
        mode: ConstraintMode,
    ) -> Result<(), CatalogTransactionError> {
        if self.aborted {
            return Err(CatalogTransactionError::Invalid(
                "catalog transaction is aborted after a failed operation".into(),
            ));
        }
        let mut targets = Vec::new();
        for (table_name, table) in &self.tables {
            let modes = match table.deferrable_constraint_modes() {
                Ok(modes) => modes,
                Err(source) => {
                    return self.abort_with(CatalogTransactionError::Catalog(
                        CatalogError::Table {
                            name: table_name.clone(),
                            source,
                        },
                    ));
                }
            };
            targets.extend(modes.into_keys().map(|name| (table_name.clone(), name)));
        }
        self.change_constraint_modes(&targets, mode)
    }

    fn change_constraint_modes(
        &mut self,
        targets: &[(String, String)],
        mode: ConstraintMode,
    ) -> Result<(), CatalogTransactionError> {
        let mut next = self.deferred_constraints.clone();
        for (table_name, name) in targets {
            let Some(table) = self.tables.get(table_name) else {
                return self.abort_with(CatalogTransactionError::Invalid(format!(
                    "table not found: {table_name}"
                )));
            };
            let modes = match table.deferrable_constraint_modes() {
                Ok(modes) => modes,
                Err(source) => {
                    return self.abort_with(CatalogTransactionError::Catalog(
                        CatalogError::Table {
                            name: table_name.clone(),
                            source,
                        },
                    ));
                }
            };
            if !modes.contains_key(name) {
                return self.abort_with(CatalogTransactionError::Invalid(format!(
                    "constraint {name} is not deferrable in table {table_name}"
                )));
            }
            let deferred = next.entry(table_name.clone()).or_default();
            match mode {
                ConstraintMode::Immediate => {
                    deferred.remove(name);
                }
                ConstraintMode::Deferred => {
                    deferred.insert(name.clone());
                }
            }
        }
        if mode == ConstraintMode::Immediate {
            if let Err(error) =
                super::constraints::validate_transaction_constraints(&self.tables, &next)
            {
                return self.abort_with(CatalogTransactionError::Catalog(error));
            }
        }
        self.deferred_constraints = next;
        Ok(())
    }

    fn abort_with<T>(
        &mut self,
        error: CatalogTransactionError,
    ) -> Result<T, CatalogTransactionError> {
        self.aborted = true;
        Err(error)
    }

    /// Commits all applied mutations as one catalog journal transaction.
    pub fn commit(self) -> Result<u64, CatalogTransactionError> {
        if self.aborted {
            return Err(CatalogTransactionError::Invalid(
                "catalog transaction is aborted after a failed operation".into(),
            ));
        }
        let Self {
            catalog,
            before,
            tables,
            touched,
            deferred_constraints,
            aborted: _,
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
        catalog.commit_loaded_tables_locked(
            before,
            tables,
            touched,
            true,
            Vec::new(),
            &deferred_constraints,
        )
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
