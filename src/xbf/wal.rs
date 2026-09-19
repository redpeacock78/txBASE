use super::checksum::crc32c;
use super::persistence::{read_path, write_encoded_path};
use super::{XbfError, XbfTable, decode, encode};
use crate::transaction::{FileWal, Wal};
use std::fs;
use std::path::Path;

const RECORD_MAGIC: [u8; 4] = *b"XWLS";
const RECORD_VERSION: u16 = 1;
const RECORD_HEADER_SIZE: usize = 40;

pub fn save_with_wal(path: impl AsRef<Path>, table: &XbfTable) -> Result<(), XbfError> {
    let path = path.as_ref();
    recover_path(path)?;
    let base_generation = current_generation(path)?;
    if table.generation <= base_generation {
        return Err(XbfError::Invalid(format!(
            "XBF generation {} is not newer than the current generation {base_generation}",
            table.generation
        )));
    }
    let snapshot = encode(table)?;
    let wal_path = wal_path(path);
    let mut wal = FileWal::open(&wal_path).map_err(wal_error)?;
    wal.append(&encode_record(
        base_generation,
        table.generation,
        &snapshot,
    )?)
    .map_err(wal_error)?;
    wal.sync().map_err(wal_error)?;
    write_encoded_path(path, &snapshot)?;
    wal.clear().map_err(wal_error)?;
    drop(wal);
    remove_wal(&wal_path)?;
    Ok(())
}

pub fn recover_path(path: impl AsRef<Path>) -> Result<bool, XbfError> {
    let path = path.as_ref();
    let wal_path = wal_path(path);
    if !wal_path.exists() {
        return Ok(false);
    }
    let mut wal = FileWal::open(&wal_path).map_err(wal_error)?;
    if wal.records().is_empty() {
        wal.clear().map_err(wal_error)?;
        drop(wal);
        remove_wal(&wal_path)?;
        return Ok(false);
    }
    let current_generation = current_generation(path)?;
    let mut pending = None;
    for (_, payload) in wal.records() {
        pending = Some(decode_record(payload)?);
    }
    let (base_generation, target_generation, snapshot) = pending.expect("non-empty WAL");
    let table = decode(&snapshot)?;
    if table.generation != target_generation {
        return Err(XbfError::Invalid(
            "XBF WAL target generation does not match its snapshot".into(),
        ));
    }
    if current_generation == target_generation {
        remove_wal_after_clear(wal, &wal_path)?;
        return Ok(false);
    }
    if current_generation != base_generation {
        return Err(XbfError::Invalid(format!(
            "XBF WAL targets base generation {base_generation}, current generation is {current_generation}"
        )));
    }
    write_encoded_path(path, &snapshot)?;
    remove_wal_after_clear(wal, &wal_path)?;
    Ok(true)
}

pub(super) fn encode_record(
    base_generation: u64,
    target_generation: u64,
    snapshot: &[u8],
) -> Result<Vec<u8>, XbfError> {
    let snapshot_length = u64::try_from(snapshot.len())
        .map_err(|_| XbfError::Invalid("XBF WAL snapshot length overflows u64".into()))?;
    let mut bytes = Vec::with_capacity(RECORD_HEADER_SIZE + snapshot.len());
    bytes.extend_from_slice(&RECORD_MAGIC);
    bytes.extend_from_slice(&RECORD_VERSION.to_le_bytes());
    bytes.extend_from_slice(&0u16.to_le_bytes());
    bytes.extend_from_slice(&base_generation.to_le_bytes());
    bytes.extend_from_slice(&target_generation.to_le_bytes());
    bytes.extend_from_slice(&snapshot_length.to_le_bytes());
    bytes.extend_from_slice(&crc32c(snapshot).to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(snapshot);
    Ok(bytes)
}

fn decode_record(payload: &[u8]) -> Result<(u64, u64, Vec<u8>), XbfError> {
    if payload.len() < RECORD_HEADER_SIZE || payload[..4] != RECORD_MAGIC {
        return Err(XbfError::Invalid("XBF WAL record header is invalid".into()));
    }
    if u16::from_le_bytes(payload[4..6].try_into().expect("checked length")) != RECORD_VERSION {
        return Err(XbfError::Invalid(
            "unsupported XBF WAL record version".into(),
        ));
    }
    if u16::from_le_bytes(payload[6..8].try_into().expect("checked length")) != 0
        || u32::from_le_bytes(payload[36..40].try_into().expect("checked length")) != 0
    {
        return Err(XbfError::Invalid(
            "XBF WAL record contains unknown flags or reserved data".into(),
        ));
    }
    let base_generation = u64::from_le_bytes(payload[8..16].try_into().expect("checked length"));
    let target_generation = u64::from_le_bytes(payload[16..24].try_into().expect("checked length"));
    let length = usize::try_from(u64::from_le_bytes(
        payload[24..32].try_into().expect("checked length"),
    ))
    .map_err(|_| XbfError::Invalid("XBF WAL snapshot length overflows usize".into()))?;
    let end = RECORD_HEADER_SIZE
        .checked_add(length)
        .ok_or_else(|| XbfError::Invalid("XBF WAL snapshot range overflows".into()))?;
    if end != payload.len() {
        return Err(XbfError::Invalid(
            "XBF WAL snapshot length does not match the record".into(),
        ));
    }
    let snapshot = payload[RECORD_HEADER_SIZE..].to_vec();
    let expected_crc = u32::from_le_bytes(payload[32..36].try_into().expect("checked length"));
    if expected_crc != crc32c(&snapshot) {
        return Err(XbfError::Invalid(
            "XBF WAL snapshot checksum does not match".into(),
        ));
    }
    Ok((base_generation, target_generation, snapshot))
}

fn current_generation(path: &Path) -> Result<u64, XbfError> {
    match read_path(path) {
        Ok(table) => Ok(table.generation),
        Err(XbfError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(error) => Err(error),
    }
}

fn wal_path(path: &Path) -> std::path::PathBuf {
    path.with_extension("xwl")
}

fn wal_error(error: crate::transaction::TransactionError) -> XbfError {
    XbfError::Invalid(format!("XBF WAL error: {error}"))
}

fn remove_wal_after_clear(mut wal: FileWal, path: &Path) -> Result<(), XbfError> {
    wal.clear().map_err(wal_error)?;
    drop(wal);
    remove_wal(path)
}

fn remove_wal(path: &Path) -> Result<(), XbfError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}
