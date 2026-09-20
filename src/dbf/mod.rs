use crate::transaction::{FileWal, TransactionError, Wal};
use crate::xbase::OperationIr;
use serde_json::{Map, Number, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

#[cfg(test)]
mod cjk_tests;
mod codec;
mod codepages;
#[cfg(test)]
mod compatibility_tests;
mod lock;
mod maintenance;
#[cfg(test)]
mod malformed_memo_tests;
#[cfg(test)]
mod malformed_tests;
mod memo;
mod mutation;
mod mutation_auto;
mod mutation_memo;
#[cfg(test)]
mod mutation_model_tests;
mod parser;
#[cfg(test)]
mod parser_fuzz_tests;
mod persistence;
mod recovery;
#[cfg(test)]
mod recovery_fault_tests;
mod schema;
mod schema_export;
#[cfg(test)]
mod schema_export_tests;
mod schema_metadata;
#[cfg(test)]
mod schema_metadata_tests;
#[cfg(test)]
mod tests;
mod wal;
#[cfg(test)]
mod writer_tests;

use codec::{
    canonical_encoding_name, decode_field_with_encoding, decode_record_field_with_encoding,
    encode_character_with_encoding, encode_field_with_encoding, encode_null_flags, encoding_name,
    flag_is_set, hex, null_flag_layout, parse_fields, read_u16, read_u32, system_field_index,
    text_with_encoding, update_null_flags, value_text,
};
#[cfg(test)]
use codec::{decode_field, encode_field, text};
use lock::TableLock;
use memo::{
    binary_value, empty_memo_value, encode_memo_pointer, find_memo_path, is_sidecar_field,
    memo_index, sidecar_update, storage_value_without_sidecar,
};
use schema_metadata::SchemaMetadata;
#[cfg(test)]
use wal::{
    ByteDelta, DELTA_MAGIC, MEMO_SNAPSHOT_MAGIC, OPERATION_MAGIC, SNAPSHOT_MAGIC, apply_byte_delta,
    decode_operation_payload, decode_snapshot, transaction_id_payload,
};
use wal::{delta_payload, memo_snapshot_payload, operation_payload, snapshot_payload};

pub use maintenance::copy_table_files;

pub(crate) use schema_export::commit_schema_export;

pub(crate) fn memo_sidecar_path(path: &Path) -> Option<PathBuf> {
    find_memo_path(path)
}

const CLASSIC_HEADER_SIZE: usize = 32;
const CLASSIC_DESCRIPTOR_SIZE: usize = 32;
const LEVEL7_HEADER_SIZE: usize = 68;
const LEVEL7_DESCRIPTOR_SIZE: usize = 48;
const FIELD_TERMINATOR: u8 = 0x0d;
const EOF_MARKER: u8 = 0x1a;
const ACTIVE_RECORD: u8 = 0x20;
const DELETED_RECORD: u8 = 0x2a;
const DBT_BLOCK_SIZE: usize = 512;
const CURRENCY_SCALE: u64 = 10_000;
const MILLISECONDS_PER_DAY: u32 = 86_400_000;
const JULIAN_DAY_UNIX_EPOCH: i64 = 2_440_588;

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

    fn is_variable(&self) -> bool {
        matches!(self.field_type.to_ascii_uppercase(), b'Q' | b'V')
    }

    fn is_nullable(&self) -> bool {
        self.flags & 0x02 != 0
    }

    pub(crate) fn is_binary(&self) -> bool {
        matches!(
            self.field_type.to_ascii_uppercase(),
            b'Q' | b'G' | b'P' | b'W'
        ) || (self.field_type.eq_ignore_ascii_case(&b'B') && self.length != 8)
            || self.flags & 0x04 != 0
    }

    fn is_auto_increment(&self, version: u8) -> bool {
        self.field_type.eq_ignore_ascii_case(&b'+')
            || (version == 0x31
                && self.field_type.eq_ignore_ascii_case(&b'I')
                && self.flags & 0x0c == 0x0c)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct NullFlagBits {
    varlength: Option<usize>,
    nullable: Option<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DbfRecord {
    pub number: usize,
    pub deleted: bool,
    pub values: Map<String, Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MemoFormat {
    Dbase3,
    Dbase4,
    FoxPro,
}

impl MemoFormat {
    fn tag(self) -> u8 {
        match self {
            Self::Dbase3 => 0,
            Self::Dbase4 => 1,
            Self::FoxPro => 2,
        }
    }

    fn from_tag(tag: u8) -> Result<Self, DbfError> {
        match tag {
            0 => Ok(Self::Dbase3),
            1 => Ok(Self::Dbase4),
            2 => Ok(Self::FoxPro),
            _ => Err(DbfError::Invalid(format!(
                "unknown memo sidecar format tag {tag}"
            ))),
        }
    }

    fn extension(self) -> &'static str {
        match self {
            Self::Dbase3 | Self::Dbase4 => "dbt",
            Self::FoxPro => "fpt",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MemoFile {
    bytes: Vec<u8>,
    block_size: usize,
    format: MemoFormat,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MemoSnapshot {
    format: MemoFormat,
    bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum MemoUpdate {
    Text(String),
    Binary(Vec<u8>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PersistedState {
    path: PathBuf,
    dbf: Vec<u8>,
    memo: Option<Vec<u8>>,
    schema: Option<Vec<u8>>,
    transaction_id: Option<u64>,
}

type MemoUpdates = BTreeMap<(usize, String), MemoUpdate>;
type PreparedStorage = (Map<String, Value>, MemoUpdates);

#[derive(Debug, Clone, PartialEq)]
pub struct DbfTable {
    pub header: DbfHeader,
    pub fields: Vec<FieldDescriptor>,
    records: Vec<DbfRecord>,
    stored_values: Vec<Map<String, Value>>,
    bytes: Vec<u8>,
    memo: Option<MemoFile>,
    schema: Option<SchemaMetadata>,
    encoding_override: Option<String>,
    memo_updates: MemoUpdates,
    transaction_id: Option<u64>,
    source: Option<PersistedState>,
}

pub(crate) struct PreparedSnapshot {
    pub(crate) dbf: Vec<u8>,
    pub(crate) memo: Option<(PathBuf, Vec<u8>)>,
    pub(crate) index_payload: Option<Vec<u8>>,
}

fn memo_format_for_version(version: u8) -> Option<MemoFormat> {
    match version {
        0x83 => Some(MemoFormat::Dbase3),
        0x8b => Some(MemoFormat::Dbase4),
        0x30 | 0x31 | 0x32 | 0xf5 => Some(MemoFormat::FoxPro),
        _ => None,
    }
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

fn save_bytes_to(path: &Path, bytes: &[u8], temporary_extension: &str) -> Result<(), DbfError> {
    let temporary_path = path.with_extension(temporary_extension);
    let mut file = fs::File::create(&temporary_path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::rename(temporary_path, path)?;
    sync_parent_directory(path)?;
    Ok(())
}

#[cfg(unix)]
fn sync_parent_directory(path: &Path) -> Result<(), DbfError> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::File::open(parent)?.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
fn sync_parent_directory(_path: &Path) -> Result<(), DbfError> {
    Ok(())
}

fn transaction_error(error: TransactionError) -> DbfError {
    DbfError::Invalid(format!("WAL error: {error}"))
}

fn index_error(error: crate::index::IndexError) -> DbfError {
    DbfError::Invalid(format!("index sidecar error: {error}"))
}

fn write_record_count(bytes: &mut [u8], count: u32) -> Result<(), DbfError> {
    let header = bytes
        .get_mut(4..8)
        .ok_or_else(|| DbfError::Invalid("header is truncated".into()))?;
    header.copy_from_slice(&count.to_le_bytes());
    Ok(())
}
