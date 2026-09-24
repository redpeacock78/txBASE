use super::CatalogError;
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

const HISTORY_FILE: &str = ".txbase.catalog.mvcc";
const MAGIC: &[u8; 4] = b"TXCM";
const VERSION: u8 = 1;
const NO_MEMO: u8 = 0xff;
const HEADER_SIZE: usize = 9;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Snapshot {
    pub(crate) transaction_id: u64,
    pub(crate) tables: BTreeMap<String, TableSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TableSnapshot {
    pub(crate) file_name: String,
    pub(crate) dbf: Vec<u8>,
    pub(crate) memo: Option<(u8, Vec<u8>)>,
    pub(crate) schema: Option<Vec<u8>>,
}

pub(crate) fn path_for(root: &Path) -> PathBuf {
    root.join(HISTORY_FILE)
}

pub(crate) fn append_snapshot(
    existing: Option<&[u8]>,
    snapshot: Snapshot,
) -> Result<Vec<u8>, CatalogError> {
    // ponytail: rewrite the bounded full-image history; add segmented snapshots and retention when size matters.
    let mut history = existing.map(decode).transpose()?.unwrap_or_default();
    if history
        .last()
        .is_some_and(|current| current.transaction_id >= snapshot.transaction_id)
    {
        return Err(CatalogError::Invalid(format!(
            "catalog MVCC transaction ID {} is not newer than the history",
            snapshot.transaction_id
        )));
    }
    history.push(snapshot);
    encode(&history)
}

pub(crate) fn versions(root: &Path) -> Result<Vec<u64>, CatalogError> {
    Ok(read_history(root)?
        .into_iter()
        .map(|snapshot| snapshot.transaction_id)
        .collect())
}

pub(crate) fn gc(root: &Path, keep_last: usize) -> Result<Vec<u64>, CatalogError> {
    if keep_last == 0 {
        return Err(CatalogError::Invalid(
            "catalog MVCC GC keep count must be positive".into(),
        ));
    }
    let path = path_for(root);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let history = read_history(root)?;
    let mut retained = history
        .into_iter()
        .rev()
        .take(keep_last)
        .collect::<Vec<_>>();
    retained.reverse();
    let retained_ids = retained
        .iter()
        .map(|snapshot| snapshot.transaction_id)
        .collect::<Vec<_>>();
    let bytes = encode(&retained)?;
    let temporary = root.join(".txbase.catalog.mvcc.gc.tmp");
    let _ = fs::remove_file(&temporary);
    let result = (|| {
        let mut file = File::create(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        replace_history(&temporary, &path)?;
        sync_directory(root)?;
        Ok(retained_ids)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

pub(crate) fn snapshot_at(root: &Path, transaction_id: u64) -> Result<Snapshot, CatalogError> {
    if transaction_id == 0 {
        return Err(CatalogError::Invalid(
            "catalog MVCC transaction ID must be positive".into(),
        ));
    }
    read_history(root)?
        .into_iter()
        .find(|snapshot| snapshot.transaction_id == transaction_id)
        .ok_or_else(|| {
            CatalogError::Invalid(format!(
                "catalog MVCC snapshot {transaction_id} is not committed"
            ))
        })
}

pub(crate) fn snapshot_bytes(root: &Path, transaction_id: u64) -> Result<Vec<u8>, CatalogError> {
    encode_single_snapshot(&snapshot_at(root, transaction_id)?)
}

pub(crate) fn decode_single_snapshot(bytes: &[u8]) -> Result<Snapshot, CatalogError> {
    let mut snapshots = decode(bytes)?;
    if snapshots.len() != 1 {
        return Err(CatalogError::Invalid(
            "catalog snapshot payload must contain exactly one snapshot".into(),
        ));
    }
    Ok(snapshots.remove(0))
}

pub(crate) fn encode_single_snapshot(snapshot: &Snapshot) -> Result<Vec<u8>, CatalogError> {
    encode(std::slice::from_ref(snapshot))
}

pub(crate) fn validate(root: &Path) -> Result<(), CatalogError> {
    let _ = read_history(root)?;
    Ok(())
}

fn read_history(root: &Path) -> Result<Vec<Snapshot>, CatalogError> {
    let path = path_for(root);
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    decode(&bytes)
}

fn replace_history(source: &Path, destination: &Path) -> Result<(), CatalogError> {
    #[cfg(windows)]
    if destination.exists() {
        fs::remove_file(destination)?;
    }
    fs::rename(source, destination)?;
    Ok(())
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), CatalogError> {
    File::open(path)?.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<(), CatalogError> {
    Ok(())
}

fn encode(history: &[Snapshot]) -> Result<Vec<u8>, CatalogError> {
    let count = u32::try_from(history.len())
        .map_err(|_| CatalogError::Invalid("catalog MVCC history has too many snapshots".into()))?;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(MAGIC);
    bytes.push(VERSION);
    bytes.extend_from_slice(&count.to_le_bytes());
    for snapshot in history {
        if snapshot.transaction_id == 0 {
            return Err(CatalogError::Invalid(
                "catalog MVCC transaction ID must be positive".into(),
            ));
        }
        bytes.extend_from_slice(&snapshot.transaction_id.to_le_bytes());
        let table_count = u32::try_from(snapshot.tables.len()).map_err(|_| {
            CatalogError::Invalid("catalog MVCC snapshot has too many tables".into())
        })?;
        bytes.extend_from_slice(&table_count.to_le_bytes());
        for (name, table) in &snapshot.tables {
            append_string(&mut bytes, name, "table name")?;
            append_string(&mut bytes, &table.file_name, "table filename")?;
            let dbf_length = u64::try_from(table.dbf.len())
                .map_err(|_| CatalogError::Invalid("catalog MVCC DBF length overflows".into()))?;
            bytes.extend_from_slice(&dbf_length.to_le_bytes());
            let (memo_tag, memo_bytes) = table
                .memo
                .as_ref()
                .map_or((NO_MEMO, &[][..]), |(tag, bytes)| (*tag, bytes.as_slice()));
            if memo_tag != NO_MEMO && memo_tag > 2 {
                return Err(CatalogError::Invalid(format!(
                    "catalog MVCC memo format tag {memo_tag} is invalid"
                )));
            }
            bytes.push(memo_tag);
            let memo_length = u64::try_from(memo_bytes.len())
                .map_err(|_| CatalogError::Invalid("catalog MVCC memo length overflows".into()))?;
            let schema_bytes = table.schema.as_deref().unwrap_or_default();
            let schema_length = u64::try_from(schema_bytes.len()).map_err(|_| {
                CatalogError::Invalid("catalog MVCC schema length overflows".into())
            })?;
            bytes.extend_from_slice(&memo_length.to_le_bytes());
            bytes.extend_from_slice(&schema_length.to_le_bytes());
            bytes.extend_from_slice(&table.dbf);
            bytes.extend_from_slice(memo_bytes);
            bytes.extend_from_slice(schema_bytes);
        }
    }
    Ok(bytes)
}

fn decode(bytes: &[u8]) -> Result<Vec<Snapshot>, CatalogError> {
    if bytes.len() < HEADER_SIZE || &bytes[..MAGIC.len()] != MAGIC {
        return Err(CatalogError::Invalid(
            "catalog MVCC history header is invalid".into(),
        ));
    }
    if bytes[4] != VERSION {
        return Err(CatalogError::Invalid(format!(
            "unknown catalog MVCC history version {}",
            bytes[4]
        )));
    }
    let mut offset = 5;
    let snapshot_count = usize_from_u32(read_u32(bytes, &mut offset, "snapshot count")?);
    let mut history = Vec::with_capacity(snapshot_count);
    for _ in 0..snapshot_count {
        let transaction_id = read_u64(bytes, &mut offset, "transaction ID")?;
        if transaction_id == 0 {
            return Err(CatalogError::Invalid(
                "catalog MVCC transaction ID must be positive".into(),
            ));
        }
        if history
            .last()
            .is_some_and(|snapshot: &Snapshot| snapshot.transaction_id >= transaction_id)
        {
            return Err(CatalogError::Invalid(
                "catalog MVCC transaction IDs are not strictly increasing".into(),
            ));
        }
        let table_count = usize_from_u32(read_u32(bytes, &mut offset, "table count")?);
        let mut tables = BTreeMap::new();
        for _ in 0..table_count {
            let name = read_string(bytes, &mut offset, "table name")?;
            let file_name = read_string(bytes, &mut offset, "table filename")?;
            validate_table_identity(&name, &file_name)?;
            let dbf_length = length_from_u64(
                read_u64(bytes, &mut offset, "DBF length")?,
                bytes.len().saturating_sub(offset),
                "DBF",
            )?;
            let memo_tag = read_byte(bytes, &mut offset, "memo format tag")?;
            if memo_tag != NO_MEMO && memo_tag > 2 {
                return Err(CatalogError::Invalid(format!(
                    "catalog MVCC memo format tag {memo_tag} is invalid"
                )));
            }
            let memo_length = length_from_u64(
                read_u64(bytes, &mut offset, "memo length")?,
                bytes.len().saturating_sub(offset),
                "memo",
            )?;
            let schema_length = length_from_u64(
                read_u64(bytes, &mut offset, "schema length")?,
                bytes.len().saturating_sub(offset),
                "schema",
            )?;
            let dbf = take(bytes, &mut offset, dbf_length, "DBF")?;
            let memo_bytes = take(bytes, &mut offset, memo_length, "memo")?;
            let schema_bytes = take(bytes, &mut offset, schema_length, "schema")?;
            if tables
                .insert(
                    name,
                    TableSnapshot {
                        file_name,
                        dbf,
                        memo: (memo_tag != NO_MEMO).then_some((memo_tag, memo_bytes)),
                        schema: (schema_length != 0).then_some(schema_bytes),
                    },
                )
                .is_some()
            {
                return Err(CatalogError::Invalid(
                    "catalog MVCC snapshot contains a duplicate table".into(),
                ));
            }
        }
        history.push(Snapshot {
            transaction_id,
            tables,
        });
    }
    if offset != bytes.len() {
        return Err(CatalogError::Invalid(
            "catalog MVCC history has trailing bytes".into(),
        ));
    }
    Ok(history)
}

fn append_string(bytes: &mut Vec<u8>, value: &str, label: &str) -> Result<(), CatalogError> {
    let length = u32::try_from(value.len())
        .map_err(|_| CatalogError::Invalid(format!("catalog MVCC {label} is too long")))?;
    bytes.extend_from_slice(&length.to_le_bytes());
    bytes.extend_from_slice(value.as_bytes());
    Ok(())
}

fn validate_table_identity(name: &str, file_name: &str) -> Result<(), CatalogError> {
    if name.is_empty() || name.contains(['/', '\\']) {
        return Err(CatalogError::Invalid(
            "catalog MVCC table name is invalid".into(),
        ));
    }
    if file_name.is_empty()
        || file_name.contains(['/', '\\'])
        || !Path::new(file_name)
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("dbf"))
        || Path::new(file_name)
            .file_stem()
            .and_then(|stem| stem.to_str())
            != Some(name)
    {
        return Err(CatalogError::Invalid(
            "catalog MVCC table filename is invalid".into(),
        ));
    }
    Ok(())
}

fn read_byte(bytes: &[u8], offset: &mut usize, label: &str) -> Result<u8, CatalogError> {
    let value = *bytes
        .get(*offset)
        .ok_or_else(|| CatalogError::Invalid(format!("catalog MVCC {label} is truncated")))?;
    *offset += 1;
    Ok(value)
}

fn read_u32(bytes: &[u8], offset: &mut usize, label: &str) -> Result<u32, CatalogError> {
    let end = offset
        .checked_add(4)
        .ok_or_else(|| CatalogError::Invalid(format!("catalog MVCC {label} offset overflows")))?;
    let value = bytes
        .get(*offset..end)
        .ok_or_else(|| CatalogError::Invalid(format!("catalog MVCC {label} is truncated")))?;
    *offset = end;
    Ok(u32::from_le_bytes(
        value.try_into().expect("u32 slice is fixed"),
    ))
}

fn read_u64(bytes: &[u8], offset: &mut usize, label: &str) -> Result<u64, CatalogError> {
    let end = offset
        .checked_add(8)
        .ok_or_else(|| CatalogError::Invalid(format!("catalog MVCC {label} offset overflows")))?;
    let value = bytes
        .get(*offset..end)
        .ok_or_else(|| CatalogError::Invalid(format!("catalog MVCC {label} is truncated")))?;
    *offset = end;
    Ok(u64::from_le_bytes(
        value.try_into().expect("u64 slice is fixed"),
    ))
}

fn read_string(bytes: &[u8], offset: &mut usize, label: &str) -> Result<String, CatalogError> {
    let length = usize_from_u32(read_u32(bytes, offset, label)?);
    let value = take(bytes, offset, length, label)?;
    String::from_utf8(value)
        .map_err(|_| CatalogError::Invalid(format!("catalog MVCC {label} is not UTF-8")))
}

fn take(
    bytes: &[u8],
    offset: &mut usize,
    length: usize,
    label: &str,
) -> Result<Vec<u8>, CatalogError> {
    let end = offset
        .checked_add(length)
        .ok_or_else(|| CatalogError::Invalid(format!("catalog MVCC {label} length overflows")))?;
    let value = bytes
        .get(*offset..end)
        .ok_or_else(|| CatalogError::Invalid(format!("catalog MVCC {label} is truncated")))?;
    *offset = end;
    Ok(value.to_vec())
}

fn length_from_u64(value: u64, remaining: usize, label: &str) -> Result<usize, CatalogError> {
    let length = usize::try_from(value)
        .map_err(|_| CatalogError::Invalid(format!("catalog MVCC {label} length overflows")))?;
    if length > remaining {
        return Err(CatalogError::Invalid(format!(
            "catalog MVCC {label} length is truncated"
        )));
    }
    Ok(length)
}

fn usize_from_u32(value: u32) -> usize {
    value as usize
}

#[cfg(test)]
#[path = "mvcc_tests.rs"]
mod tests;
