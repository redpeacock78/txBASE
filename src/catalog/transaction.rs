pub(crate) use super::journal::{CatalogReadLock, CatalogWriteLock, read_lock, write_lock};
use super::journal::{FileChange, commit};
use super::{Catalog, CatalogError, CatalogTransactionError};
use crate::dbf::DbfTable;
use crate::xbase::{OperationIr, OperationMethod};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

impl Catalog {
    pub(crate) fn commit_operations(
        &self,
        operations: &[OperationIr],
    ) -> Result<(), CatalogTransactionError> {
        if operations.is_empty() {
            return Err(CatalogTransactionError::Invalid(
                "operations must not be empty".into(),
            ));
        }
        let _lock = self
            .acquire_write_lock()
            .map_err(CatalogTransactionError::Catalog)?;
        let mut tables = BTreeMap::<String, DbfTable>::new();
        for operation in operations {
            let Some((name, local_path)) = transaction_operation_path(&operation.path) else {
                return Err(CatalogTransactionError::Invalid(format!(
                    "operation path is not a named record route: {}",
                    operation.path
                )));
            };
            if !matches!(
                operation.method,
                OperationMethod::Post
                    | OperationMethod::Put
                    | OperationMethod::Patch
                    | OperationMethod::Delete
            ) {
                return Err(CatalogTransactionError::Invalid(format!(
                    "operation method {} is not a mutation",
                    operation.method.as_str()
                )));
            }
            if self.table_path(name).is_none() {
                return Err(CatalogTransactionError::Invalid(format!(
                    "table not found: {name}"
                )));
            }
            if !tables.contains_key(name) {
                let table = self
                    .open_table_unlocked(name)
                    .map_err(CatalogTransactionError::Catalog)?;
                tables.insert(name.to_owned(), table);
            }
            let table = tables
                .get_mut(name)
                .expect("catalog transaction table was inserted");
            table
                .apply_operation(&OperationIr {
                    method: operation.method,
                    path: local_path,
                    body: operation.body.clone(),
                })
                .map_err(|error| {
                    CatalogTransactionError::Invalid(format!("table {name}: {error}"))
                })?;
        }
        self.validate_replacements(&tables)
            .map_err(CatalogTransactionError::Catalog)?;

        let mut changes = Vec::new();
        for (name, mut table) in tables {
            let path = self
                .table_path(&name)
                .expect("catalog transaction table path exists");
            let prepared = table.prepare_snapshot(path).map_err(|error| {
                CatalogTransactionError::Catalog(CatalogError::Table {
                    name: name.clone(),
                    source: error,
                })
            })?;
            changes.push(FileChange {
                target: path.to_path_buf(),
                before: Some(
                    fs::read(path).map_err(|error| {
                        CatalogTransactionError::Catalog(CatalogError::Io(error))
                    })?,
                ),
                after: Some(prepared.dbf),
            });
            if let Some((memo_path, bytes)) = prepared.memo {
                changes.push(FileChange {
                    target: memo_path.clone(),
                    before: read_optional(&memo_path).map_err(CatalogTransactionError::Catalog)?,
                    after: Some(bytes),
                });
            }
            if let Some(index_payload) = prepared.index_payload {
                let index_bytes = crate::index::decode_snapshot_payload(&index_payload)
                    .map_err(|error| {
                        CatalogTransactionError::Catalog(CatalogError::Invalid(format!(
                            "index snapshot error: {error}"
                        )))
                    })?
                    .ok_or_else(|| {
                        CatalogTransactionError::Catalog(CatalogError::Invalid(
                            "index snapshot payload is missing".into(),
                        ))
                    })?;
                let index_path = crate::index::sidecar_path(path);
                changes.push(FileChange {
                    target: index_path.clone(),
                    before: read_optional(&index_path).map_err(CatalogTransactionError::Catalog)?,
                    after: Some(index_bytes),
                });
            }
        }
        commit(&self.root, changes).map_err(CatalogTransactionError::Catalog)
    }
}

fn transaction_operation_path(path: &str) -> Option<(&str, String)> {
    let segments = path.strip_prefix('/')?.split('/').collect::<Vec<_>>();
    match segments.as_slice() {
        [table, "records"] if !table.is_empty() => Some((table, String::from("/records"))),
        [table, "records", record] if !table.is_empty() && !record.is_empty() => {
            Some((table, format!("/records/{record}")))
        }
        _ => None,
    }
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, CatalogError> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(CatalogError::Io(error)),
    }
}
