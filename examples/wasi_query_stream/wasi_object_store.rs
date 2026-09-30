#[path = "wasi_object_store/filesystem.rs"]
mod filesystem;

use std::sync::atomic::{AtomicU64, Ordering};

use futures::future::join;
use txbase::edge::{AsyncObjectStore, AsyncObjectStoreFuture, ObjectStoreError};
use wasip3::filesystem::preopens;
use wasip3::filesystem::types::{Descriptor, DescriptorFlags, ErrorCode, OpenFlags, PathFlags};

use self::filesystem::{
    ParentDirectory, collect_entries, ensure_regular_file, open_directory_at, read_directory,
    relative_to_preopen, validate_root, wasi_error, write_stream,
};
use crate::keys::{TEMP_PREFIX, validate_key};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub struct WasiFilesystemObjectStore {
    root: Descriptor,
}

impl WasiFilesystemObjectStore {
    pub async fn new(root: &str) -> Result<Self, ObjectStoreError> {
        let root = validate_root(root)?;
        let mut selected: Option<(usize, Descriptor, String)> = None;
        for (directory, path) in preopens::get_directories() {
            if let Some(relative) = relative_to_preopen(&root, &path) {
                let path_length = path.trim_end_matches('/').len();
                if selected
                    .as_ref()
                    .is_none_or(|(selected_length, _, _)| path_length > *selected_length)
                {
                    selected = Some((path_length, directory, relative));
                }
            }
        }
        let Some((_, mut directory, relative)) = selected else {
            return Err(ObjectStoreError::Invalid(format!(
                "object-store root is outside every preopened directory: {root}"
            )));
        };

        for component in relative
            .split('/')
            .filter(|component| !component.is_empty())
        {
            directory = open_directory_at(&directory, component, true, true)
                .await
                .map_err(|error| wasi_error("open object-store root", component, error))?;
        }

        Ok(Self { root: directory })
    }

    async fn open_parent<'a>(
        &'a self,
        key: &str,
        create: bool,
    ) -> Result<Option<(ParentDirectory<'a>, String)>, ObjectStoreError> {
        validate_key(key)?;
        let (directories, name) = key.rsplit_once('/').unwrap_or(("", key));
        let mut parent = ParentDirectory::new(&self.root);

        for component in directories
            .split('/')
            .filter(|component| !component.is_empty())
        {
            match open_directory_at(parent.descriptor(), component, create, create).await {
                Ok(directory) => parent.set_current(directory),
                Err(ErrorCode::NoEntry) if !create => return Ok(None),
                Err(error) => return Err(wasi_error("open object directory", key, error)),
            }
        }

        Ok(Some((parent, name.to_owned())))
    }

    async fn open_file(
        &self,
        key: &str,
    ) -> Result<Option<(ParentDirectory<'_>, String, Descriptor)>, ObjectStoreError> {
        let Some((parent, name)) = self.open_parent(key, false).await? else {
            return Ok(None);
        };
        let file = match parent
            .descriptor()
            .open_at(
                PathFlags::empty(),
                name.clone(),
                OpenFlags::empty(),
                DescriptorFlags::READ,
            )
            .await
        {
            Ok(file) => file,
            Err(ErrorCode::NoEntry) => return Ok(None),
            Err(error) => return Err(wasi_error("open object", key, error)),
        };
        ensure_regular_file(&file, key).await?;
        Ok(Some((parent, name, file)))
    }

    async fn get_inner(&self, key: &str) -> Result<Option<Vec<u8>>, ObjectStoreError> {
        let Some((_, _, file)) = self.open_file(key).await? else {
            return Ok(None);
        };
        let (stream, completion) = file.read_via_stream(0);
        let (bytes, result) = join(stream.collect(), async { completion.await }).await;
        result.map_err(|error| wasi_error("read object", key, error))?;
        Ok(Some(bytes))
    }

    async fn put_inner(&self, key: &str, bytes: &[u8]) -> Result<(), ObjectStoreError> {
        if let Some(existing) = self.get_inner(key).await? {
            return if existing == bytes {
                Ok(())
            } else {
                Err(ObjectStoreError::Conflict(format!(
                    "object already exists with different bytes: {key}"
                )))
            };
        }
        let (parent, name) = self
            .open_parent(key, true)
            .await?
            .ok_or_else(|| ObjectStoreError::Unavailable("object parent is missing".into()))?;
        self.write_new(parent.descriptor(), &name, bytes, key, true)
            .await?;
        parent
            .descriptor()
            .sync()
            .await
            .map_err(|error| wasi_error("sync object directory", key, error))
    }

    async fn compare_and_swap_inner(
        &self,
        key: &str,
        expected: Option<&[u8]>,
        replacement: &[u8],
    ) -> Result<(), ObjectStoreError> {
        if self.get_inner(key).await?.as_deref() != expected {
            return Err(ObjectStoreError::Conflict(format!(
                "compare-and-swap precondition failed: {key}"
            )));
        }
        let (parent, name) = self
            .open_parent(key, true)
            .await?
            .ok_or_else(|| ObjectStoreError::Unavailable("object parent is missing".into()))?;
        let temporary = format!(
            "{TEMP_PREFIX}{name}-{}",
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        );
        self.write_new(parent.descriptor(), &temporary, replacement, key, false)
            .await?;
        if let Err(error) = parent
            .descriptor()
            .rename_at(temporary.clone(), parent.descriptor(), name.clone())
            .await
        {
            let _ = parent.descriptor().unlink_file_at(temporary).await;
            return Err(wasi_error("replace object", key, error));
        }
        parent
            .descriptor()
            .sync()
            .await
            .map_err(|error| wasi_error("sync object directory", key, error))
    }

    async fn write_new(
        &self,
        parent: &Descriptor,
        name: &str,
        bytes: &[u8],
        key: &str,
        conflict_on_exist: bool,
    ) -> Result<(), ObjectStoreError> {
        let file = match parent
            .open_at(
                PathFlags::empty(),
                name.to_owned(),
                OpenFlags::CREATE | OpenFlags::EXCLUSIVE,
                DescriptorFlags::WRITE,
            )
            .await
        {
            Ok(file) => file,
            Err(ErrorCode::Exist) if conflict_on_exist => {
                return Err(ObjectStoreError::Conflict(format!(
                    "object already exists: {key}"
                )));
            }
            Err(error) => return Err(wasi_error("create object", key, error)),
        };

        if let Err(error) = write_stream(&file, bytes.to_vec(), key).await {
            let _ = parent.unlink_file_at(name.to_owned()).await;
            return Err(error);
        }
        Ok(())
    }

    async fn delete_inner(&self, key: &str) -> Result<(), ObjectStoreError> {
        let Some((parent, name, file)) = self.open_file(key).await? else {
            return Ok(());
        };
        drop(file);
        match parent.descriptor().unlink_file_at(name).await {
            Ok(()) | Err(ErrorCode::NoEntry) => parent
                .descriptor()
                .sync()
                .await
                .map_err(|error| wasi_error("sync object directory", key, error)),
            Err(error) => Err(wasi_error("delete object", key, error)),
        }
    }

    async fn list_inner(&self, prefix: &str) -> Result<Vec<String>, ObjectStoreError> {
        if prefix.is_empty() || prefix.contains('\0') {
            return Err(ObjectStoreError::Invalid(
                "object key must be non-empty and must not contain NUL".into(),
            ));
        }
        let mut keys = Vec::new();
        let mut pending = Vec::new();
        let entries = read_directory(&self.root).await?;
        collect_entries(&self.root, "", prefix, entries, &mut pending, &mut keys).await?;

        while let Some((directory, relative)) = pending.pop() {
            let entries = read_directory(&directory).await?;
            collect_entries(
                &directory,
                &relative,
                prefix,
                entries,
                &mut pending,
                &mut keys,
            )
            .await?;
        }

        keys.sort();
        Ok(keys)
    }
}

impl AsyncObjectStore for WasiFilesystemObjectStore {
    fn get<'a>(
        &'a self,
        key: &'a str,
    ) -> AsyncObjectStoreFuture<'a, Result<Option<Vec<u8>>, ObjectStoreError>> {
        Box::pin(async move { self.get_inner(key).await })
    }

    fn put_if_absent<'a>(
        &'a self,
        key: &'a str,
        bytes: &'a [u8],
    ) -> AsyncObjectStoreFuture<'a, Result<(), ObjectStoreError>> {
        Box::pin(async move { self.put_inner(key, bytes).await })
    }

    fn compare_and_swap<'a>(
        &'a self,
        key: &'a str,
        expected: Option<&'a [u8]>,
        replacement: &'a [u8],
    ) -> AsyncObjectStoreFuture<'a, Result<(), ObjectStoreError>> {
        Box::pin(async move {
            self.compare_and_swap_inner(key, expected, replacement)
                .await
        })
    }

    fn delete<'a>(
        &'a self,
        key: &'a str,
    ) -> AsyncObjectStoreFuture<'a, Result<(), ObjectStoreError>> {
        Box::pin(async move { self.delete_inner(key).await })
    }

    fn list<'a>(
        &'a self,
        prefix: &'a str,
    ) -> AsyncObjectStoreFuture<'a, Result<Vec<String>, ObjectStoreError>> {
        Box::pin(async move { self.list_inner(prefix).await })
    }
}
