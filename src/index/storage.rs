use super::{FileFingerprint, IndexError, IndexFile, SourceFingerprint};
use crate::dbf::memo_sidecar_path;
use std::fs::{self, File};
use std::io::Write;
use std::path::Path;

pub(super) fn read_sidecar(path: &Path) -> Result<IndexFile, IndexError> {
    let index = read_sidecar_unvalidated(path)?;
    super::validation::validate_shape(&index)?;
    Ok(index)
}

pub(super) fn read_sidecar_unvalidated(path: &Path) -> Result<IndexFile, IndexError> {
    let bytes = fs::read(path)?;
    Ok(serde_json::from_slice::<IndexFile>(&bytes)?)
}

pub(super) fn source_fingerprint(path: &Path) -> Result<SourceFingerprint, IndexError> {
    let dbf = file_fingerprint(path)?;
    let memo = memo_sidecar_path(path)
        .map(|path| file_fingerprint(&path))
        .transpose()?;
    Ok(SourceFingerprint { dbf, memo })
}

pub(super) fn source_fingerprint_for(
    path: &Path,
    dbf_bytes: &[u8],
    memo_override: Option<&[u8]>,
) -> Result<SourceFingerprint, IndexError> {
    let dbf = fingerprint_bytes(dbf_bytes);
    let memo = memo_sidecar_path(path)
        .map(|path| {
            memo_override
                .map(fingerprint_bytes)
                .map(Ok)
                .unwrap_or_else(|| file_fingerprint(&path))
        })
        .transpose()?;
    Ok(SourceFingerprint { dbf, memo })
}

pub(super) fn source_fingerprint_for_without_memo(dbf_bytes: &[u8]) -> SourceFingerprint {
    SourceFingerprint {
        dbf: fingerprint_bytes(dbf_bytes),
        memo: None,
    }
}

fn file_fingerprint(path: &Path) -> Result<FileFingerprint, IndexError> {
    let bytes = fs::read(path)?;
    Ok(fingerprint_bytes(&bytes))
}

fn fingerprint_bytes(bytes: &[u8]) -> FileFingerprint {
    FileFingerprint {
        length: bytes.len() as u64,
        hash: fnv1a(bytes),
    }
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 14_695_981_039_346_656_037u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(1_099_511_628_211u64);
    }
    hash
}

pub(super) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), IndexError> {
    let temporary = path.with_extension("txidx.tmp");
    let result = (|| {
        let mut file = File::create(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        #[cfg(windows)]
        if path.exists() {
            fs::remove_file(path)?;
        }
        fs::rename(&temporary, path)?;
        sync_parent_directory(path)?;
        Ok::<(), std::io::Error>(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map_err(IndexError::Io)
}

#[cfg(unix)]
fn sync_parent_directory(path: &Path) -> Result<(), std::io::Error> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    File::open(parent)?.sync_all()
}

#[cfg(not(unix))]
fn sync_parent_directory(_path: &Path) -> Result<(), std::io::Error> {
    Ok(())
}
