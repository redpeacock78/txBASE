use crate::dbf::{DbfError, DbfTable};
use serde_json::{Value, json};
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::path::{Path, PathBuf};

const INDEX_FORMAT: &str = "txbase-index";
const INDEX_VERSION: u8 = 3;
const INDEX_EXTENSION: &str = "txidx";
pub(crate) const COST_PAGE_SIZE: usize = 4 * 1024;

mod commit;
mod lookup;
mod ordering;
mod statistics;
mod storage;
mod types;
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

use types::{FileFingerprint, SourceFingerprint};
pub use types::{IndexDefinition, IndexEntry, IndexFile, IndexKey, SecondaryIndex};

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
        let statistics = Some(statistics::build(table.active_records().count(), &indexes));
        Ok(Self {
            format: INDEX_FORMAT.to_owned(),
            version: INDEX_VERSION,
            source,
            statistics,
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
        let existing = storage::read_sidecar_unvalidated(&sidecar_path(dbf_path))?;
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

    pub(crate) fn active_record_count(&self) -> usize {
        self.statistics
            .as_ref()
            .map(|statistics| statistics.active_record_count)
            .unwrap_or_else(|| {
                self.indexes
                    .first()
                    .map(|index| index.entries.iter().map(|entry| entry.records.len()).sum())
                    .unwrap_or_default()
            })
    }

    pub(crate) fn source_dbf_page_count(&self) -> usize {
        usize::try_from(self.source.dbf.length)
            .unwrap_or(usize::MAX)
            .div_ceil(COST_PAGE_SIZE)
    }

    pub(crate) fn estimated_page_count(&self) -> usize {
        serde_json::to_vec_pretty(self)
            .map(|bytes| bytes.len().div_ceil(COST_PAGE_SIZE).max(1))
            .unwrap_or(1)
    }

    pub fn schema_json(&self) -> Value {
        json!({
            "format": self.format,
            "version": self.version,
            "source": self.source,
            "statistics": {
                "active_record_count": self.active_record_count(),
            },
            "indexes": self.indexes.iter().map(|index| {
                if index.definition.fields.len() == 1 {
                    json!({
                        "name": index.definition.name,
                        "field": index.definition.fields[0],
                        "entry_count": index.entries.len(),
                        "distinct_key_count": index.entries.len(),
                        "indexed_record_count": index.entries.iter().map(|entry| entry.records.len()).sum::<usize>(),
                        "histogram_bucket_count": self.statistics.as_ref().map_or(0, |statistics| statistics.histogram_bucket_count(&index.definition.name)),
                    })
                } else {
                    json!({
                        "name": index.definition.name,
                        "fields": index.definition.fields,
                        "directions": index.definition.directions,
                        "entry_count": index.entries.len(),
                        "distinct_key_count": index.entries.len(),
                        "indexed_record_count": index.entries.iter().map(|entry| entry.records.len()).sum::<usize>(),
                        "histogram_bucket_count": self.statistics.as_ref().map_or(0, |statistics| statistics.histogram_bucket_count(&index.definition.name)),
                    })
                }
            }).collect::<Vec<_>>(),
        })
    }
}

pub(crate) use commit::{
    apply_snapshot_payload, decode_snapshot_payload, pending_snapshot_payload, refresh_if_present,
    snapshot_bytes_without_memo, validate_snapshot_bytes,
};

#[cfg(test)]
#[path = "index_tests.rs"]
mod tests;
