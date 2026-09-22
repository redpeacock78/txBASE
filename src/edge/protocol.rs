use super::super::store::ObjectStoreError;
use serde::{Deserialize, Serialize};

pub(super) const MANIFEST_VERSION: u16 = 1;
pub(super) const PENDING_VERSION: u16 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub version: u16,
    pub generation: u64,
    pub root: String,
    pub wal_head: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub history: Vec<u64>,
}

impl Manifest {
    pub fn to_bytes(&self) -> Result<Vec<u8>, ObjectStoreError> {
        self.validate()?;
        let mut bytes = serde_json::to_vec(self)?;
        bytes.push(b'\n');
        Ok(bytes)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ObjectStoreError> {
        let manifest: Self = serde_json::from_slice(bytes)?;
        manifest.validate()?;
        Ok(manifest)
    }

    fn validate(&self) -> Result<(), ObjectStoreError> {
        if self.version != MANIFEST_VERSION {
            return Err(ObjectStoreError::Invalid(format!(
                "unsupported manifest version {}",
                self.version
            )));
        }
        if self.root.is_empty() || self.root.contains('\0') {
            return Err(ObjectStoreError::Invalid(
                "manifest root must be a non-empty object key".into(),
            ));
        }
        if !self.history.is_empty() {
            if self.history.windows(2).any(|window| window[0] >= window[1]) {
                return Err(ObjectStoreError::Invalid(
                    "manifest history must be strictly increasing".into(),
                ));
            }
            if self.history.last().copied() != Some(self.generation) {
                return Err(ObjectStoreError::Invalid(
                    "manifest history must end at the current generation".into(),
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitResult {
    Committed { generation: u64 },
    AlreadyCommitted { generation: u64 },
}

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct PendingCommit {
    pub(super) version: u16,
    pub(super) base_generation: Option<u64>,
    pub(super) target_generation: u64,
    pub(super) root: String,
    pub(super) wal_head: u64,
}

impl PendingCommit {
    pub(super) fn to_bytes(&self) -> Result<Vec<u8>, ObjectStoreError> {
        self.validate()?;
        Ok(serde_json::to_vec(self)?)
    }

    pub(super) fn from_bytes(bytes: &[u8]) -> Result<Self, ObjectStoreError> {
        let pending: Self = serde_json::from_slice(bytes)?;
        pending.validate()?;
        Ok(pending)
    }

    fn validate(&self) -> Result<(), ObjectStoreError> {
        if self.version != PENDING_VERSION {
            return Err(ObjectStoreError::Invalid(format!(
                "unsupported pending commit version {}",
                self.version
            )));
        }
        if self
            .base_generation
            .is_some_and(|base| self.target_generation <= base)
        {
            return Err(ObjectStoreError::Invalid(
                "pending commit generation is not newer than its base".into(),
            ));
        }
        if self.root.is_empty() || self.root.contains('\0') {
            return Err(ObjectStoreError::Invalid(
                "pending commit root must be a non-empty object key".into(),
            ));
        }
        Ok(())
    }
}

pub(super) fn manifest_history(manifest: &Manifest) -> Vec<u64> {
    if manifest.history.is_empty() {
        vec![manifest.generation]
    } else {
        manifest.history.clone()
    }
}

pub(super) fn history_with_generation(
    current: Option<&Manifest>,
    generation: u64,
) -> Result<Vec<u64>, ObjectStoreError> {
    let mut history = current.map(manifest_history).unwrap_or_default();
    if history
        .last()
        .is_some_and(|previous| generation <= *previous)
    {
        return Err(ObjectStoreError::Invalid(format!(
            "XBF generation {generation} is not newer than the manifest history"
        )));
    }
    history.push(generation);
    Ok(history)
}

pub(super) fn snapshot_generation(prefix: &str, key: &str) -> Option<u64> {
    key.strip_prefix(prefix)?.strip_suffix(".xbf")?.parse().ok()
}

pub(super) fn validate_namespace(namespace: &str) -> Result<(), ObjectStoreError> {
    if namespace.is_empty()
        || namespace.contains('\0')
        || namespace
            .split('/')
            .any(|component| component.is_empty() || component == "." || component == "..")
    {
        return Err(ObjectStoreError::Invalid(
            "object-store namespace must contain ordinary non-empty key components".into(),
        ));
    }
    Ok(())
}
