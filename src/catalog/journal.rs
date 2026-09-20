use super::CatalogError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

const CATALOG_LOCK: &str = ".txbase.catalog.lock";
const JOURNAL_DIR: &str = ".txbase.catalog.txn";
const MANIFEST: &str = "manifest.json";
const TRANSACTION_STATE: &str = ".txbase.catalog.state";
const TRANSACTION_STATE_MAGIC: &[u8; 4] = b"TXCS";
const TRANSACTION_STATE_VERSION: u8 = 1;

pub(crate) struct CatalogReadLock {
    _file: File,
}

pub(crate) struct CatalogWriteLock {
    _file: File,
}

pub(crate) struct FileChange {
    pub(crate) target: PathBuf,
    pub(crate) before: Option<Vec<u8>>,
    pub(crate) after: Option<Vec<u8>>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Manifest {
    phase: Phase,
    changes: Vec<ManifestChange>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Phase {
    Prepared,
    Committed,
}

#[derive(Debug, Serialize, Deserialize)]
struct ManifestChange {
    target: String,
    before: Option<String>,
    after: Option<String>,
}

pub(crate) fn read_lock(root: &Path) -> Result<CatalogReadLock, CatalogError> {
    recover(root)?;
    let file = open_lock(root, false)?;
    read_transaction_id_locked(root)?;
    Ok(CatalogReadLock { _file: file })
}

pub(crate) fn write_lock(root: &Path) -> Result<CatalogWriteLock, CatalogError> {
    let file = open_lock(root, true)?;
    recover_locked(root)?;
    read_transaction_id_locked(root)?;
    Ok(CatalogWriteLock { _file: file })
}

pub(crate) fn transaction_id(root: &Path) -> Result<Option<u64>, CatalogError> {
    let _lock = read_lock(root)?;
    read_transaction_id_locked(root)
}

pub(crate) fn read_transaction_id_locked(root: &Path) -> Result<Option<u64>, CatalogError> {
    let path = root.join(TRANSACTION_STATE);
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    decode_transaction_id(&bytes).map(Some)
}

pub(crate) fn commit(root: &Path, mut changes: Vec<FileChange>) -> Result<u64, CatalogError> {
    if changes.is_empty() {
        return Ok(read_transaction_id_locked(root)?.unwrap_or(0));
    }
    let journal = root.join(JOURNAL_DIR);
    if journal.exists() {
        return Err(CatalogError::Invalid(
            "catalog transaction journal already exists".into(),
        ));
    }

    let state_path = root.join(TRANSACTION_STATE);
    let state_before = read_optional(&state_path)?;
    let transaction_id = next_transaction_id(state_before.as_deref())?;
    changes.push(FileChange {
        target: state_path,
        before: state_before,
        after: Some(transaction_state_bytes(transaction_id)),
    });

    let mut targets = BTreeSet::new();
    for change in &changes {
        let target = change
            .target
            .strip_prefix(root)
            .map_err(|_| CatalogError::Invalid("catalog transaction target escapes root".into()))?;
        if target.components().count() != 1 {
            return Err(CatalogError::Invalid(
                "catalog transaction targets must be direct children".into(),
            ));
        }
        let target = target.to_string_lossy().into_owned();
        if !targets.insert(target) {
            return Err(CatalogError::Invalid(
                "catalog transaction contains a duplicate target".into(),
            ));
        }
    }

    fs::create_dir(&journal)?;
    fs::create_dir(journal.join("before"))?;
    fs::create_dir(journal.join("after"))?;
    let result = (|| {
        let mut manifest = Manifest {
            phase: Phase::Prepared,
            changes: Vec::with_capacity(changes.len()),
        };
        for (index, change) in changes.iter().enumerate() {
            let target = change
                .target
                .strip_prefix(root)
                .map_err(|_| {
                    CatalogError::Invalid("catalog transaction target escapes root".into())
                })?
                .to_string_lossy()
                .into_owned();
            let before = change.before.as_ref().map(|bytes| {
                let name = index.to_string();
                (name, bytes)
            });
            let after = change.after.as_ref().map(|bytes| {
                let name = index.to_string();
                (name, bytes)
            });
            if let Some((name, bytes)) = before.as_ref() {
                write_synced(&journal.join("before").join(name), bytes)?;
            }
            if let Some((name, bytes)) = after.as_ref() {
                write_synced(&journal.join("after").join(name), bytes)?;
            }
            manifest.changes.push(ManifestChange {
                target,
                before: before.map(|(name, _)| name),
                after: after.map(|(name, _)| name),
            });
        }
        write_manifest(&journal, &manifest)?;

        for change in &manifest.changes {
            apply_change(root, &journal, change, false)?;
        }
        manifest.phase = Phase::Committed;
        write_manifest(&journal, &manifest)?;
        Ok::<(), CatalogError>(())
    })();

    result?;
    let _ = fs::remove_dir_all(&journal);
    sync_directory(root)?;
    Ok(transaction_id)
}

fn next_transaction_id(state: Option<&[u8]>) -> Result<u64, CatalogError> {
    state
        .map(decode_transaction_id)
        .transpose()?
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| CatalogError::Invalid("catalog transaction ID exhausted".into()))
}

fn decode_transaction_id(bytes: &[u8]) -> Result<u64, CatalogError> {
    let Some(body) = bytes.strip_prefix(TRANSACTION_STATE_MAGIC) else {
        return Err(CatalogError::Invalid(
            "catalog transaction state header is invalid".into(),
        ));
    };
    if body.len() != 9 {
        return Err(CatalogError::Invalid(
            "catalog transaction state payload is truncated".into(),
        ));
    }
    if body[0] != TRANSACTION_STATE_VERSION {
        return Err(CatalogError::Invalid(format!(
            "unknown catalog transaction state version {}",
            body[0]
        )));
    }
    let transaction_id = u64::from_le_bytes(
        body[1..9]
            .try_into()
            .expect("catalog transaction state payload is fixed length"),
    );
    if transaction_id == 0 {
        return Err(CatalogError::Invalid(
            "catalog transaction ID must be positive".into(),
        ));
    }
    Ok(transaction_id)
}

fn transaction_state_bytes(transaction_id: u64) -> Vec<u8> {
    let mut bytes = TRANSACTION_STATE_MAGIC.to_vec();
    bytes.push(TRANSACTION_STATE_VERSION);
    bytes.extend_from_slice(&transaction_id.to_le_bytes());
    bytes
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, CatalogError> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn recover(root: &Path) -> Result<(), CatalogError> {
    let file = open_lock(root, true)?;
    let result = recover_locked(root);
    drop(file);
    result
}

fn recover_locked(root: &Path) -> Result<(), CatalogError> {
    let journal = root.join(JOURNAL_DIR);
    if !journal.exists() {
        return Ok(());
    }
    let manifest_path = journal.join(MANIFEST);
    if !manifest_path.exists() {
        fs::remove_dir_all(journal)?;
        sync_directory(root)?;
        return Ok(());
    }
    let manifest: Manifest =
        serde_json::from_slice(&fs::read(&manifest_path)?).map_err(|error| {
            CatalogError::Invalid(format!("invalid catalog transaction journal: {error}"))
        })?;
    match manifest.phase {
        Phase::Prepared => {
            for change in manifest.changes.iter().rev() {
                apply_change(root, &journal, change, true)?;
            }
        }
        Phase::Committed => {
            for change in &manifest.changes {
                apply_change(root, &journal, change, false)?;
            }
        }
    }
    fs::remove_dir_all(journal)?;
    sync_directory(root)?;
    Ok(())
}

fn open_lock(root: &Path, exclusive: bool) -> Result<File, CatalogError> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join(CATALOG_LOCK))?;
    if exclusive {
        fs2::FileExt::lock_exclusive(&file)?;
    } else {
        fs2::FileExt::lock_shared(&file)?;
    }
    Ok(file)
}

fn write_manifest(journal: &Path, manifest: &Manifest) -> Result<(), CatalogError> {
    let path = journal.join(MANIFEST);
    let temporary = journal.join("manifest.json.tmp");
    let bytes = serde_json::to_vec(manifest).map_err(|error| {
        CatalogError::Invalid(format!("catalog transaction encoding failed: {error}"))
    })?;
    write_synced(&temporary, &bytes)?;
    #[cfg(windows)]
    if path.exists() {
        fs::remove_file(&path)?;
    }
    fs::rename(temporary, path)?;
    sync_directory(journal)?;
    Ok(())
}

fn write_synced(path: &Path, bytes: &[u8]) -> Result<(), CatalogError> {
    let mut file = File::create(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn apply_change(
    root: &Path,
    journal: &Path,
    change: &ManifestChange,
    rollback: bool,
) -> Result<(), CatalogError> {
    let staged_name = if rollback {
        change.before.as_deref()
    } else {
        change.after.as_deref()
    };
    let target = root.join(&change.target);
    let bytes = staged_name
        .map(|name| {
            fs::read(
                journal
                    .join(if rollback { "before" } else { "after" })
                    .join(name),
            )
        })
        .transpose()?;
    match bytes {
        Some(bytes) => write_target(&target, &bytes)?,
        None => remove_if_exists(&target)?,
    }
    Ok(())
}

fn write_target(path: &Path, bytes: &[u8]) -> Result<(), CatalogError> {
    let mut temporary = path.as_os_str().to_os_string();
    temporary.push(format!(".txbase-catalog-{}.tmp", std::process::id()));
    let temporary = PathBuf::from(temporary);
    let result = (|| {
        write_synced(&temporary, bytes)?;
        #[cfg(windows)]
        if path.exists() {
            fs::remove_file(path)?;
        }
        fs::rename(&temporary, path)?;
        sync_directory(path.parent().unwrap_or_else(|| Path::new(".")))?;
        Ok::<(), CatalogError>(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn remove_if_exists(path: &Path) -> Result<(), CatalogError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

    fn temporary_root() -> PathBuf {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "txbase-catalog-transaction-test-{}-{id}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir(&root).unwrap();
        root
    }

    fn prepared_journal(root: &Path, phase: Phase) {
        let journal = root.join(JOURNAL_DIR);
        fs::create_dir(&journal).unwrap();
        fs::create_dir(journal.join("before")).unwrap();
        fs::create_dir(journal.join("after")).unwrap();
        write_synced(&journal.join("before/0"), b"users-before").unwrap();
        write_synced(&journal.join("before/1"), b"posts-before").unwrap();
        write_synced(&journal.join("before/2"), &transaction_state_bytes(1)).unwrap();
        write_synced(&journal.join("after/0"), b"users-after").unwrap();
        write_synced(&journal.join("after/1"), b"posts-after").unwrap();
        write_synced(&journal.join("after/2"), &transaction_state_bytes(2)).unwrap();
        write_manifest(
            &journal,
            &Manifest {
                phase,
                changes: vec![
                    ManifestChange {
                        target: "users.dbf".into(),
                        before: Some("0".into()),
                        after: Some("0".into()),
                    },
                    ManifestChange {
                        target: "posts.dbf".into(),
                        before: Some("1".into()),
                        after: Some("1".into()),
                    },
                    ManifestChange {
                        target: TRANSACTION_STATE.into(),
                        before: Some("2".into()),
                        after: Some("2".into()),
                    },
                ],
            },
        )
        .unwrap();
    }

    #[test]
    fn prepared_journal_rolls_back_partial_catalog_commit() {
        let root = temporary_root();
        fs::write(root.join("users.dbf"), b"users-after").unwrap();
        fs::write(root.join("posts.dbf"), b"posts-before").unwrap();
        fs::write(root.join(TRANSACTION_STATE), transaction_state_bytes(2)).unwrap();
        prepared_journal(&root, Phase::Prepared);

        recover(&root).unwrap();

        assert_eq!(fs::read(root.join("users.dbf")).unwrap(), b"users-before");
        assert_eq!(fs::read(root.join("posts.dbf")).unwrap(), b"posts-before");
        assert_eq!(read_transaction_id_locked(&root).unwrap(), Some(1));
        assert!(!root.join(JOURNAL_DIR).exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn committed_journal_replays_the_complete_catalog_commit() {
        let root = temporary_root();
        fs::write(root.join("users.dbf"), b"users-before").unwrap();
        fs::write(root.join("posts.dbf"), b"posts-before").unwrap();
        fs::write(root.join(TRANSACTION_STATE), transaction_state_bytes(1)).unwrap();
        prepared_journal(&root, Phase::Committed);

        recover(&root).unwrap();

        assert_eq!(fs::read(root.join("users.dbf")).unwrap(), b"users-after");
        assert_eq!(fs::read(root.join("posts.dbf")).unwrap(), b"posts-after");
        assert_eq!(read_transaction_id_locked(&root).unwrap(), Some(2));
        assert!(!root.join(JOURNAL_DIR).exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_a_malformed_catalog_transaction_state() {
        let root = temporary_root();
        fs::write(root.join(TRANSACTION_STATE), b"not a catalog state").unwrap();

        let error = match read_lock(&root) {
            Ok(_) => panic!("malformed catalog state was accepted"),
            Err(error) => error,
        };
        assert!(
            error
                .to_string()
                .contains("catalog transaction state header is invalid")
        );
        fs::remove_dir_all(root).unwrap();
    }
}
