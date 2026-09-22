use super::{DbfError, DbfTable};
use crate::query::{self, QueryError, QueryRequest};
use crate::xbase::OperationIr;
use serde_json::Value;
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
}

impl DbfTransaction {
    /// Opens a current table snapshot and starts a transaction.
    pub fn begin(path: impl AsRef<Path>) -> Result<Self, DbfError> {
        let path = path.as_ref().to_path_buf();
        let table = DbfTable::from_path(&path)?;
        Ok(Self { path, table })
    }

    /// Creates a transaction from an already loaded current table.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn from_table(path: impl AsRef<Path>, table: DbfTable) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
            table,
        }
    }

    /// Applies one mutation to the private transaction snapshot.
    pub fn apply(&mut self, operation: &OperationIr) -> Result<(), DbfError> {
        self.table.apply_operation(operation)
    }

    /// Executes a query against the transaction's private snapshot.
    pub fn query(&self, request: &QueryRequest) -> Result<Vec<Value>, QueryError> {
        query::execute_query(&self.table, request)
    }

    /// Commits all applied mutations through one WAL-backed table save.
    pub fn commit(mut self) -> Result<DbfTable, DbfError> {
        self.table.save_with_wal(&self.path)?;
        Ok(self.table)
    }

    /// Discards the private snapshot without touching the path.
    pub fn rollback(self) {}
}

impl crate::query::QueryExecutor for DbfTransaction {
    fn execute(&self, request: &QueryRequest) -> Result<Vec<Value>, QueryError> {
        self.query(request)
    }
}
