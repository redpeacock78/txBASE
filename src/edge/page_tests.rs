use super::tests::{block_on, table};
use super::{
    AsyncObjectTable, CommitResult, Manifest, MemoryObjectStore, ObjectStore, ObjectStoreError,
    ObjectTable, SyncObjectStoreAdapter,
};
use crate::xbf::encode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[test]
fn commit_reuses_an_identical_orphan_page_after_interruption() {
    let store = MemoryObjectStore::new();
    let pending = table(0, "Alice");
    store
        .put_if_absent("users/pages/0/0.bin", &encode(&pending).unwrap())
        .unwrap();
    let object_table = ObjectTable::new(store, "users").unwrap();

    assert_eq!(
        object_table.commit(&pending).unwrap(),
        CommitResult::Committed { generation: 0 }
    );
    assert_eq!(object_table.read().unwrap(), Some(pending));
}

#[test]
fn commit_rejects_a_conflicting_orphan_page_and_cleans_its_intent() {
    let store = MemoryObjectStore::new();
    let pending = table(0, "Alice");
    store
        .put_if_absent("users/pages/0/0.bin", &encode(&table(0, "Bob")).unwrap())
        .unwrap();
    let object_table = ObjectTable::new(store, "users").unwrap();

    assert!(matches!(
        object_table.commit(&pending),
        Err(ObjectStoreError::Conflict(_))
    ));
    assert!(object_table.manifest().unwrap().is_none());
    assert_eq!(object_table.recover().unwrap(), 0);
    assert!(
        object_table
            .cleanup_orphans()
            .unwrap()
            .iter()
            .any(|key| key.ends_with("pages/0/0.bin"))
    );
    assert_eq!(
        object_table.commit(&pending).unwrap(),
        CommitResult::Committed { generation: 0 }
    );
}

#[test]
fn page_reads_validate_crc_and_reuse_unchanged_pages_across_generations() {
    const PAGE_SIZE: usize = 4 * 1024 * 1024;
    let store = MemoryObjectStore::new();
    let object_table = ObjectTable::new(store.clone(), "users").unwrap();
    let payload = "x".repeat(PAGE_SIZE + 8192);
    let first = table(0, &payload);
    object_table.commit(&first).unwrap();
    let first_manifest = object_table.page_manifest_at(0).unwrap().unwrap();
    assert_eq!(first_manifest.pages.len(), 2);
    assert_eq!(
        object_table.read_page_at(0, 0).unwrap().unwrap().len(),
        PAGE_SIZE
    );

    let second = table(1, &payload);
    object_table.commit(&second).unwrap();
    let second_manifest = object_table.page_manifest_at(1).unwrap().unwrap();
    assert_eq!(second_manifest.pages[1].generation, 0);
    assert_eq!(
        object_table.read_page_at(1, 1).unwrap(),
        object_table.read_page_at(0, 1).unwrap()
    );
    assert_eq!(object_table.read_at(1).unwrap(), Some(second));

    let page_key = "users/pages/1/0.bin";
    store.delete(page_key).unwrap();
    store.put_if_absent(page_key, b"corrupt").unwrap();
    assert!(matches!(
        object_table.read_page_at(1, 0),
        Err(ObjectStoreError::Invalid(message)) if message.contains("CRC-32C")
    ));

    let removed = object_table.retain_generations(1).unwrap();
    assert!(
        removed
            .iter()
            .any(|key| key.ends_with("snapshots/0.pages.json"))
    );
    assert!(removed.iter().any(|key| key.ends_with("pages/0/0.bin")));
    assert!(!removed.iter().any(|key| key.ends_with("pages/0/1.bin")));
}

#[test]
fn legacy_v1_snapshot_remains_readable_after_a_v2_page_commit() {
    let store = MemoryObjectStore::new();
    let object_table = ObjectTable::new(store.clone(), "users").unwrap();
    let legacy = table(0, "legacy");
    let legacy_root = "users/snapshots/0.xbf";
    store
        .put_if_absent(legacy_root, &encode(&legacy).unwrap())
        .unwrap();
    let legacy_manifest = Manifest {
        version: 1,
        generation: 0,
        root: legacy_root.into(),
        wal_head: 0,
        history: vec![],
    }
    .to_bytes()
    .unwrap();
    store
        .compare_and_swap(object_table.manifest_key(), None, &legacy_manifest)
        .unwrap();

    assert_eq!(object_table.read().unwrap(), Some(legacy.clone()));
    assert!(object_table.page_manifest_at(0).unwrap().is_none());
    object_table.commit(&table(1, "current")).unwrap();
    assert_eq!(object_table.read_at(0).unwrap(), Some(legacy));
    assert_eq!(object_table.manifest().unwrap().unwrap().version, 2);
}

#[test]
fn read_rejects_two_snapshot_roots_for_one_generation() {
    let store = MemoryObjectStore::new();
    let object_table = ObjectTable::new(store.clone(), "users").unwrap();
    object_table.commit(&table(0, "current")).unwrap();
    store
        .put_if_absent(
            "users/snapshots/0.xbf",
            &encode(&table(0, "legacy")).unwrap(),
        )
        .unwrap();

    assert!(matches!(
        object_table.read_at(0),
        Err(ObjectStoreError::Invalid(message)) if message.contains("multiple snapshot roots")
    ));
}

#[test]
fn recovery_skips_an_incomplete_page_intent_until_retry() {
    const PAGE_SIZE: usize = 4 * 1024 * 1024;
    let inner = MemoryObjectStore::new();
    let store = FailingPageStore {
        inner: inner.clone(),
        failing_key: "users/pages/0/1.bin".into(),
        failure: Arc::new(AtomicBool::new(true)),
    };
    let object_table = ObjectTable::new(store, "users").unwrap();
    let snapshot = table(0, &"x".repeat(PAGE_SIZE + 8192));

    assert!(matches!(
        object_table.commit(&snapshot),
        Err(ObjectStoreError::Unavailable(_))
    ));
    assert!(object_table.manifest().unwrap().is_none());
    assert_eq!(object_table.recover().unwrap(), 0);
    assert!(inner.get("users/pages/0/0.bin").unwrap().is_some());
    assert_eq!(
        object_table.commit(&snapshot).unwrap(),
        CommitResult::Committed { generation: 0 }
    );
    assert_eq!(object_table.read().unwrap(), Some(snapshot));
}

#[test]
fn async_page_reads_reuse_unchanged_pages_across_generations() {
    const PAGE_SIZE: usize = 4 * 1024 * 1024;
    let store = MemoryObjectStore::new();
    let object_table = AsyncObjectTable::new(SyncObjectStoreAdapter::new(store), "users").unwrap();
    let payload = "y".repeat(PAGE_SIZE + 8192);
    let first = table(0, &payload);
    block_on(object_table.commit(&first)).unwrap();
    let second = table(1, &payload);
    block_on(object_table.commit(&second)).unwrap();

    let manifest = block_on(object_table.page_manifest_at(1)).unwrap().unwrap();
    assert_eq!(manifest.pages.len(), 2);
    assert_eq!(manifest.pages[1].generation, 0);
    assert_eq!(
        block_on(object_table.read_page_at(1, 0))
            .unwrap()
            .unwrap()
            .len(),
        PAGE_SIZE
    );
    assert_eq!(block_on(object_table.read_at(0)).unwrap(), Some(first));
}

#[test]
fn async_commit_reuses_an_identical_orphan_page_after_interruption() {
    let store = MemoryObjectStore::new();
    let pending = table(0, "Alice");
    store
        .put_if_absent("users/pages/0/0.bin", &encode(&pending).unwrap())
        .unwrap();
    let object_table = AsyncObjectTable::new(SyncObjectStoreAdapter::new(store), "users").unwrap();

    assert_eq!(
        block_on(object_table.commit(&pending)).unwrap(),
        CommitResult::Committed { generation: 0 }
    );
    assert_eq!(block_on(object_table.read()).unwrap(), Some(pending));
}

#[test]
fn async_recovery_skips_an_incomplete_page_intent_until_retry() {
    const PAGE_SIZE: usize = 4 * 1024 * 1024;
    let inner = MemoryObjectStore::new();
    let store = FailingPageStore {
        inner: inner.clone(),
        failing_key: "users/pages/0/1.bin".into(),
        failure: Arc::new(AtomicBool::new(true)),
    };
    let object_table = AsyncObjectTable::new(SyncObjectStoreAdapter::new(store), "users").unwrap();
    let snapshot = table(0, &"y".repeat(PAGE_SIZE + 8192));

    assert!(matches!(
        block_on(object_table.commit(&snapshot)),
        Err(ObjectStoreError::Unavailable(_))
    ));
    assert_eq!(block_on(object_table.recover()).unwrap(), 0);
    assert!(inner.get("users/pages/0/0.bin").unwrap().is_some());
    assert_eq!(
        block_on(object_table.commit(&snapshot)).unwrap(),
        CommitResult::Committed { generation: 0 }
    );
    assert_eq!(block_on(object_table.read()).unwrap(), Some(snapshot));
}

#[derive(Clone)]
struct FailingPageStore<S> {
    inner: S,
    failing_key: String,
    failure: Arc<AtomicBool>,
}

impl<S: ObjectStore> ObjectStore for FailingPageStore<S> {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>, ObjectStoreError> {
        self.inner.get(key)
    }

    fn put_if_absent(&self, key: &str, bytes: &[u8]) -> Result<(), ObjectStoreError> {
        if key == self.failing_key && self.failure.swap(false, Ordering::SeqCst) {
            return Err(ObjectStoreError::Unavailable(
                "injected page publication failure".into(),
            ));
        }
        self.inner.put_if_absent(key, bytes)
    }

    fn compare_and_swap(
        &self,
        key: &str,
        expected: Option<&[u8]>,
        replacement: &[u8],
    ) -> Result<(), ObjectStoreError> {
        self.inner.compare_and_swap(key, expected, replacement)
    }

    fn delete(&self, key: &str) -> Result<(), ObjectStoreError> {
        self.inner.delete(key)
    }

    fn list(&self, prefix: &str) -> Result<Vec<String>, ObjectStoreError> {
        self.inner.list(prefix)
    }
}
