use super::super::store::ObjectStoreError;
use serde::{Deserialize, Serialize};

pub(super) const PAGE_SIZE: usize = 4 * 1024 * 1024;
pub(super) const PAGE_MANIFEST_VERSION: u16 = 1;
const PAGE_MANIFEST_BYTES_LIMIT: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageManifest {
    /// Version of the immutable page-manifest format.
    pub version: u16,
    /// Logical XBF generation represented by this manifest.
    pub generation: u64,
    /// Encoded XBF snapshot length before page splitting.
    pub snapshot_length: u64,
    /// Byte length of each page except the final page.
    pub page_size: u32,
    /// Ordered page references; the vector index is the page index.
    pub pages: Vec<PageReference>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageReference {
    /// Generation whose immutable object stores these page bytes.
    pub generation: u64,
    /// Encoded byte length of this page.
    pub length: u32,
    /// CRC-32C of this page's bytes.
    pub crc32c: u32,
}

impl PageManifest {
    pub(super) fn to_bytes(&self) -> Result<Vec<u8>, ObjectStoreError> {
        self.validate(self.generation, u64::MAX)?;
        let bytes = serde_json::to_vec(self)?;
        if bytes.len() > PAGE_MANIFEST_BYTES_LIMIT {
            return Err(ObjectStoreError::Invalid(
                "page manifest exceeds the configured metadata limit".into(),
            ));
        }
        Ok(bytes)
    }

    pub(super) fn from_bytes(
        bytes: &[u8],
        generation: u64,
        max_file_size: usize,
    ) -> Result<Self, ObjectStoreError> {
        if bytes.len() > PAGE_MANIFEST_BYTES_LIMIT {
            return Err(ObjectStoreError::Invalid(
                "page manifest exceeds the configured metadata limit".into(),
            ));
        }
        let manifest: Self = serde_json::from_slice(bytes)?;
        manifest.validate(generation, max_file_size as u64)?;
        Ok(manifest)
    }

    fn validate(&self, generation: u64, max_file_size: u64) -> Result<(), ObjectStoreError> {
        if self.version != PAGE_MANIFEST_VERSION {
            return Err(ObjectStoreError::Invalid(format!(
                "unsupported page-manifest version {}",
                self.version
            )));
        }
        if self.generation != generation {
            return Err(ObjectStoreError::Invalid(format!(
                "page-manifest generation {} does not match root generation {generation}",
                self.generation
            )));
        }
        if self.page_size as usize != PAGE_SIZE {
            return Err(ObjectStoreError::Invalid(format!(
                "unsupported page size {}",
                self.page_size
            )));
        }
        if self.snapshot_length == 0 || self.snapshot_length > max_file_size {
            return Err(ObjectStoreError::Invalid(
                "page-manifest snapshot length is outside the configured limit".into(),
            ));
        }
        let page_size = PAGE_SIZE as u64;
        let expected_page_count =
            self.snapshot_length / page_size + u64::from(self.snapshot_length % page_size != 0);
        if usize::try_from(expected_page_count).ok() != Some(self.pages.len()) {
            return Err(ObjectStoreError::Invalid(
                "page-manifest page count does not match snapshot length".into(),
            ));
        }
        for (index, page) in self.pages.iter().enumerate() {
            let expected_length = if index + 1 == self.pages.len() {
                self.snapshot_length - page_size * index as u64
            } else {
                page_size
            };
            if u64::from(page.length) != expected_length || page.generation > generation {
                return Err(ObjectStoreError::Invalid(format!(
                    "page-manifest entry {index} has an invalid length or generation"
                )));
            }
        }
        Ok(())
    }
}

pub(super) fn legacy_snapshot_key(prefix: &str, generation: u64) -> String {
    format!("{prefix}snapshots/{generation}.xbf")
}

pub(super) fn page_manifest_key(prefix: &str, generation: u64) -> String {
    format!("{prefix}snapshots/{generation}.pages.json")
}

pub(super) fn validate_snapshot_root(
    prefix: &str,
    root: &str,
    generation: u64,
) -> Result<(), ObjectStoreError> {
    if root != legacy_snapshot_key(prefix, generation)
        && root != page_manifest_key(prefix, generation)
    {
        return Err(ObjectStoreError::Invalid(format!(
            "snapshot root does not match generation {generation}: {root}"
        )));
    }
    Ok(())
}

pub(super) fn root_generation(prefix: &str, key: &str) -> Option<u64> {
    let key = key.strip_prefix(&format!("{prefix}snapshots/"))?;
    key.strip_suffix(".xbf")
        .or_else(|| key.strip_suffix(".pages.json"))?
        .parse()
        .ok()
}
