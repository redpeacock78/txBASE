use super::schema_metadata::SchemaMetadata;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::path::PathBuf;

#[derive(Debug)]
pub enum DbfError {
    Io(std::io::Error),
    Invalid(String),
}

impl Display for DbfError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "I/O error: {error}"),
            Self::Invalid(message) => write!(f, "invalid DBF: {message}"),
        }
    }
}

impl Error for DbfError {}

impl From<std::io::Error> for DbfError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DbfHeader {
    pub version: u8,
    pub last_update: [u8; 3],
    pub record_count: u32,
    pub header_length: u16,
    pub record_length: u16,
    pub language_driver: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldDescriptor {
    pub name: String,
    pub field_type: u8,
    pub length: u8,
    pub decimal_count: u8,
    pub flags: u8,
    pub offset: usize,
}

impl FieldDescriptor {
    pub(crate) fn is_system(&self) -> bool {
        self.flags & 0x01 != 0 || self.name.eq_ignore_ascii_case("_NULLFLAGS")
    }

    pub(super) fn is_variable(&self) -> bool {
        matches!(self.field_type.to_ascii_uppercase(), b'Q' | b'V')
    }

    pub(crate) fn is_nullable(&self) -> bool {
        self.flags & 0x02 != 0
    }

    pub(crate) fn is_binary(&self) -> bool {
        matches!(
            self.field_type.to_ascii_uppercase(),
            b'Q' | b'G' | b'P' | b'W'
        ) || (self.field_type.eq_ignore_ascii_case(&b'B') && self.length != 8)
            || self.flags & 0x04 != 0
    }

    pub(super) fn is_auto_increment(&self, version: u8) -> bool {
        self.field_type.eq_ignore_ascii_case(&b'+')
            || (version == 0x31
                && self.field_type.eq_ignore_ascii_case(&b'I')
                && self.flags & 0x0c == 0x0c)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NullFlagBits {
    pub(super) varlength: Option<usize>,
    pub(super) nullable: Option<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DbfRecord {
    pub number: usize,
    pub deleted: bool,
    pub values: Map<String, Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct RowId {
    pub epoch: u64,
    pub record_number: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RowVersion {
    pub id: RowId,
    pub transaction_id: u64,
    pub deleted: bool,
    pub values: Map<String, Value>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ForeignKeyAction {
    #[default]
    Restrict,
    Cascade,
    SetNull,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ForeignKey {
    pub(crate) local_fields: Vec<String>,
    pub(crate) parent_table: String,
    pub(crate) parent_fields: Vec<String>,
    pub(crate) on_delete: ForeignKeyAction,
    pub(crate) on_update: ForeignKeyAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoFormat {
    Dbase3,
    Dbase4,
    FoxPro,
}

impl MemoFormat {
    pub(super) fn tag(self) -> u8 {
        match self {
            Self::Dbase3 => 0,
            Self::Dbase4 => 1,
            Self::FoxPro => 2,
        }
    }

    pub(super) fn from_tag(tag: u8) -> Result<Self, DbfError> {
        match tag {
            0 => Ok(Self::Dbase3),
            1 => Ok(Self::Dbase4),
            2 => Ok(Self::FoxPro),
            _ => Err(DbfError::Invalid(format!(
                "unknown memo sidecar format tag {tag}"
            ))),
        }
    }

    pub(super) fn extension(self) -> &'static str {
        match self {
            Self::Dbase3 | Self::Dbase4 => "dbt",
            Self::FoxPro => "fpt",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoFile {
    pub(super) bytes: Vec<u8>,
    pub(super) block_size: usize,
    pub(super) format: MemoFormat,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoSnapshot {
    pub(super) format: MemoFormat,
    pub(super) bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoUpdate {
    Text(String),
    Binary(Vec<u8>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistedState {
    pub(super) path: PathBuf,
    pub(super) dbf: Vec<u8>,
    pub(super) memo: Option<Vec<u8>>,
    pub(super) schema: Option<Vec<u8>>,
    pub(super) transaction_id: Option<u64>,
}

pub type MemoUpdates = BTreeMap<(usize, String), MemoUpdate>;
pub type PreparedStorage = (Map<String, Value>, MemoUpdates);

#[derive(Debug, Clone, PartialEq)]
pub struct DbfTable {
    pub header: DbfHeader,
    pub fields: Vec<FieldDescriptor>,
    pub(super) records: Vec<DbfRecord>,
    pub(super) stored_values: Vec<Map<String, Value>>,
    pub(super) bytes: Vec<u8>,
    pub(super) memo: Option<MemoFile>,
    pub(super) schema: Option<SchemaMetadata>,
    pub(super) encoding_override: Option<String>,
    pub(super) memo_updates: MemoUpdates,
    pub(super) transaction_id: Option<u64>,
    pub(super) source: Option<PersistedState>,
    pub(super) historical_snapshot: bool,
    pub(super) layout_changed: bool,
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) struct PreparedSnapshot {
    pub(crate) dbf: Vec<u8>,
    pub(crate) memo: Option<(PathBuf, Vec<u8>)>,
    pub(crate) index_payload: Option<Vec<u8>>,
}

impl DbfTable {
    pub fn records(&self) -> &[DbfRecord] {
        &self.records
    }

    pub fn active_records(&self) -> impl Iterator<Item = &DbfRecord> {
        self.records.iter().filter(|record| !record.deleted)
    }

    pub fn active_json(&self) -> Vec<Value> {
        self.active_records()
            .map(|record| Value::Object(record.values.clone()))
            .collect()
    }

    pub fn active_record(&self, number: usize) -> Option<&DbfRecord> {
        number
            .checked_sub(1)
            .and_then(|index| self.records.get(index).filter(|record| !record.deleted))
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        self.bytes.clone()
    }

    pub fn transaction_id(&self) -> Option<u64> {
        self.transaction_id
    }

    pub(crate) fn xbf_field_constraints(
        &self,
    ) -> Result<BTreeMap<String, (bool, bool, bool)>, DbfError> {
        self.schema.as_ref().map_or_else(
            || Ok(BTreeMap::new()),
            SchemaMetadata::xbf_field_constraints,
        )
    }

    pub(crate) fn representation_hash(&self) -> u64 {
        let mut representation = self.to_bytes();
        representation.extend_from_slice(
            &serde_json::to_vec(&self.active_json()).expect("DBF values must be JSON serializable"),
        );
        let mut hash = 0xcbf29ce484222325;
        for byte in representation {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
        hash
    }
}
