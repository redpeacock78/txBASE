use super::{Catalog, CatalogError, CatalogTransactionError};
use crate::dbf::DbfTable;
use crate::xbase::{OperationIr, OperationMethod};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

const CATALOG_LOCK: &str = ".txbase.catalog.lock";
const JOURNAL_DIR: &str = ".txbase.catalog.txn";
const MANIFEST: &str = "manifest.json";

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
    Ok(CatalogReadLock { _file: file })
}

pub(crate) fn write_lock(root: &Path) -> Result<CatalogWriteLock, CatalogError> {
    let file = open_lock(root, true)?;
    recover_locked(root)?;
    Ok(CatalogWriteLock { _file: file })
}

pub(crate) fn commit(root: &Path, changes: Vec<FileChange>) -> Result<(), CatalogError> {
    if changes.is_empty() {
        return Ok(());
    }
    let journal = root.join(JOURNAL_DIR);
    if journal.exists() {
        return Err(CatalogError::Invalid(
            "catalog transaction journal already exists".into(),
        ));
    }

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

    if result.is_err() {
        return result;
    }
    let _ = fs::remove_dir_all(&journal);
    sync_directory(root)?;
    Ok(())
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
        file.lock_exclusive()?;
    } else {
        file.lock_shared()?;
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

impl Catalog {
    pub(crate) fn commit_operations(
        &self,
        operations: &[OperationIr],
    ) -> Result<(), CatalogTransactionError> {
        if operations.is_empty() {
            return Err(CatalogTransactionError::Invalid(
                "operations must not be empty".into(),
            ));
        }
        let _lock = self
            .acquire_write_lock()
            .map_err(CatalogTransactionError::Catalog)?;
        let mut tables = BTreeMap::<String, DbfTable>::new();
        for operation in operations {
            let Some((name, local_path)) = transaction_operation_path(&operation.path) else {
                return Err(CatalogTransactionError::Invalid(format!(
                    "operation path is not a named record route: {}",
                    operation.path
                )));
            };
            if !matches!(
                operation.method,
                OperationMethod::Post
                    | OperationMethod::Put
                    | OperationMethod::Patch
                    | OperationMethod::Delete
            ) {
                return Err(CatalogTransactionError::Invalid(format!(
                    "operation method {} is not a mutation",
                    operation.method.as_str()
                )));
            }
            if self.table_path(name).is_none() {
                return Err(CatalogTransactionError::Invalid(format!(
                    "table not found: {name}"
                )));
            }
            if !tables.contains_key(name) {
                let table = self
                    .open_table_unlocked(name)
                    .map_err(CatalogTransactionError::Catalog)?;
                tables.insert(name.to_owned(), table);
            }
            let table = tables
                .get_mut(name)
                .expect("catalog transaction table was inserted");
            table
                .apply_operation(&OperationIr {
                    method: operation.method,
                    path: local_path,
                    body: operation.body.clone(),
                })
                .map_err(|error| {
                    CatalogTransactionError::Invalid(format!("table {name}: {error}"))
                })?;
        }

        let mut changes = Vec::new();
        for (name, mut table) in tables {
            let path = self
                .table_path(&name)
                .expect("catalog transaction table path exists");
            let prepared = table.prepare_snapshot(path).map_err(|error| {
                CatalogTransactionError::Catalog(CatalogError::Table {
                    name: name.clone(),
                    source: error,
                })
            })?;
            changes.push(FileChange {
                target: path.to_path_buf(),
                before: Some(
                    fs::read(path).map_err(|error| {
                        CatalogTransactionError::Catalog(CatalogError::Io(error))
                    })?,
                ),
                after: Some(prepared.dbf),
            });
            if let Some((memo_path, bytes)) = prepared.memo {
                changes.push(FileChange {
                    target: memo_path.clone(),
                    before: read_optional(&memo_path).map_err(CatalogTransactionError::Catalog)?,
                    after: Some(bytes),
                });
            }
            if let Some(index_payload) = prepared.index_payload {
                let index_bytes = crate::index::decode_snapshot_payload(&index_payload)
                    .map_err(|error| {
                        CatalogTransactionError::Catalog(CatalogError::Invalid(format!(
                            "index snapshot error: {error}"
                        )))
                    })?
                    .ok_or_else(|| {
                        CatalogTransactionError::Catalog(CatalogError::Invalid(
                            "index snapshot payload is missing".into(),
                        ))
                    })?;
                let index_path = crate::index::sidecar_path(path);
                changes.push(FileChange {
                    target: index_path.clone(),
                    before: read_optional(&index_path).map_err(CatalogTransactionError::Catalog)?,
                    after: Some(index_bytes),
                });
            }
        }
        commit(&self.root, changes).map_err(CatalogTransactionError::Catalog)
    }
}

fn transaction_operation_path(path: &str) -> Option<(&str, String)> {
    let segments = path.strip_prefix('/')?.split('/').collect::<Vec<_>>();
    match segments.as_slice() {
        [table, "records"] if !table.is_empty() => Some((table, String::from("/records"))),
        [table, "records", record] if !table.is_empty() && !record.is_empty() => {
            Some((table, format!("/records/{record}")))
        }
        _ => None,
    }
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, CatalogError> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(CatalogError::Io(error)),
    }
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
        write_synced(&journal.join("after/0"), b"users-after").unwrap();
        write_synced(&journal.join("after/1"), b"posts-after").unwrap();
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
        prepared_journal(&root, Phase::Prepared);

        recover(&root).unwrap();

        assert_eq!(fs::read(root.join("users.dbf")).unwrap(), b"users-before");
        assert_eq!(fs::read(root.join("posts.dbf")).unwrap(), b"posts-before");
        assert!(!root.join(JOURNAL_DIR).exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn committed_journal_replays_the_complete_catalog_commit() {
        let root = temporary_root();
        fs::write(root.join("users.dbf"), b"users-before").unwrap();
        fs::write(root.join("posts.dbf"), b"posts-before").unwrap();
        prepared_journal(&root, Phase::Committed);

        recover(&root).unwrap();

        assert_eq!(fs::read(root.join("users.dbf")).unwrap(), b"users-after");
        assert_eq!(fs::read(root.join("posts.dbf")).unwrap(), b"posts-after");
        assert!(!root.join(JOURNAL_DIR).exists());
        fs::remove_dir_all(root).unwrap();
    }
}
