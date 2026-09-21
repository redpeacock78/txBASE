use super::store::{ObjectStore, ObjectStoreError};
use crate::xbf::{XbfLimits, XbfTable, decode_with_limits, encode};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

const MANIFEST_VERSION: u16 = 1;
const PENDING_VERSION: u16 = 1;

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
struct PendingCommit {
    version: u16,
    base_generation: Option<u64>,
    target_generation: u64,
    root: String,
    wal_head: u64,
}

impl PendingCommit {
    fn to_bytes(&self) -> Result<Vec<u8>, ObjectStoreError> {
        self.validate()?;
        Ok(serde_json::to_vec(self)?)
    }

    fn from_bytes(bytes: &[u8]) -> Result<Self, ObjectStoreError> {
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

pub struct ObjectTable<S> {
    store: S,
    prefix: String,
    manifest_key: String,
    limits: XbfLimits,
}

impl<S: ObjectStore> ObjectTable<S> {
    pub fn new(store: S, namespace: impl Into<String>) -> Result<Self, ObjectStoreError> {
        let namespace = namespace.into();
        validate_namespace(&namespace)?;
        let prefix = format!("{namespace}/");
        Ok(Self {
            store,
            manifest_key: format!("{prefix}manifest.json"),
            prefix,
            limits: XbfLimits::default(),
        })
    }

    pub fn with_limits(mut self, limits: XbfLimits) -> Self {
        self.limits = limits;
        self
    }

    pub fn manifest_key(&self) -> &str {
        &self.manifest_key
    }

    pub fn manifest(&self) -> Result<Option<Manifest>, ObjectStoreError> {
        let (_, manifest) = self.current_manifest()?;
        if let Some(manifest) = &manifest {
            self.validate_manifest_root(manifest)?;
        }
        Ok(manifest)
    }

    pub fn read(&self) -> Result<Option<XbfTable>, ObjectStoreError> {
        self.recover()?;
        let (_, manifest) = self.current_manifest()?;
        let Some(manifest) = manifest else {
            return Ok(None);
        };
        let snapshot = self.read_snapshot(&manifest.root, manifest.generation)?;
        Ok(Some(snapshot))
    }

    pub fn read_at(&self, generation: u64) -> Result<Option<XbfTable>, ObjectStoreError> {
        self.recover()?;
        let (_, manifest) = self.current_manifest()?;
        let Some(manifest) = manifest else {
            return Ok(None);
        };
        self.validate_manifest_root(&manifest)?;
        if !manifest_history(&manifest).contains(&generation) {
            return Ok(None);
        }
        let root = self.snapshot_key(generation);
        if self.store.get(&root)?.is_none() {
            return Ok(None);
        }
        Ok(Some(self.read_snapshot(&root, generation)?))
    }

    pub fn retain_generations(&self, keep_last: usize) -> Result<Vec<String>, ObjectStoreError> {
        if keep_last == 0 {
            return Err(ObjectStoreError::Invalid(
                "object-store retention count must be positive".into(),
            ));
        }
        self.recover()?;
        let (_, manifest) = self.current_manifest()?;
        let Some(manifest) = manifest else {
            return Ok(Vec::new());
        };
        self.validate_manifest_root(&manifest)?;
        let retained = manifest_history(&manifest)
            .into_iter()
            .rev()
            .take(keep_last)
            .collect::<BTreeSet<_>>();
        let prefix = self.snapshot_prefix();
        let mut removed = Vec::new();
        for key in self.store.list(&prefix)? {
            let keep = snapshot_generation(&prefix, &key)
                .is_some_and(|generation| retained.contains(&generation));
            if !keep {
                self.store.delete(&key)?;
                removed.push(key);
            }
        }
        removed.sort();
        Ok(removed)
    }

    pub fn commit(&self, table: &XbfTable) -> Result<CommitResult, ObjectStoreError> {
        self.recover()?;
        let snapshot = encode(table)?;
        let (expected_bytes, current) = self.current_manifest()?;
        if let Some(manifest) = &current {
            self.validate_manifest_root(manifest)?;
            if table.generation < manifest.generation {
                return Err(ObjectStoreError::Invalid(format!(
                    "XBF generation {} is older than the manifest generation {}",
                    table.generation, manifest.generation
                )));
            }
            if table.generation == manifest.generation {
                let current_snapshot = self.read_snapshot_bytes(&manifest.root)?;
                if current_snapshot == snapshot {
                    return Ok(CommitResult::AlreadyCommitted {
                        generation: table.generation,
                    });
                }
                return Err(ObjectStoreError::Conflict(format!(
                    "generation {} is already committed with different bytes",
                    table.generation
                )));
            }
        }

        let root = self.snapshot_key(table.generation);
        self.store.put_if_absent(&root, &snapshot)?;
        let pending = PendingCommit {
            version: PENDING_VERSION,
            base_generation: current.as_ref().map(|manifest| manifest.generation),
            target_generation: table.generation,
            root: root.clone(),
            wal_head: table.generation,
        };
        let wal_key = self.wal_key(table.generation);
        self.store.put_if_absent(&wal_key, &pending.to_bytes()?)?;
        let manifest = Manifest {
            version: MANIFEST_VERSION,
            generation: table.generation,
            root,
            wal_head: table.generation,
            history: history_with_generation(current.as_ref(), table.generation)?,
        };
        let manifest_bytes = manifest.to_bytes()?;
        match self.store.compare_and_swap(
            &self.manifest_key,
            expected_bytes.as_deref(),
            &manifest_bytes,
        ) {
            Ok(()) => {
                self.store.delete(&wal_key)?;
                Ok(CommitResult::Committed {
                    generation: table.generation,
                })
            }
            Err(error) => {
                if matches!(&error, ObjectStoreError::Conflict(_)) {
                    let _ = self.store.delete(&wal_key);
                }
                Err(error)
            }
        }
    }

    pub fn recover(&self) -> Result<usize, ObjectStoreError> {
        let mut pending_commits = Vec::new();
        for wal_key in self.store.list(&self.wal_prefix())? {
            let Some(bytes) = self.store.get(&wal_key)? else {
                continue;
            };
            let pending = PendingCommit::from_bytes(&bytes)?;
            self.validate_pending(&wal_key, &pending)?;
            self.read_snapshot(&pending.root, pending.target_generation)?;
            pending_commits.push((wal_key, pending));
        }
        pending_commits.sort_by_key(|(_, pending)| pending.target_generation);

        let mut recovered = 0;
        for (wal_key, pending) in pending_commits {
            let (expected_bytes, current) = self.current_manifest()?;
            if let Some(manifest) = &current {
                self.validate_manifest_root(manifest)?;
            }
            if current.as_ref().is_some_and(|manifest| {
                manifest.generation == pending.target_generation && manifest.root == pending.root
            }) {
                self.store.delete(&wal_key)?;
                recovered += 1;
                continue;
            }
            let current_generation = current.as_ref().map(|manifest| manifest.generation);
            if current_generation != pending.base_generation {
                match (current_generation, pending.base_generation) {
                    (Some(current), Some(base)) if current > base => {
                        self.store.delete(&wal_key)?;
                        continue;
                    }
                    (Some(_), None) => {
                        self.store.delete(&wal_key)?;
                        continue;
                    }
                    _ => {
                        return Err(ObjectStoreError::Conflict(format!(
                            "pending generation {} no longer follows the current manifest",
                            pending.target_generation
                        )));
                    }
                }
            }
            let manifest = Manifest {
                version: MANIFEST_VERSION,
                generation: pending.target_generation,
                root: pending.root,
                wal_head: pending.wal_head,
                history: history_with_generation(current.as_ref(), pending.target_generation)?,
            };
            self.store.compare_and_swap(
                &self.manifest_key,
                expected_bytes.as_deref(),
                &manifest.to_bytes()?,
            )?;
            self.store.delete(&wal_key)?;
            recovered += 1;
        }
        Ok(recovered)
    }

    pub fn cleanup_orphans(&self) -> Result<Vec<String>, ObjectStoreError> {
        let (_, current) = self.current_manifest()?;
        if let Some(manifest) = &current {
            self.validate_manifest_root(manifest)?;
        }
        let current_generation = current.as_ref().map(|manifest| manifest.generation);
        let current_root = current.as_ref().map(|manifest| manifest.root.as_str());
        let mut retained_roots = BTreeSet::new();
        if let Some(manifest) = &current {
            for generation in manifest_history(manifest) {
                retained_roots.insert(self.snapshot_key(generation));
            }
        } else if let Some(root) = current_root {
            retained_roots.insert(root.to_owned());
        }
        let mut removed = Vec::new();
        for wal_key in self.store.list(&self.wal_prefix())? {
            let Some(bytes) = self.store.get(&wal_key)? else {
                continue;
            };
            let pending = PendingCommit::from_bytes(&bytes)?;
            self.validate_pending(&wal_key, &pending)?;
            if current_generation == pending.base_generation {
                retained_roots.insert(pending.root);
                continue;
            }
            self.store.delete(&wal_key)?;
            removed.push(wal_key);
        }
        for snapshot_key in self.store.list(&self.snapshot_prefix())? {
            if retained_roots.contains(&snapshot_key) {
                continue;
            }
            self.store.delete(&snapshot_key)?;
            removed.push(snapshot_key);
        }
        removed.sort();
        Ok(removed)
    }

    fn current_manifest(&self) -> Result<(Option<Vec<u8>>, Option<Manifest>), ObjectStoreError> {
        let bytes = self.store.get(&self.manifest_key)?;
        let manifest = bytes.as_deref().map(Manifest::from_bytes).transpose()?;
        Ok((bytes, manifest))
    }

    fn read_snapshot(&self, root: &str, generation: u64) -> Result<XbfTable, ObjectStoreError> {
        self.validate_snapshot_root(root, generation)?;
        let table = decode_with_limits(&self.read_snapshot_bytes(root)?, &self.limits)?;
        if table.generation != generation {
            return Err(ObjectStoreError::Invalid(format!(
                "snapshot generation {} does not match manifest generation {generation}",
                table.generation
            )));
        }
        Ok(table)
    }

    fn read_snapshot_bytes(&self, root: &str) -> Result<Vec<u8>, ObjectStoreError> {
        self.store
            .get(root)?
            .ok_or_else(|| ObjectStoreError::Missing(root.into()))
    }

    fn validate_manifest_root(&self, manifest: &Manifest) -> Result<(), ObjectStoreError> {
        self.validate_snapshot_root(&manifest.root, manifest.generation)
    }

    fn validate_pending(
        &self,
        wal_key: &str,
        pending: &PendingCommit,
    ) -> Result<(), ObjectStoreError> {
        if wal_key != self.wal_key(pending.target_generation) {
            return Err(ObjectStoreError::Invalid(format!(
                "pending commit key does not match generation {}: {wal_key}",
                pending.target_generation
            )));
        }
        self.validate_snapshot_root(&pending.root, pending.target_generation)
    }

    fn validate_manifest_root_key(&self, root: &str) -> Result<(), ObjectStoreError> {
        if !root.starts_with(&self.snapshot_prefix()) {
            return Err(ObjectStoreError::Invalid(format!(
                "manifest root is outside the table namespace: {root}"
            )));
        }
        Ok(())
    }

    fn validate_snapshot_root(&self, root: &str, generation: u64) -> Result<(), ObjectStoreError> {
        self.validate_manifest_root_key(root)?;
        let expected = self.snapshot_key(generation);
        if root != expected {
            return Err(ObjectStoreError::Invalid(format!(
                "snapshot root does not match generation {generation}: {root}"
            )));
        }
        Ok(())
    }

    fn snapshot_prefix(&self) -> String {
        format!("{}snapshots/", self.prefix)
    }

    fn snapshot_key(&self, generation: u64) -> String {
        format!("{}snapshots/{generation}.xbf", self.prefix)
    }

    fn wal_prefix(&self) -> String {
        format!("{}wal/", self.prefix)
    }

    fn wal_key(&self, generation: u64) -> String {
        format!("{}wal/{generation}.json", self.prefix)
    }
}

fn manifest_history(manifest: &Manifest) -> Vec<u64> {
    if manifest.history.is_empty() {
        vec![manifest.generation]
    } else {
        manifest.history.clone()
    }
}

fn history_with_generation(
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

fn snapshot_generation(prefix: &str, key: &str) -> Option<u64> {
    key.strip_prefix(prefix)?.strip_suffix(".xbf")?.parse().ok()
}

fn validate_namespace(namespace: &str) -> Result<(), ObjectStoreError> {
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
