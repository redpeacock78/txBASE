use super::JournalRecord;
use sha2::{Digest, Sha256};
use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

pub(super) const JOURNAL_MAGIC: &[u8; 5] = b"TXRL\x01";
const JOURNAL_PREFIX: &str = "raft-log-";
const JOURNAL_SUFFIX: &str = ".journal";
pub(super) const LOCK_NAME: &str = ".raft-log.lock";
pub(super) const FRAME_HEADER_BYTES: u64 = 4 + 32;
pub(super) const MAX_RECORD_BYTES: usize = crate::MAX_JSON_INPUT_BYTES + 16 * 1024;

pub(super) fn write_record(file: &mut File, record: &JournalRecord) -> io::Result<()> {
    let payload = serde_json::to_vec(record).map_err(|error| invalid_data(error.to_string()))?;
    if payload.is_empty() || payload.len() > MAX_RECORD_BYTES {
        return Err(invalid_data(
            "Raft log journal record exceeds its size limit",
        ));
    }
    let length = u32::try_from(payload.len())
        .map_err(|_| invalid_data("Raft log journal record length overflows"))?;
    let checksum: [u8; 32] = Sha256::digest(&payload).into();
    file.write_all(&length.to_le_bytes())?;
    file.write_all(&checksum)?;
    file.write_all(&payload)
}

pub(super) fn create_generation<F>(directory: &Path, generation: u64, write: F) -> io::Result<File>
where
    F: FnOnce(&mut File) -> io::Result<()>,
{
    let path = journal_path(directory, generation);
    let temporary = path.with_extension("tmp");
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .read(true)
        .append(true)
        .open(&temporary)?;
    let result = (|| {
        file.write_all(JOURNAL_MAGIC)?;
        write(&mut file)?;
        file.sync_all()?;
        fs::rename(&temporary, &path)?;
        sync_directory(directory)
    })();
    if let Err(error) = result {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    Ok(file)
}

pub(super) fn latest_generation(directory: &Path) -> io::Result<Option<u64>> {
    let mut latest = None;
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if let Some(generation) = parse_generation(&entry.file_name()) {
            latest = Some(latest.map_or(generation, |value: u64| value.max(generation)));
        }
    }
    Ok(latest)
}

pub(super) fn parse_generation(name: &OsStr) -> Option<u64> {
    let name = name.to_str()?;
    let digits = name
        .strip_prefix(JOURNAL_PREFIX)?
        .strip_suffix(JOURNAL_SUFFIX)?;
    if digits.len() != 20 || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

pub(super) fn journal_path(directory: &Path, generation: u64) -> PathBuf {
    directory.join(format!("{JOURNAL_PREFIX}{generation:020}{JOURNAL_SUFFIX}"))
}

#[cfg(unix)]
pub(super) fn sync_directory(directory: &Path) -> io::Result<()> {
    File::open(directory)?.sync_all()
}

#[cfg(not(unix))]
pub(super) fn sync_directory(_directory: &Path) -> io::Result<()> {
    Ok(())
}

pub(super) fn invalid_data(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
