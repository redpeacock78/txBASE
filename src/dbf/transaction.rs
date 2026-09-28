use super::{DbfError, DbfTable, TableLock};
use crate::ConstraintMode;
use crate::query::{self, QueryError, QueryRequest};
use crate::xbase::OperationIr;
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// An optimistic snapshot transaction for one path-backed DBF table.
///
/// The table is loaded when the transaction starts. Mutations are applied to
/// the private copy and become visible only when `commit` completes its one
/// WAL-backed save. A concurrent change to the loaded DBF, memo, schema, or
/// transaction-state bytes makes the commit fail instead of overwriting it.
pub struct DbfTransaction {
    path: PathBuf,
    table: DbfTable,
    deferred_constraints: BTreeSet<String>,
    serializable_lock: Option<TableLock>,
}

impl DbfTransaction {
    /// Opens a current table snapshot and starts a transaction.
    pub fn begin(path: impl AsRef<Path>) -> Result<Self, DbfError> {
        let path = path.as_ref().to_path_buf();
        let table = DbfTable::from_path(&path)?;
        let deferred_constraints = initially_deferred(&table);
        Ok(Self {
            path,
            table,
            deferred_constraints,
            serializable_lock: None,
        })
    }

    /// Opens a transaction that holds the table's exclusive lock until commit
    /// or rollback, providing a coarse-grained serializable boundary.
    pub fn begin_serializable(path: impl AsRef<Path>) -> Result<Self, DbfError> {
        let path = path.as_ref().to_path_buf();
        let lock = TableLock::acquire(&path)?;
        super::recover_path_with_lock_held(&path)?;
        let table = super::load_path_with_lock_held(&path)?;
        let deferred_constraints = initially_deferred(&table);
        Ok(Self {
            path,
            table,
            deferred_constraints,
            serializable_lock: Some(lock),
        })
    }

    /// Creates a transaction from an already loaded current table.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn from_table(path: impl AsRef<Path>, table: DbfTable) -> Self {
        let deferred_constraints = initially_deferred(&table);
        Self {
            path: path.as_ref().to_path_buf(),
            table,
            deferred_constraints,
            serializable_lock: None,
        }
    }

    /// Applies one mutation to the private transaction snapshot.
    pub fn apply(&mut self, operation: &OperationIr) -> Result<(), DbfError> {
        self.table
            .apply_operation_with_deferred_constraints(operation, &self.deferred_constraints)
    }

    /// Changes the timing of named deferrable constraints in this table.
    ///
    /// A transition to Immediate validates the current transaction image
    /// before changing the mode.
    pub fn set_constraints(
        &mut self,
        names: &[&str],
        mode: ConstraintMode,
    ) -> Result<(), DbfError> {
        if names.is_empty() {
            return Err(DbfError::Invalid(
                "constraint name list must not be empty".into(),
            ));
        }
        let names = names
            .iter()
            .map(|name| (*name).to_owned())
            .collect::<Vec<_>>();
        self.change_constraint_modes(&names, mode)
    }

    /// Changes the timing of every deferrable local constraint in this table.
    pub fn set_all_constraints(&mut self, mode: ConstraintMode) -> Result<(), DbfError> {
        let names = self
            .table
            .local_deferrable_constraint_modes()
            .into_keys()
            .collect::<Vec<_>>();
        self.change_constraint_modes(&names, mode)
    }

    fn change_constraint_modes(
        &mut self,
        names: &[String],
        mode: ConstraintMode,
    ) -> Result<(), DbfError> {
        let modes = self.table.local_deferrable_constraint_modes();
        if names.iter().any(|name| !modes.contains_key(name)) {
            return Err(DbfError::Invalid(
                "constraint name does not identify a deferrable constraint in this table".into(),
            ));
        }
        let mut deferred_constraints = self.deferred_constraints.clone();
        for name in names {
            match mode {
                ConstraintMode::Immediate => {
                    deferred_constraints.remove(name);
                }
                ConstraintMode::Deferred => {
                    deferred_constraints.insert(name.clone());
                }
            }
        }
        if mode == ConstraintMode::Immediate {
            self.table
                .validate_schema_constraints(&deferred_constraints)?;
        }
        self.deferred_constraints = deferred_constraints;
        Ok(())
    }

    /// Executes a query against the transaction's private snapshot.
    pub fn query(&self, request: &QueryRequest) -> Result<Vec<Value>, QueryError> {
        query::execute_query(&self.table, request)
    }

    /// Commits all applied mutations through one WAL-backed table save.
    pub fn commit(mut self) -> Result<DbfTable, DbfError> {
        if let Some(lock) = self.serializable_lock.take() {
            self.table.save_with_wal_locked(&self.path, &lock)?;
        } else {
            self.table.save_with_wal(&self.path)?;
        }
        Ok(self.table)
    }

    /// Commits disjoint row changes on top of a newer table image.
    ///
    /// This explicit opt-in preserves the default stale-source rejection.
    /// Inserts, layout changes, schema changes, and different changes to the
    /// same row remain conflicts; an identical resulting row is a no-op.
    pub fn commit_with_row_merge(mut self) -> Result<DbfTable, DbfError> {
        if let Some(lock) = self.serializable_lock.take() {
            self.table.save_with_row_merge_locked(&self.path, &lock)?;
        } else {
            self.table.save_with_row_merge(&self.path)?;
        }
        Ok(self.table)
    }

    /// Discards the private snapshot without touching the path.
    pub fn rollback(self) {}
}

fn initially_deferred(table: &DbfTable) -> BTreeSet<String> {
    table
        .local_deferrable_constraint_modes()
        .into_iter()
        .filter_map(|(name, mode)| (mode == ConstraintMode::Deferred).then_some(name))
        .collect()
}

impl crate::query::QueryExecutor for DbfTransaction {
    fn execute(&self, request: &QueryRequest) -> Result<Vec<Value>, QueryError> {
        self.query(request)
    }
}
