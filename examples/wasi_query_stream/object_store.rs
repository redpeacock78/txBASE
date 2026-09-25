use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use txbase::edge::{ObjectStore, ObjectStoreError};

const LOCK_FILE: &str = ".txbase-object-store.lock";
const TEMP_PREFIX: &str = ".txbase-object-store-tmp-";

pub struct ReadOnlyFilesystemObjectStore {
    root: PathBuf,
}

impl ReadOnlyFilesystemObjectStore {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
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
        }
        Ok(path)
    }

    fn read_only_error<T>(&self) -> Result<T, ObjectStoreError> {
        Err(ObjectStoreError::Unavailable(
            "WASI query object store is read-only; pending recovery requires a writable host"
                .into(),
        ))
    }

    fn collect_keys(
        directory: &Path,
        relative: &str,
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

            let key = format!("{relative}/{name}");
            let path = entry.path();
            let file_type = entry
                .file_type()
                .map_err(|error| io_error("inspect object-store entry", &path, error))?;
            if file_type.is_dir() {
                Self::collect_keys(&path, &key, keys)?;
            } else if file_type.is_file() {
                keys.push(key);
            }
        }
        Ok(())
    }
}

impl ObjectStore for ReadOnlyFilesystemObjectStore {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>, ObjectStoreError> {
        let path = self.path_for(key)?;
        match fs::read(&path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(io_error("read object", &path, error)),
        }
    }

    fn put_if_absent(&self, _key: &str, _bytes: &[u8]) -> Result<(), ObjectStoreError> {
        self.read_only_error()
    }

    fn compare_and_swap(
        &self,
        _key: &str,
        _expected: Option<&[u8]>,
        _replacement: &[u8],
    ) -> Result<(), ObjectStoreError> {
        self.read_only_error()
    }

    fn delete(&self, _key: &str) -> Result<(), ObjectStoreError> {
        self.read_only_error()
    }

    fn list(&self, prefix: &str) -> Result<Vec<String>, ObjectStoreError> {
        let relative = prefix.strip_suffix('/').ok_or_else(|| {
            ObjectStoreError::Invalid(
                "WASI query object store requires a directory prefix ending in '/'".into(),
            )
        })?;
        let directory = self.path_for(relative)?;
        let mut keys = Vec::new();
        match fs::metadata(&directory) {
            Ok(metadata) if metadata.is_dir() => {
                Self::collect_keys(&directory, relative, &mut keys)?;
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(io_error(
                    "inspect object-store directory",
                    &directory,
                    error,
                ));
            }
        }
        keys.sort();
        Ok(keys)
    }
}

fn io_error(operation: &str, path: &Path, error: io::Error) -> ObjectStoreError {
    ObjectStoreError::Unavailable(format!("{operation} {}: {error}", path.display()))
}
