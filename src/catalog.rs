use crate::dbf::{DbfError, DbfTable};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::fs;
use std::path::{Path, PathBuf};

mod constraints;
mod journal;
mod transaction;

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

        let mut tables = BTreeMap::new();
        for entry in fs::read_dir(&root)? {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }

            let path = entry.path();
            let Some(extension) = path.extension().and_then(|value| value.to_str()) else {
                continue;
            };
            if !extension.eq_ignore_ascii_case("dbf") {
                continue;
            }

            let Some(name) = path
                .file_stem()
                .and_then(|value| value.to_str())
                .map(ToOwned::to_owned)
            else {
                return Err(CatalogError::Invalid(format!(
                    "DBF filename is not valid UTF-8: {}",
                    path.display()
                )));
            };
            if name.is_empty() {
                return Err(CatalogError::Invalid(format!(
                    "DBF filename has an empty table name: {}",
                    path.display()
                )));
            }
            if tables
                .insert(
                    name.to_owned(),
                    CatalogTable {
                        name: name.clone(),
                        file_name: entry.file_name().to_string_lossy().into_owned(),
                        path,
                    },
                )
                .is_some()
            {
                return Err(CatalogError::Invalid(format!(
                    "duplicate table name: {name}"
                )));
            }
        }

        Ok(Self { root, tables })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn transaction_id(&self) -> Result<Option<u64>, CatalogError> {
        journal::transaction_id(&self.root)
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
        let transaction_id = journal::read_transaction_id_locked(&self.root)?;
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
        for entry in self.tables() {
            let table = self.open_table_unlocked(entry.name())?;
            table.verify().map_err(|source| CatalogError::Table {
                name: entry.name().to_owned(),
                source,
            })?;
            if crate::index::sidecar_path(entry.path()).exists() {
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
}

fn representation_tag(schema: &Value) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in schema.to_string().bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("\"txbase-catalog-{hash:016x}\"")
}
