use crate::dbf::{DbfError, DbfTable};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::fs;
use std::path::{Path, PathBuf};

mod cdc;
mod constraint_actions;
mod constraints;
mod discovery;
mod journal;
mod mvcc;
mod transaction;

pub use cdc::{CatalogChangeEvent, CatalogTableChange};

#[cfg(test)]
mod cdc_tests;
#[cfg(test)]
mod tests;

#[derive(Debug)]
pub enum CatalogError {
    Io(std::io::Error),
    Invalid(String),
    Table { name: String, source: DbfError },
}

impl Display for CatalogError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "catalog I/O error: {error}"),
            Self::Invalid(message) => write!(formatter, "invalid catalog: {message}"),
            Self::Table { name, source } => write!(formatter, "table {name}: {source}"),
        }
    }
}

impl Error for CatalogError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Table { source, .. } => Some(source),
            Self::Invalid(_) => None,
        }
    }
}

impl From<std::io::Error> for CatalogError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

#[derive(Debug)]
pub(crate) enum CatalogTransactionError {
    Invalid(String),
    PreconditionFailed { tag: String },
    Catalog(CatalogError),
}

impl Display for CatalogTransactionError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(message) => write!(formatter, "invalid catalog transaction: {message}"),
            Self::PreconditionFailed { .. } => {
                write!(formatter, "catalog transaction precondition failed")
            }
            Self::Catalog(error) => write!(formatter, "catalog transaction error: {error}"),
        }
    }
}

impl Error for CatalogTransactionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Invalid(_) | Self::PreconditionFailed { .. } => None,
            Self::Catalog(error) => Some(error),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogTable {
    name: String,
    file_name: String,
    path: PathBuf,
}

impl CatalogTable {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn file_name(&self) -> &str {
        &self.file_name
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Catalog {
    root: PathBuf,
    tables: BTreeMap<String, CatalogTable>,
    historical_snapshot: Option<mvcc::Snapshot>,
}

impl Catalog {
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, CatalogError> {
        let root = path.as_ref().to_path_buf();
        if !fs::metadata(&root)?.is_dir() {
            return Err(CatalogError::Invalid(format!(
                "catalog path is not a directory: {}",
                root.display()
            )));
        }
        let _lock = transaction::read_lock(&root)?;

        Ok(Self {
            tables: discovery::discover_tables(&root)?,
            root,
            historical_snapshot: None,
        })
    }

    pub fn from_path_at(path: impl AsRef<Path>, transaction_id: u64) -> Result<Self, CatalogError> {
        let root = path.as_ref().to_path_buf();
        if !fs::metadata(&root)?.is_dir() {
            return Err(CatalogError::Invalid(format!(
                "catalog path is not a directory: {}",
                root.display()
            )));
        }
        let _lock = transaction::read_lock(&root)?;
        let snapshot = mvcc::snapshot_at(&root, transaction_id)?;
        let tables = snapshot
            .tables
            .iter()
            .map(|(name, table)| {
                (
                    name.clone(),
                    CatalogTable {
                        name: name.clone(),
                        file_name: table.file_name.clone(),
                        path: root.join(&table.file_name),
                    },
                )
            })
            .collect();
        Ok(Self {
            root,
            tables,
            historical_snapshot: Some(snapshot),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn transaction_id(&self) -> Result<Option<u64>, CatalogError> {
        if let Some(snapshot) = &self.historical_snapshot {
            return Ok(Some(snapshot.transaction_id));
        }
        journal::transaction_id(&self.root)
    }

    pub fn cdc_events(
        path: impl AsRef<Path>,
        after: Option<u64>,
    ) -> Result<Vec<CatalogChangeEvent>, CatalogError> {
        let root = path.as_ref();
        if !fs::metadata(root)?.is_dir() {
            return Err(CatalogError::Invalid(format!(
                "catalog path is not a directory: {}",
                root.display()
            )));
        }
        let _lock = transaction::write_lock(root)?;
        cdc::read(root, after)
    }

    pub fn cdc_events_page(
        path: impl AsRef<Path>,
        after: Option<u64>,
        limit: usize,
    ) -> Result<(Vec<CatalogChangeEvent>, Option<u64>), CatalogError> {
        let root = path.as_ref();
        if !fs::metadata(root)?.is_dir() {
            return Err(CatalogError::Invalid(format!(
                "catalog path is not a directory: {}",
                root.display()
            )));
        }
        let _lock = transaction::write_lock(root)?;
        cdc::read_page(root, after, limit)
    }

    pub fn mvcc_versions(path: impl AsRef<Path>) -> Result<Vec<u64>, CatalogError> {
        let root = path.as_ref();
        let _lock = transaction::read_lock(root)?;
        mvcc::versions(root)
    }

    pub fn gc_mvcc(path: impl AsRef<Path>, keep_last: usize) -> Result<Vec<u64>, CatalogError> {
        let root = path.as_ref();
        let _lock = transaction::write_lock(root)?;
        mvcc::gc(root, keep_last)
    }

    pub fn table_names(&self) -> Vec<String> {
        self.tables.keys().cloned().collect()
    }

    pub fn tables(&self) -> impl Iterator<Item = &CatalogTable> {
        self.tables.values()
    }

    pub fn table_path(&self, name: &str) -> Option<&Path> {
        self.tables.get(name).map(CatalogTable::path)
    }

    pub fn open_table(&self, name: &str) -> Result<DbfTable, CatalogError> {
        let _lock = transaction::read_lock(&self.root)?;
        self.open_table_unlocked(name)
    }

    pub(crate) fn open_table_unlocked(&self, name: &str) -> Result<DbfTable, CatalogError> {
        let table = self
            .tables
            .get(name)
            .ok_or_else(|| CatalogError::Invalid(format!("table not found: {name}")))?;
        if let Some(snapshot) = &self.historical_snapshot {
            let table_snapshot = snapshot
                .tables
                .get(name)
                .ok_or_else(|| CatalogError::Invalid(format!("table not found: {name}")))?;
            return DbfTable::from_catalog_snapshot(
                &table_snapshot.dbf,
                table_snapshot.memo.clone(),
                table_snapshot.schema.as_deref(),
            )
            .map_err(|source| CatalogError::Table {
                name: table.name.clone(),
                source,
            });
        }
        DbfTable::from_path(table.path()).map_err(|source| CatalogError::Table {
            name: table.name.clone(),
            source,
        })
    }

    pub fn schema_json(&self) -> Result<Value, CatalogError> {
        let _lock = transaction::read_lock(&self.root)?;
        self.schema_json_unlocked()
    }

    pub(crate) fn schema_representation(&self) -> Result<(Value, String), CatalogError> {
        let _lock = transaction::read_lock(&self.root)?;
        self.schema_representation_unlocked()
    }

    pub(crate) fn schema_representation_unlocked(&self) -> Result<(Value, String), CatalogError> {
        let schema = self.schema_json_unlocked()?;
        let tag = representation_tag(&schema);
        Ok((schema, tag))
    }

    fn schema_json_unlocked(&self) -> Result<Value, CatalogError> {
        let transaction_id = match &self.historical_snapshot {
            Some(snapshot) => Some(snapshot.transaction_id),
            None => journal::read_transaction_id_locked(&self.root)?,
        };
        let mut tables = Vec::with_capacity(self.tables.len());
        for entry in self.tables() {
            let table = self.open_table_unlocked(entry.name())?;
            tables.push(json!({
                "name": entry.name(),
                "file": entry.file_name(),
                "schema": table.schema_json(),
            }));
        }
        Ok(json!({
            "format": "txbase-catalog",
            "transaction_id": transaction_id,
            "tables": tables,
        }))
    }

    pub fn verify(&self) -> Result<(), CatalogError> {
        let _lock = transaction::read_lock(&self.root)?;
        self.verify_unlocked()
    }

    fn verify_unlocked(&self) -> Result<(), CatalogError> {
        if self.historical_snapshot.is_none() {
            mvcc::validate(&self.root)?;
        }
        for entry in self.tables() {
            let table = self.open_table_unlocked(entry.name())?;
            table.verify().map_err(|source| CatalogError::Table {
                name: entry.name().to_owned(),
                source,
            })?;
            if self.historical_snapshot.is_none()
                && crate::index::sidecar_path(entry.path()).exists()
            {
                crate::index::IndexFile::load(entry.path()).map_err(|error| {
                    CatalogError::Table {
                        name: entry.name().to_owned(),
                        source: DbfError::Invalid(format!("index sidecar is invalid: {error}")),
                    }
                })?;
            }
        }
        self.validate_replacements(&BTreeMap::new())?;
        Ok(())
    }

    pub(crate) fn acquire_read_lock(&self) -> Result<transaction::CatalogReadLock, CatalogError> {
        transaction::read_lock(&self.root)
    }

    pub(crate) fn acquire_write_lock(&self) -> Result<transaction::CatalogWriteLock, CatalogError> {
        transaction::write_lock(&self.root)
    }

    pub(crate) fn open_tables(
        &self,
        left: &str,
        right: &str,
    ) -> Result<(DbfTable, DbfTable), CatalogError> {
        let _lock = transaction::read_lock(&self.root)?;
        Ok((
            self.open_table_unlocked(left)?,
            self.open_table_unlocked(right)?,
        ))
    }

    pub(crate) fn validate_replacements(
        &self,
        replacements: &BTreeMap<String, DbfTable>,
    ) -> Result<(), CatalogError> {
        constraints::validate_replacements(self, replacements)
    }

    pub(crate) fn is_historical(&self) -> bool {
        self.historical_snapshot.is_some()
    }
}

fn representation_tag(schema: &Value) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in schema.to_string().bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("\"txbase-catalog-{hash:016x}\"")
}
