use super::lock::TableLock;
use super::persistence::{next_transaction_id, read_transaction_state, transaction_state_bytes};
use super::schema_metadata::{SchemaMetadata, schema_metadata_path};
use super::{DbfError, DbfTable, sync_parent_directory};
use crate::transaction::{FileWal, Wal};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

const JOURNAL_MAGIC: [u8; 4] = *b"TXSE";
const JOURNAL_VERSION: u16 = 1;
const JOURNAL_RECORD_SIZE: usize = 8;
const TARGET_COUNT: usize = 8;
const DBF_TARGET: usize = 0;
const SCHEMA_TARGET: usize = 1;
const STATE_TARGET: usize = 6;
const INDEX_TARGET: usize = 7;

pub(crate) fn commit_schema_export(
    path: &Path,
    table: &DbfTable,
    schema_bytes: &[u8],
) -> Result<(), DbfError> {
    let _lock = TableLock::acquire(path)?;
    let _ = DbfTable::recover_wal_with_encoding(path, None)?;
    recover_schema_export_locked(path)?;

    commit_schema_export_locked(path, table, schema_bytes)
}

pub(crate) fn apply_schema_metadata(path: &Path, schema_bytes: &[u8]) -> Result<(), DbfError> {
    let _lock = TableLock::acquire(path)?;
    let _ = DbfTable::recover_wal_with_encoding(path, None)?;
    recover_schema_export_locked(path)?;

    let table = DbfTable::load_path_with_encoding(path, None)?;
    let metadata = SchemaMetadata::from_bytes(schema_bytes)?;
    metadata.validate_fields(&table.fields)?;
    metadata.validate_records(&table.records)?;
    write_bytes_atomically(&schema_metadata_path(path), schema_bytes, SCHEMA_TARGET)
}

fn commit_schema_export_locked(
    path: &Path,
    table: &DbfTable,
    schema_bytes: &[u8],
) -> Result<(), DbfError> {
    let dbf_bytes = table.to_bytes();
    validate_staged_export(&dbf_bytes, schema_bytes)?;
    let index_path = crate::index::sidecar_path(path);
    let index_bytes = match crate::index::snapshot_bytes_without_memo(path, table, &dbf_bytes)
        .map_err(super::index_error)?
    {
        Some(bytes) => Some(bytes),
        None if index_path.is_file() => Some(fs::read(&index_path)?),
        None => None,
    };
    let transaction_id = next_transaction_id(read_transaction_state(path)?)?;
    let state_bytes = transaction_state_bytes(transaction_id)?;
    let directory = transaction_directory(path);
    remove_directory_if_exists(&directory)?;
    fs::create_dir(&directory)?;

    write_file(&stage_path(&directory, DBF_TARGET), &dbf_bytes)?;
    write_file(&stage_path(&directory, SCHEMA_TARGET), schema_bytes)?;
    write_file(&stage_path(&directory, STATE_TARGET), &state_bytes)?;
    if let Some(bytes) = &index_bytes {
        write_file(&stage_path(&directory, INDEX_TARGET), bytes)?;
    }
    let flags = capture_bases(path, &directory)?;
    sync_directory(&directory)?;
    write_journal(path, flags)?;
    recover_schema_export_locked(path)?;
    Ok(())
}

pub(super) fn recover_schema_export_locked(path: &Path) -> Result<bool, DbfError> {
    let journal_path = journal_path(path);
    if !journal_path.exists() {
        return Ok(false);
    }

    let mut journal = FileWal::open(&journal_path).map_err(super::transaction_error)?;
    if journal.records().is_empty() {
        journal.clear().map_err(super::transaction_error)?;
        drop(journal);
        remove_file_if_exists(&journal_path)?;
        remove_directory_if_exists(&transaction_directory(path))?;
        sync_parent_directory(path)?;
        return Ok(false);
    }
    if journal.records().len() != 1 {
        return Err(DbfError::Invalid(
            "XBF schema export journal contains multiple records".into(),
        ));
    }
    let flags = decode_journal_record(&journal.records()[0].1)?;
    apply_export(path, flags)?;

    journal.clear().map_err(super::transaction_error)?;
    drop(journal);
    remove_file_if_exists(&journal_path)?;
    remove_directory_if_exists(&transaction_directory(path))?;
    sync_parent_directory(path)?;
    Ok(true)
}

fn apply_export(path: &Path, flags: u16) -> Result<(), DbfError> {
    let directory = transaction_directory(path);
    let dbf_bytes = fs::read(stage_path(&directory, DBF_TARGET))?;
    let schema_bytes = fs::read(stage_path(&directory, SCHEMA_TARGET))?;
    let state_bytes = read_optional(&stage_path(&directory, STATE_TARGET))?;
    let index_bytes = if flags & (1 << INDEX_TARGET) != 0 {
        Some(fs::read(stage_path(&directory, INDEX_TARGET))?)
    } else {
        None
    };
    validate_staged_export(&dbf_bytes, &schema_bytes)?;
    if let Some(bytes) = &index_bytes {
        crate::index::validate_snapshot_bytes(path, &dbf_bytes, bytes)
            .map_err(super::index_error)?;
    }

    let targets = target_paths(path);
    let desired = [
        Some(dbf_bytes.as_slice()),
        Some(schema_bytes.as_slice()),
        None,
        None,
        None,
        None,
        state_bytes.as_deref(),
        index_bytes.as_deref(),
    ];
    let mut replace = [false; TARGET_COUNT];
    for (index, target) in targets.iter().enumerate() {
        if index == STATE_TARGET && state_bytes.is_none() {
            continue;
        }
        let base = if flags & (1 << index) != 0 {
            Some(fs::read(base_path(&directory, index))?)
        } else {
            None
        };
        replace[index] = target_needs_replacement(target, desired[index], base.as_deref())?;
    }

    for (index, target) in targets.iter().enumerate() {
        if !replace[index] {
            continue;
        }
        let base = if flags & (1 << index) != 0 {
            Some(fs::read(base_path(&directory, index))?)
        } else {
            None
        };
        if !target_needs_replacement(target, desired[index], base.as_deref())? {
            continue;
        }
        if let Some(bytes) = desired[index] {
            write_bytes_atomically(target, bytes, index)?;
        } else {
            remove_file_if_exists(target)?;
            sync_parent_directory(target)?;
        }
    }
    Ok(())
}

fn target_needs_replacement(
    target: &Path,
    desired: Option<&[u8]>,
    base: Option<&[u8]>,
) -> Result<bool, DbfError> {
    let current = read_optional(target)?;
    if current.as_deref() == desired {
        return Ok(false);
    }
    if current.as_deref() != base {
        return Err(DbfError::Invalid(format!(
            "XBF schema export target changed during recovery: {}",
            target.display()
        )));
    }
    Ok(true)
}

fn validate_staged_export(dbf_bytes: &[u8], schema_bytes: &[u8]) -> Result<(), DbfError> {
    let table = DbfTable::from_bytes(dbf_bytes)?;
    table.verify()?;
    let metadata = SchemaMetadata::from_bytes(schema_bytes)?;
    metadata.validate_fields(&table.fields)?;
    metadata.validate_records(table.records())
}

fn capture_bases(path: &Path, directory: &Path) -> Result<u16, DbfError> {
    let mut flags = 0;
    for (index, target) in target_paths(path).iter().enumerate() {
        let Some(bytes) = read_optional(target)? else {
            continue;
        };
        write_file(&base_path(directory, index), &bytes)?;
        flags |= 1 << index;
    }
    Ok(flags)
}

fn write_journal(path: &Path, flags: u16) -> Result<(), DbfError> {
    let mut record = Vec::with_capacity(JOURNAL_RECORD_SIZE);
    record.extend_from_slice(&JOURNAL_MAGIC);
    record.extend_from_slice(&JOURNAL_VERSION.to_le_bytes());
    record.extend_from_slice(&flags.to_le_bytes());
    let journal_path = journal_path(path);
    let mut journal = FileWal::open(&journal_path).map_err(super::transaction_error)?;
    journal.append(&record).map_err(super::transaction_error)?;
    journal.sync().map_err(super::transaction_error)?;
    drop(journal);
    sync_parent_directory(path)?;
    Ok(())
}

fn decode_journal_record(record: &[u8]) -> Result<u16, DbfError> {
    if record.len() != JOURNAL_RECORD_SIZE || record[..4] != JOURNAL_MAGIC {
        return Err(DbfError::Invalid(
            "XBF schema export journal header is invalid".into(),
        ));
    }
    if u16::from_le_bytes(record[4..6].try_into().expect("checked length")) != JOURNAL_VERSION {
        return Err(DbfError::Invalid(
            "unsupported XBF schema export journal version".into(),
        ));
    }
    let flags = u16::from_le_bytes(record[6..8].try_into().expect("checked length"));
    if flags & !((1 << TARGET_COUNT) - 1) != 0 {
        return Err(DbfError::Invalid(
            "XBF schema export journal contains unknown flags".into(),
        ));
    }
    Ok(flags)
}

fn target_paths(path: &Path) -> [PathBuf; TARGET_COUNT] {
    [
        path.to_path_buf(),
        schema_metadata_path(path),
        path.with_extension("dbt"),
        path.with_extension("DBT"),
        path.with_extension("fpt"),
        path.with_extension("FPT"),
        super::persistence::transaction_state_path(path),
        crate::index::sidecar_path(path),
    ]
}

fn transaction_directory(path: &Path) -> PathBuf {
    append_suffix(path, ".txbase-xbf-export")
}

fn journal_path(path: &Path) -> PathBuf {
    path.with_extension("txbase.xbf-export.wal")
}

fn stage_path(directory: &Path, index: usize) -> PathBuf {
    directory.join(match index {
        DBF_TARGET => "dbf",
        SCHEMA_TARGET => "schema",
        STATE_TARGET => "state",
        INDEX_TARGET => "index",
        _ => "unused",
    })
}

fn base_path(directory: &Path, index: usize) -> PathBuf {
    directory.join(format!("base-{index}"))
}

fn append_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut value = path.as_os_str().to_os_string();
    value.push(suffix);
    value.into()
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<(), DbfError> {
    let mut file = File::create(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), DbfError> {
    File::open(path)?.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<(), DbfError> {
    Ok(())
}

fn write_bytes_atomically(path: &Path, bytes: &[u8], index: usize) -> Result<(), DbfError> {
    let temporary = append_suffix(path, &format!(".txbase-xbf-export-apply-{index}.tmp"));
    let result = (|| {
        write_file(&temporary, bytes)?;
        replace_file(&temporary, path)?;
        sync_parent_directory(path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn replace_file(source: &Path, destination: &Path) -> Result<(), std::io::Error> {
    #[cfg(windows)]
    if destination.exists() {
        fs::remove_file(destination)?;
    }
    fs::rename(source, destination)
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, DbfError> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn remove_file_if_exists(path: &Path) -> Result<(), DbfError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn remove_directory_if_exists(path: &Path) -> Result<(), DbfError> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
pub(super) fn test_journal_path(path: &Path) -> PathBuf {
    journal_path(path)
}

#[cfg(test)]
pub(super) fn test_transaction_directory(path: &Path) -> PathBuf {
    transaction_directory(path)
}

#[cfg(test)]
pub(super) fn test_stage_path(directory: &Path, index: usize) -> PathBuf {
    stage_path(directory, index)
}

#[cfg(test)]
pub(super) fn test_base_path(directory: &Path, index: usize) -> PathBuf {
    base_path(directory, index)
}

#[cfg(test)]
pub(super) fn test_write_journal(path: &Path, flags: u16) -> Result<(), DbfError> {
    write_journal(path, flags)
}

#[cfg(test)]
pub(super) fn test_write_journal_record(path: &Path, record: &[u8]) -> Result<(), DbfError> {
    let journal_path = journal_path(path);
    let mut journal = FileWal::open(&journal_path).map_err(super::transaction_error)?;
    journal.append(record).map_err(super::transaction_error)?;
    journal.sync().map_err(super::transaction_error)
}

#[cfg(test)]
pub(super) fn test_write_file(path: &Path, bytes: &[u8]) -> Result<(), DbfError> {
    write_file(path, bytes)
}
