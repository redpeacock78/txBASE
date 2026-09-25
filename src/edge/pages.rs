use super::super::store::{
    AsyncObjectStore, ObjectStore, ObjectStoreError, put_if_absent_or_matching,
    put_if_absent_or_matching_async,
};
pub(super) use super::page_manifest::PageManifest;
use super::page_manifest::{PAGE_MANIFEST_VERSION, PAGE_SIZE, PageReference};
pub(super) use super::page_manifest::{
    legacy_snapshot_key, page_manifest_key, root_generation, validate_snapshot_root,
};
use crate::xbf::crc32c;

pub(super) fn page_key(prefix: &str, generation: u64, index: usize) -> String {
    format!("{prefix}pages/{generation}/{index}.bin")
}

pub(super) fn make_manifest<S: ObjectStore>(
    store: &S,
    prefix: &str,
    generation: u64,
    previous_root: Option<&str>,
    snapshot: &[u8],
) -> Result<PageManifest, ObjectStoreError> {
    let previous = load_previous_manifest(store, prefix, previous_root)?;
    let pages = snapshot
        .chunks(PAGE_SIZE)
        .enumerate()
        .map(|(index, bytes)| {
            let reused_generation = match previous
                .as_ref()
                .and_then(|manifest| manifest.pages.get(index))
            {
                Some(page) if page.length as usize == bytes.len() => {
                    let old = store
                        .get(&page_key(prefix, page.generation, index))?
                        .ok_or_else(|| {
                            ObjectStoreError::Missing(page_key(prefix, page.generation, index))
                        })?;
                    (old == bytes).then_some(page.generation)
                }
                _ => None,
            };
            Ok(PageReference {
                generation: reused_generation.unwrap_or(generation),
                length: bytes.len() as u32,
                crc32c: crc32c(bytes),
            })
        })
        .collect::<Result<Vec<_>, ObjectStoreError>>()?;
    Ok(PageManifest {
        version: PAGE_MANIFEST_VERSION,
        generation,
        snapshot_length: snapshot.len() as u64,
        page_size: PAGE_SIZE as u32,
        pages,
    })
}

pub(super) async fn make_manifest_async<S: AsyncObjectStore>(
    store: &S,
    prefix: &str,
    generation: u64,
    previous_root: Option<&str>,
    snapshot: &[u8],
) -> Result<PageManifest, ObjectStoreError> {
    let previous = load_previous_manifest_async(store, prefix, previous_root).await?;
    let mut pages = Vec::new();
    for (index, bytes) in snapshot.chunks(PAGE_SIZE).enumerate() {
        let reused_generation = match previous
            .as_ref()
            .and_then(|manifest| manifest.pages.get(index))
        {
            Some(page) if page.length as usize == bytes.len() => {
                let key = page_key(prefix, page.generation, index);
                let old = store
                    .get(&key)
                    .await?
                    .ok_or_else(|| ObjectStoreError::Missing(key.clone()))?;
                (old == bytes).then_some(page.generation)
            }
            _ => None,
        };
        pages.push(PageReference {
            generation: reused_generation.unwrap_or(generation),
            length: bytes.len() as u32,
            crc32c: crc32c(bytes),
        });
    }
    Ok(PageManifest {
        version: PAGE_MANIFEST_VERSION,
        generation,
        snapshot_length: snapshot.len() as u64,
        page_size: PAGE_SIZE as u32,
        pages,
    })
}

fn load_previous_manifest<S: ObjectStore>(
    store: &S,
    prefix: &str,
    root: Option<&str>,
) -> Result<Option<PageManifest>, ObjectStoreError> {
    let Some(root) = root.filter(|root| root.ends_with(".pages.json")) else {
        return Ok(None);
    };
    let generation = root_generation(prefix, root)
        .ok_or_else(|| ObjectStoreError::Invalid(format!("invalid page-manifest root: {root}")))?;
    let bytes = store
        .get(root)?
        .ok_or_else(|| ObjectStoreError::Missing(root.into()))?;
    PageManifest::from_bytes(&bytes, generation, usize::MAX).map(Some)
}

async fn load_previous_manifest_async<S: AsyncObjectStore>(
    store: &S,
    prefix: &str,
    root: Option<&str>,
) -> Result<Option<PageManifest>, ObjectStoreError> {
    let Some(root) = root.filter(|root| root.ends_with(".pages.json")) else {
        return Ok(None);
    };
    let generation = root_generation(prefix, root)
        .ok_or_else(|| ObjectStoreError::Invalid(format!("invalid page-manifest root: {root}")))?;
    let bytes = store
        .get(root)
        .await?
        .ok_or_else(|| ObjectStoreError::Missing(root.into()))?;
    PageManifest::from_bytes(&bytes, generation, usize::MAX).map(Some)
}

pub(super) fn publish_pages<S: ObjectStore>(
    store: &S,
    prefix: &str,
    manifest: &PageManifest,
    snapshot: &[u8],
) -> Result<(), ObjectStoreError> {
    for (index, (page, bytes)) in manifest
        .pages
        .iter()
        .zip(snapshot.chunks(PAGE_SIZE))
        .enumerate()
    {
        if page.generation == manifest.generation {
            put_if_absent_or_matching(store, &page_key(prefix, page.generation, index), bytes)?;
        }
    }
    put_if_absent_or_matching(
        store,
        &page_manifest_key(prefix, manifest.generation),
        &manifest.to_bytes()?,
    )
}

pub(super) async fn publish_pages_async<S: AsyncObjectStore>(
    store: &S,
    prefix: &str,
    manifest: &PageManifest,
    snapshot: &[u8],
) -> Result<(), ObjectStoreError> {
    for (index, (page, bytes)) in manifest
        .pages
        .iter()
        .zip(snapshot.chunks(PAGE_SIZE))
        .enumerate()
    {
        if page.generation == manifest.generation {
            put_if_absent_or_matching_async(
                store,
                &page_key(prefix, page.generation, index),
                bytes,
            )
            .await?;
        }
    }
    put_if_absent_or_matching_async(
        store,
        &page_manifest_key(prefix, manifest.generation),
        &manifest.to_bytes()?,
    )
    .await
}

pub(super) fn load_manifest<S: ObjectStore>(
    store: &S,
    prefix: &str,
    generation: u64,
    max_file_size: usize,
) -> Result<Option<PageManifest>, ObjectStoreError> {
    let key = page_manifest_key(prefix, generation);
    let Some(bytes) = store.get(&key)? else {
        return Ok(None);
    };
    PageManifest::from_bytes(&bytes, generation, max_file_size).map(Some)
}

pub(super) async fn load_manifest_async<S: AsyncObjectStore>(
    store: &S,
    prefix: &str,
    generation: u64,
    max_file_size: usize,
) -> Result<Option<PageManifest>, ObjectStoreError> {
    let key = page_manifest_key(prefix, generation);
    let Some(bytes) = store.get(&key).await? else {
        return Ok(None);
    };
    PageManifest::from_bytes(&bytes, generation, max_file_size).map(Some)
}

pub(super) fn read_snapshot_bytes<S: ObjectStore>(
    store: &S,
    prefix: &str,
    root: &str,
    generation: u64,
    max_file_size: usize,
    allow_missing: bool,
) -> Result<Option<Vec<u8>>, ObjectStoreError> {
    validate_snapshot_root(prefix, root, generation)?;
    if root.ends_with(".xbf") {
        let Some(bytes) = store.get(root)? else {
            return missing_snapshot(root, allow_missing);
        };
        if bytes.len() > max_file_size {
            return Err(ObjectStoreError::Invalid(
                "snapshot exceeds the configured file-size limit".into(),
            ));
        }
        return Ok(Some(bytes));
    }

    let Some(bytes) = store.get(root)? else {
        return missing_snapshot(root, allow_missing);
    };
    let manifest = PageManifest::from_bytes(&bytes, generation, max_file_size)?;
    let mut snapshot = Vec::with_capacity(
        usize::try_from(manifest.snapshot_length)
            .map_err(|_| ObjectStoreError::Invalid("snapshot length overflows usize".into()))?,
    );
    for index in 0..manifest.pages.len() {
        let page = match read_page(store, prefix, &manifest, index) {
            Ok(Some(page)) => page,
            Ok(None) => unreachable!("page index comes from the manifest"),
            Err(ObjectStoreError::Missing(_)) if allow_missing => return Ok(None),
            Err(error) => return Err(error),
        };
        snapshot.extend_from_slice(&page);
    }
    if snapshot.len() as u64 != manifest.snapshot_length {
        return Err(ObjectStoreError::Invalid(
            "reassembled snapshot length does not match its page manifest".into(),
        ));
    }
    Ok(Some(snapshot))
}

pub(super) async fn read_snapshot_bytes_async<S: AsyncObjectStore>(
    store: &S,
    prefix: &str,
    root: &str,
    generation: u64,
    max_file_size: usize,
    allow_missing: bool,
) -> Result<Option<Vec<u8>>, ObjectStoreError> {
    validate_snapshot_root(prefix, root, generation)?;
    if root.ends_with(".xbf") {
        let Some(bytes) = store.get(root).await? else {
            return missing_snapshot(root, allow_missing);
        };
        if bytes.len() > max_file_size {
            return Err(ObjectStoreError::Invalid(
                "snapshot exceeds the configured file-size limit".into(),
            ));
        }
        return Ok(Some(bytes));
    }

    let Some(bytes) = store.get(root).await? else {
        return missing_snapshot(root, allow_missing);
    };
    let manifest = PageManifest::from_bytes(&bytes, generation, max_file_size)?;
    let mut snapshot = Vec::with_capacity(
        usize::try_from(manifest.snapshot_length)
            .map_err(|_| ObjectStoreError::Invalid("snapshot length overflows usize".into()))?,
    );
    for index in 0..manifest.pages.len() {
        let page = match read_page_async(store, prefix, &manifest, index).await {
            Ok(Some(page)) => page,
            Ok(None) => unreachable!("page index comes from the manifest"),
            Err(ObjectStoreError::Missing(_)) if allow_missing => return Ok(None),
            Err(error) => return Err(error),
        };
        snapshot.extend_from_slice(&page);
    }
    if snapshot.len() as u64 != manifest.snapshot_length {
        return Err(ObjectStoreError::Invalid(
            "reassembled snapshot length does not match its page manifest".into(),
        ));
    }
    Ok(Some(snapshot))
}

fn missing_snapshot<T>(root: &str, allow_missing: bool) -> Result<Option<T>, ObjectStoreError> {
    if allow_missing {
        Ok(None)
    } else {
        Err(ObjectStoreError::Missing(root.into()))
    }
}

pub(super) fn read_page<S: ObjectStore>(
    store: &S,
    prefix: &str,
    manifest: &PageManifest,
    index: usize,
) -> Result<Option<Vec<u8>>, ObjectStoreError> {
    let Some(page) = manifest.pages.get(index) else {
        return Ok(None);
    };
    let key = page_key(prefix, page.generation, index);
    let Some(bytes) = store.get(&key)? else {
        return Err(ObjectStoreError::Missing(key));
    };
    validate_page_bytes(page, index, &bytes)?;
    Ok(Some(bytes))
}

pub(super) async fn read_page_async<S: AsyncObjectStore>(
    store: &S,
    prefix: &str,
    manifest: &PageManifest,
    index: usize,
) -> Result<Option<Vec<u8>>, ObjectStoreError> {
    let Some(page) = manifest.pages.get(index) else {
        return Ok(None);
    };
    let key = page_key(prefix, page.generation, index);
    let Some(bytes) = store.get(&key).await? else {
        return Err(ObjectStoreError::Missing(key));
    };
    validate_page_bytes(page, index, &bytes)?;
    Ok(Some(bytes))
}

fn validate_page_bytes(
    page: &PageReference,
    index: usize,
    bytes: &[u8],
) -> Result<(), ObjectStoreError> {
    if bytes.len() != page.length as usize || crc32c(bytes) != page.crc32c {
        return Err(ObjectStoreError::Invalid(format!(
            "page {index} does not match its manifest length or CRC-32C"
        )));
    }
    Ok(())
}

pub(super) fn page_keys(prefix: &str, manifest: &PageManifest) -> Vec<String> {
    manifest
        .pages
        .iter()
        .enumerate()
        .map(|(index, page)| page_key(prefix, page.generation, index))
        .collect()
}
