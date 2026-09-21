use super::store::{ObjectStore, ObjectStoreError, validate_key};
use fs2::FileExt;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const LOCK_FILE: &str = ".txbase-object-store.lock";
const TEMP_PREFIX: &str = ".txbase-object-store-tmp-";
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone)]
pub struct FilesystemObjectStore {
    root: PathBuf,
}

impl FilesystemObjectStore {
    pub fn new(root: impl AsRef<Path>) -> Result<Self, ObjectStoreError> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(&root)
            .map_err(|error| io_error("create object-store root", &root, error))?;
        Ok(Self { root })
    }

    fn path_for(&self, key: &str) -> Result<PathBuf, ObjectStoreError> {
        validate_key(key)?;
        let mut path = self.root.clone();
        for component in key.split('/') {
            validate_component(component)?;
            path.push(component);
        }
        Ok(path)
    }

    fn with_lock<T>(
        &self,
        exclusive: bool,
        operation: impl FnOnce() -> Result<T, ObjectStoreError>,
    ) -> Result<T, ObjectStoreError> {
        let lock_path = self.root.join(LOCK_FILE);
        let lock_file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&lock_path)
            .map_err(|error| io_error("open object-store lock", &lock_path, error))?;
        let lock_result = if exclusive {
            FileExt::lock_exclusive(&lock_file)
        } else {
            FileExt::lock_shared(&lock_file)
        };
        lock_result.map_err(|error| io_error("lock object store", &lock_path, error))?;

        // ponytail: one store-wide lock keeps CAS portable; shard locks only if measured throughput requires it.
        let result = operation();
        let unlock_result = FileExt::unlock(&lock_file);
        match (result, unlock_result) {
            (Ok(value), Ok(())) => Ok(value),
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(io_error("unlock object store", &lock_path, error)),
        }
    }

    fn collect_keys(
        directory: &Path,
        relative: &str,
        prefix: &str,
        keys: &mut Vec<String>,
    ) -> Result<(), ObjectStoreError> {
        let entries = fs::read_dir(directory)
            .map_err(|error| io_error("list object-store directory", directory, error))?;
        for entry in entries {
            let entry =
                entry.map_err(|error| io_error("read object-store entry", directory, error))?;
            let name = entry.file_name().into_string().map_err(|_| {
                ObjectStoreError::Invalid(format!(
                    "object-store path is not valid UTF-8: {}",
                    entry.path().display()
                ))
            })?;
            if name == LOCK_FILE || name.starts_with(TEMP_PREFIX) {
                continue;
            }
            let key = if relative.is_empty() {
                name
            } else {
                format!("{relative}/{name}")
            };
            let path = entry.path();
            let file_type = entry
                .file_type()
                .map_err(|error| io_error("inspect object-store entry", &path, error))?;
            if file_type.is_dir() {
                Self::collect_keys(&path, &key, prefix, keys)?;
            } else if file_type.is_file() && key.starts_with(prefix) {
                keys.push(key);
            }
        }
        Ok(())
    }
}

impl ObjectStore for FilesystemObjectStore {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>, ObjectStoreError> {
        let path = self.path_for(key)?;
        self.with_lock(false, || match fs::read(&path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(io_error("read object", &path, error)),
        })
    }

    fn put_if_absent(&self, key: &str, bytes: &[u8]) -> Result<(), ObjectStoreError> {
        let path = self.path_for(key)?;
        self.with_lock(true, || {
            if let Some(existing) = read_optional(&path)? {
                return if existing == bytes {
                    Ok(())
                } else {
                    Err(ObjectStoreError::Conflict(format!(
                        "object already exists with different bytes: {key}"
                    )))
                };
            }
            write_new(&path, bytes)?;
            sync_directory(path.parent().unwrap_or(&self.root))?;
            Ok(())
        })
    }

    fn compare_and_swap(
        &self,
        key: &str,
        expected: Option<&[u8]>,
        replacement: &[u8],
    ) -> Result<(), ObjectStoreError> {
        let path = self.path_for(key)?;
        self.with_lock(true, || {
            if read_optional(&path)?.as_deref() != expected {
                return Err(ObjectStoreError::Conflict(format!(
                    "compare-and-swap precondition failed: {key}"
                )));
            }
            replace_atomically(&path, replacement)?;
            Ok(())
        })
    }

    fn delete(&self, key: &str) -> Result<(), ObjectStoreError> {
        let path = self.path_for(key)?;
        self.with_lock(true, || match fs::remove_file(&path) {
            Ok(()) => {
                sync_directory(path.parent().unwrap_or(&self.root))?;
                Ok(())
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(io_error("delete object", &path, error)),
        })
    }

    fn list(&self, prefix: &str) -> Result<Vec<String>, ObjectStoreError> {
        validate_key(prefix)?;
        self.with_lock(false, || {
            let mut keys = Vec::new();
            Self::collect_keys(&self.root, "", prefix, &mut keys)?;
            keys.sort();
            Ok(keys)
        })
    }
}

fn validate_component(component: &str) -> Result<(), ObjectStoreError> {
    if component.is_empty()
        || component == "."
        || component == ".."
        || component == LOCK_FILE
        || component.starts_with(TEMP_PREFIX)
        || component.ends_with([' ', '.'])
        || component
            .chars()
            .any(|character| matches!(character, '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'))
    {
        return Err(ObjectStoreError::Invalid(format!(
            "object key component is not portable: {component}"
        )));
    }
    Ok(())
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, ObjectStoreError> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(io_error("read object", path, error)),
    }
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), ObjectStoreError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| io_error("create object parent", parent, error))?;
    }
    let mut file = match OpenOptions::new().create_new(true).write(true).open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err(ObjectStoreError::Conflict(format!(
                "object already exists: {}",
                path.display()
            )));
        }
        Err(error) => return Err(io_error("create object", path, error)),
    };
    if let Err(error) = file.write_all(bytes).and_then(|()| file.sync_all()) {
        let _ = fs::remove_file(path);
        return Err(io_error("write object", path, error));
    }
    Ok(())
}

fn replace_atomically(path: &Path, bytes: &[u8]) -> Result<(), ObjectStoreError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|error| io_error("create object parent", parent, error))?;
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("object");
    let temporary = parent.join(format!(
        "{TEMP_PREFIX}{name}-{}-{sequence}",
        std::process::id()
    ));
    write_new(&temporary, bytes)?;
    let rename_result = replace_path(&temporary, path);
    if let Err(error) = rename_result {
        let _ = fs::remove_file(&temporary);
        return Err(io_error("replace object", path, error));
    }
    sync_directory(parent)
}

#[cfg(not(windows))]
fn replace_path(temporary: &Path, target: &Path) -> std::io::Result<()> {
    fs::rename(temporary, target)
}

#[cfg(windows)]
fn replace_path(temporary: &Path, target: &Path) -> std::io::Result<()> {
    match fs::rename(temporary, target) {
        Ok(()) => Ok(()),
        Err(error) if target.exists() => {
            fs::remove_file(target)?;
            fs::rename(temporary, target).map_err(|_| error)
        }
        Err(error) => Err(error),
    }
}

#[cfg(unix)]
fn sync_directory(directory: &Path) -> Result<(), ObjectStoreError> {
    File::open(directory)
        .and_then(|file| file.sync_all())
        .map_err(|error| io_error("sync object directory", directory, error))
}

#[cfg(not(unix))]
fn sync_directory(_directory: &Path) -> Result<(), ObjectStoreError> {
    Ok(())
}

fn io_error(operation: &str, path: &Path, error: std::io::Error) -> ObjectStoreError {
    ObjectStoreError::Unavailable(format!("{operation} {}: {error}", path.display()))
}
