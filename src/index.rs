use crate::dbf::{DbfError, DbfTable};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::path::{Path, PathBuf};

const INDEX_FORMAT: &str = "txbase-index";
const INDEX_VERSION: u8 = 1;
const INDEX_EXTENSION: &str = "txidx";

mod storage;
mod validation;

#[derive(Debug)]
pub enum IndexError {
    Io(std::io::Error),
    Json(serde_json::Error),
    Dbf(DbfError),
    Invalid(String),
    Stale { path: PathBuf },
}

impl Display for IndexError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "index I/O error: {error}"),
            Self::Json(error) => write!(formatter, "invalid index JSON: {error}"),
            Self::Dbf(error) => write!(formatter, "indexed DBF error: {error}"),
            Self::Invalid(message) => write!(formatter, "invalid index: {message}"),
            Self::Stale { path } => {
                write!(formatter, "index sidecar is stale: {}", path.display())
            }
        }
    }
}

impl Error for IndexError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Json(error) => Some(error),
            Self::Dbf(error) => Some(error),
            Self::Invalid(_) | Self::Stale { .. } => None,
        }
    }
}

impl From<std::io::Error> for IndexError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for IndexError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

impl From<DbfError> for IndexError {
    fn from(error: DbfError) -> Self {
        Self::Dbf(error)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IndexDefinition {
    name: String,
    field: String,
}

impl IndexDefinition {
    pub fn for_field(field: impl Into<String>) -> Self {
        let field = field.into();
        Self {
            name: field.clone(),
            field,
        }
    }

    pub fn named(name: impl Into<String>, field: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            field: field.into(),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn field(&self) -> &str {
        &self.field
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "lowercase")]
pub enum IndexKey {
    Missing,
    Null,
    Scalar(Value),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IndexEntry {
    key: IndexKey,
    records: Vec<usize>,
}

impl IndexEntry {
    pub fn key(&self) -> &IndexKey {
        &self.key
    }

    pub fn records(&self) -> &[usize] {
        &self.records
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecondaryIndex {
    definition: IndexDefinition,
    entries: Vec<IndexEntry>,
}

impl SecondaryIndex {
    pub fn definition(&self) -> &IndexDefinition {
        &self.definition
    }

    pub fn entries(&self) -> &[IndexEntry] {
        &self.entries
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileFingerprint {
    length: u64,
    hash: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceFingerprint {
    dbf: FileFingerprint,
    memo: Option<FileFingerprint>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IndexFile {
    format: String,
    version: u8,
    source: SourceFingerprint,
    indexes: Vec<SecondaryIndex>,
}

pub fn sidecar_path(dbf_path: impl AsRef<Path>) -> PathBuf {
    dbf_path.as_ref().with_extension(INDEX_EXTENSION)
}

impl IndexFile {
    pub fn build(
        dbf_path: impl AsRef<Path>,
        definitions: Vec<IndexDefinition>,
    ) -> Result<Self, IndexError> {
        let dbf_path = dbf_path.as_ref();
        let table = DbfTable::from_path(dbf_path)?;
        let source = storage::source_fingerprint(dbf_path)?;
        let indexes = validation::build_indexes(&table, &definitions)?;
        Ok(Self {
            format: INDEX_FORMAT.to_owned(),
            version: INDEX_VERSION,
            source,
            indexes,
        })
    }

    pub fn load(dbf_path: impl AsRef<Path>) -> Result<Self, IndexError> {
        let dbf_path = dbf_path.as_ref();
        let table = DbfTable::from_path(dbf_path)?;
        let index_path = sidecar_path(dbf_path);
        let index = storage::read_sidecar(&index_path)?;
        if index.source != storage::source_fingerprint(dbf_path)? {
            return Err(IndexError::Stale { path: index_path });
        }
        validation::validate_for_table(&index, &table)?;
        Ok(index)
    }

    pub fn rebuild(dbf_path: impl AsRef<Path>) -> Result<Self, IndexError> {
        let dbf_path = dbf_path.as_ref();
        let existing = storage::read_sidecar(&sidecar_path(dbf_path))?;
        let definitions = existing
            .indexes
            .iter()
            .map(|index| index.definition.clone())
            .collect();
        let rebuilt = Self::build(dbf_path, definitions)?;
        rebuilt.save(dbf_path)?;
        Ok(rebuilt)
    }

    pub fn save(&self, dbf_path: impl AsRef<Path>) -> Result<(), IndexError> {
        let dbf_path = dbf_path.as_ref();
        let _table = DbfTable::from_path(dbf_path)?;
        validation::validate_shape(self)?;
        let index_path = sidecar_path(dbf_path);
        if self.source != storage::source_fingerprint(dbf_path)? {
            return Err(IndexError::Stale { path: index_path });
        }
        let bytes = serde_json::to_vec_pretty(self)?;
        storage::write_atomic(&index_path, &bytes)
    }

    pub fn index_names(&self) -> Vec<String> {
        self.indexes
            .iter()
            .map(|index| index.definition.name.clone())
            .collect()
    }

    pub fn lookup_eq(&self, index_name: &str, value: &Value) -> Result<Vec<usize>, IndexError> {
        let key = IndexKey::from_value(Some(value))?;
        let index = self
            .indexes
            .iter()
            .find(|index| index.definition.name == index_name)
            .ok_or_else(|| IndexError::Invalid(format!("index not found: {index_name}")))?;
        Ok(index
            .entries
            .iter()
            .find(|entry| entry.key == key)
            .map(|entry| entry.records.clone())
            .unwrap_or_default())
    }

    pub fn schema_json(&self) -> Value {
        json!({
            "format": self.format,
            "version": self.version,
            "source": self.source,
            "indexes": self.indexes.iter().map(|index| json!({
                "name": index.definition.name,
                "field": index.definition.field,
                "entry_count": index.entries.len(),
            })).collect::<Vec<_>>(),
        })
    }
}

#[cfg(test)]
#[path = "index_tests.rs"]
mod tests;
