use crate::transaction::{FileWal, TransactionError, Wal};
use crate::xbase::OperationIr;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

mod cdc;
#[cfg(test)]
mod cdc_tests;
#[cfg(test)]
mod cjk_fields_tests;
#[cfg(test)]
mod cjk_tests;
mod codec;
mod codepages;
#[cfg(test)]
mod compatibility_tests;
#[cfg(test)]
mod encoding_name_tests;
mod initializer;
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
mod mvcc;
#[cfg(test)]
mod mvcc_tests;
mod parser;
#[cfg(test)]
mod parser_fuzz_tests;
mod persistence;
mod recovery;
#[cfg(test)]
mod recovery_fault_tests;
mod row_mvcc;
mod schema;
mod schema_export;
#[cfg(test)]
mod schema_export_tests;
mod schema_metadata;
#[cfg(test)]
mod schema_metadata_tests;
mod transaction;
#[cfg(test)]
mod transaction_tests;
mod types;
#[cfg(test)]
mod upstream_cjk_tests;

#[cfg(test)]
mod schema_encoding_tests;
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
pub use initializer::DbfFieldSpec;
use lock::TableLock;
use memo::{
    binary_value, empty_memo_value, encode_memo_pointer, find_memo_path, is_sidecar_field,
    memo_index, sidecar_update, storage_value_without_sidecar,
};
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use types::PreparedSnapshot;
pub(super) use types::PreparedStorage;
pub use types::{DbfError, DbfHeader, DbfRecord, DbfTable, FieldDescriptor, RowId, RowVersion};
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use types::{
    ForeignKey, ForeignKeyAction, MemoFile, MemoFormat, MemoSnapshot, MemoUpdate, NullFlagBits,
    PersistedState,
};
#[cfg(target_arch = "wasm32")]
pub(crate) use types::{
    MemoFile, MemoFormat, MemoSnapshot, MemoUpdate, NullFlagBits, PersistedState,
};
#[cfg(test)]
use wal::{
    ByteDelta, DELTA_MAGIC, MEMO_SNAPSHOT_MAGIC, OPERATION_MAGIC, SNAPSHOT_MAGIC, apply_byte_delta,
    decode_operation_payload, decode_snapshot, transaction_id_payload,
};
use wal::{delta_payload, memo_snapshot_payload, operation_payload, snapshot_payload};

pub use cdc::{ChangeEvent, ChangeRecord, ChangeState};
pub use maintenance::copy_table_files;
pub use transaction::DbfTransaction;

pub(crate) use schema_export::commit_schema_export;

pub fn apply_schema_metadata(path: impl AsRef<Path>, schema_bytes: &[u8]) -> Result<(), DbfError> {
    schema_export::apply_schema_metadata(path.as_ref(), schema_bytes)
}

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

fn memo_format_for_version(version: u8) -> Option<MemoFormat> {
    match version {
        0x83 => Some(MemoFormat::Dbase3),
        0x8b => Some(MemoFormat::Dbase4),
        0x30 | 0x31 | 0x32 | 0xf5 => Some(MemoFormat::FoxPro),
        _ => None,
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
