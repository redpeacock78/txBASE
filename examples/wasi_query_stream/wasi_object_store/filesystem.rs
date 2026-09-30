use txbase::edge::ObjectStoreError;
use wasip3::filesystem::preopens;
use wasip3::filesystem::types::{
    Descriptor, DescriptorFlags, DescriptorType, DirectoryEntry, ErrorCode, OpenFlags, PathFlags,
};

use crate::keys::{LOCK_FILE, TEMP_PREFIX};

pub(super) struct ParentDirectory<'a> {
    root: &'a Descriptor,
    current: Option<Descriptor>,
}

impl<'a> ParentDirectory<'a> {
    pub(super) fn new(root: &'a Descriptor) -> Self {
        Self {
            root,
            current: None,
        }
    }

    pub(super) fn descriptor(&self) -> &Descriptor {
        self.current.as_ref().unwrap_or(self.root)
    }

    pub(super) fn set_current(&mut self, directory: Descriptor) {
        self.current = Some(directory);
    }
}

pub(super) fn validate_root(root: &str) -> Result<String, ObjectStoreError> {
    if !root.starts_with('/')
        || root.contains('\0')
        || (root != "/"
            && root[1..]
                .split('/')
                .any(|component| component.is_empty() || component == "." || component == ".."))
    {
        return Err(ObjectStoreError::Invalid(
            "object-store root must be an absolute normalized WASI path".into(),
        ));
    }
    Ok(root.to_owned())
}

pub(super) fn relative_to_preopen(root: &str, preopen: &str) -> Option<String> {
    let preopen = preopen.trim_end_matches('/');
    if preopen.is_empty() {
        return root.strip_prefix('/').map(str::to_owned);
    }
    if root == preopen {
        return Some(String::new());
    }
    root.strip_prefix(preopen)
        .and_then(|suffix| suffix.strip_prefix('/'))
        .map(str::to_owned)
}

pub(super) async fn open_directory_at(
    parent: &Descriptor,
    name: &str,
    create: bool,
    mutate: bool,
) -> Result<Descriptor, ErrorCode> {
    let mut flags = DescriptorFlags::READ;
    if mutate {
        flags |= DescriptorFlags::MUTATE_DIRECTORY;
    }
    match parent
        .open_at(
            PathFlags::empty(),
            name.to_owned(),
            OpenFlags::DIRECTORY,
            flags,
        )
        .await
    {
        Ok(directory) => Ok(directory),
        Err(ErrorCode::NoEntry) if create => {
            match parent.create_directory_at(name.to_owned()).await {
                Ok(()) | Err(ErrorCode::Exist) => {}
                Err(error) => return Err(error),
            }
            parent
                .open_at(
                    PathFlags::empty(),
                    name.to_owned(),
                    OpenFlags::DIRECTORY,
                    flags,
                )
                .await
        }
        Err(error) => Err(error),
    }
}

pub(super) async fn ensure_regular_file(
    file: &Descriptor,
    key: &str,
) -> Result<(), ObjectStoreError> {
    match file
        .get_type()
        .await
        .map_err(|error| wasi_error("inspect object", key, error))?
    {
        DescriptorType::RegularFile => Ok(()),
        DescriptorType::SymbolicLink => Err(ObjectStoreError::Invalid(format!(
            "object key must not traverse symbolic links: {key}"
        ))),
        _ => Err(ObjectStoreError::Invalid(format!(
            "object key does not refer to a regular file: {key}"
        ))),
    }
}

pub(super) async fn write_stream(
    file: &Descriptor,
    bytes: Vec<u8>,
    key: &str,
) -> Result<(), ObjectStoreError> {
    let (mut writer, reader) = wasip3::wit_stream::new::<u8>();
    let write = file.write_via_stream(reader, 0);
    let producer = async move {
        let unwritten = writer.write_all(bytes).await;
        let complete = unwritten.is_empty();
        drop(writer);
        complete
    };
    let (complete, result) = futures::future::join(producer, async { write.await }).await;
    result.map_err(|error| wasi_error("write object", key, error))?;
    if !complete {
        return Err(ObjectStoreError::Unavailable(format!(
            "object-store write stream closed early: {key}"
        )));
    }
    file.sync()
        .await
        .map_err(|error| wasi_error("sync object", key, error))
}

pub(super) async fn read_directory(
    directory: &Descriptor,
) -> Result<Vec<DirectoryEntry>, ObjectStoreError> {
    let (stream, completion) = directory.read_directory();
    let (entries, result) =
        futures::future::join(stream.collect(), async { completion.await }).await;
    result.map_err(|error| wasi_error("read object directory", "", error))?;
    Ok(entries)
}

pub(super) async fn collect_entries(
    directory: &Descriptor,
    relative: &str,
    prefix: &str,
    entries: Vec<DirectoryEntry>,
    pending: &mut Vec<(Descriptor, String)>,
    keys: &mut Vec<String>,
) -> Result<(), ObjectStoreError> {
    for entry in entries {
        if entry.name == LOCK_FILE || entry.name.starts_with(TEMP_PREFIX) {
            continue;
        }
        let key = if relative.is_empty() {
            entry.name.clone()
        } else {
            format!("{relative}/{}", entry.name)
        };
        let prefix_is_below = prefix
            .strip_prefix(&key)
            .is_some_and(|remainder| remainder.starts_with('/'));
        match entry.type_ {
            DescriptorType::Directory if key.starts_with(prefix) || prefix_is_below => {
                match open_directory_at(directory, &entry.name, false, false).await {
                    Ok(child) => pending.push((child, key)),
                    Err(ErrorCode::NoEntry) => {}
                    Err(error) => {
                        return Err(wasi_error("open object directory", &key, error));
                    }
                }
            }
            DescriptorType::RegularFile if key.starts_with(prefix) => keys.push(key),
            _ => {}
        }
    }
    Ok(())
}

pub(super) fn wasi_error(operation: &str, key: &str, error: ErrorCode) -> ObjectStoreError {
    match error {
        ErrorCode::Loop | ErrorCode::NotPermitted => ObjectStoreError::Invalid(format!(
            "symbolic links and paths outside the object-store root are not allowed: {key}"
        )),
        error => ObjectStoreError::Unavailable(format!("{operation} {key}: {error:?}")),
    }
}
