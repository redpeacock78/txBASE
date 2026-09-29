use std::fs;
use std::io;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use txbase::edge::{ObjectStore, ObjectStoreError};

const LOCK_FILE: &str = ".txbase-object-store.lock";
const TEMP_PREFIX: &str = ".txbase-object-store-tmp-";
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub struct FilesystemObjectStore {
    root: PathBuf,
}

impl FilesystemObjectStore {
    pub fn new(root: PathBuf) -> Result<Self, ObjectStoreError> {
        fs::create_dir_all(&root)
            .map_err(|error| io_error("create object-store root", &root, error))?;
        Ok(Self { root })
    }

    fn path_for(&self, key: &str) -> Result<PathBuf, ObjectStoreError> {
        if key.is_empty() || key.contains('\0') {
            return Err(ObjectStoreError::Invalid(
                "object key must be non-empty and must not contain NUL".into(),
            ));
        }

        let mut path = self.root.clone();
        for component in key.split('/') {
            if component.is_empty()
                || component == "."
                || component == ".."
                || component == LOCK_FILE
                || component.starts_with(TEMP_PREFIX)
                || component.ends_with([' ', '.'])
                || component.chars().any(|character| {
                    matches!(character, '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
                })
            {
                return Err(ObjectStoreError::Invalid(format!(
                    "object key component is not portable: {component}"
                )));
            }
            path.push(component);
            match fs::symlink_metadata(&path) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err(ObjectStoreError::Invalid(
                        "object key must not traverse symbolic links".into(),
                    ));
                }
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(io_error("inspect object path", &path, error)),
            }
        }
        Ok(path)
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
            let prefix_is_below = prefix
                .strip_prefix(&key)
                .is_some_and(|remainder| remainder.starts_with('/'));
            if !(key.starts_with(prefix) || file_type.is_dir() && prefix_is_below) {
                continue;
            }
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
        match fs::read(&path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(io_error("read object", &path, error)),
        }
    }

    fn put_if_absent(&self, key: &str, bytes: &[u8]) -> Result<(), ObjectStoreError> {
        let path = self.path_for(key)?;
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
        sync_directory(path.parent().unwrap_or(&self.root))
    }

    fn compare_and_swap(
        &self,
        key: &str,
        expected: Option<&[u8]>,
        replacement: &[u8],
    ) -> Result<(), ObjectStoreError> {
        let path = self.path_for(key)?;
        if read_optional(&path)?.as_deref() != expected {
            return Err(ObjectStoreError::Conflict(format!(
                "compare-and-swap precondition failed: {key}"
            )));
        }
        replace_atomically(&path, replacement)
    }

    fn delete(&self, key: &str) -> Result<(), ObjectStoreError> {
        let path = self.path_for(key)?;
        match fs::remove_file(&path) {
            Ok(()) => sync_directory(path.parent().unwrap_or(&self.root)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(io_error("delete object", &path, error)),
        }
    }

    fn list(&self, prefix: &str) -> Result<Vec<String>, ObjectStoreError> {
        if prefix.is_empty() || prefix.contains('\0') {
            return Err(ObjectStoreError::Invalid(
                "object key must be non-empty and must not contain NUL".into(),
            ));
        }
        let mut keys = Vec::new();
        Self::collect_keys(&self.root, "", prefix, &mut keys)?;
        keys.sort();
        Ok(keys)
    }
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, ObjectStoreError> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(io_error("read object", path, error)),
    }
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), ObjectStoreError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| io_error("create object parent", parent, error))?;
    }
    let mut file = match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
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
    let temporary = parent.join(format!("{TEMP_PREFIX}{name}-{sequence}"));
    write_new(&temporary, bytes)?;
    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(io_error("replace object", path, error));
    }
    sync_directory(parent)
}

#[cfg(unix)]
fn sync_directory(directory: &Path) -> Result<(), ObjectStoreError> {
    fs::File::open(directory)
        .and_then(|file| file.sync_all())
        .map_err(|error| io_error("sync object directory", directory, error))
}

#[cfg(not(unix))]
fn sync_directory(_directory: &Path) -> Result<(), ObjectStoreError> {
    Ok(())
}

fn io_error(operation: &str, path: &Path, error: io::Error) -> ObjectStoreError {
    ObjectStoreError::Unavailable(format!("{operation} {}: {error}", path.display()))
}
