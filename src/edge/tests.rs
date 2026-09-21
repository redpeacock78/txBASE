use super::{
    CommitResult, FilesystemObjectStore, Manifest, MemoryObjectStore, ObjectStore,
    ObjectStoreError, ObjectTable,
};
use crate::xbf::{XbfField, XbfRecord, XbfTable, XbfType, XbfValue, encode};
use serde_json::json;
use std::env;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

fn temporary_directory(label: &str) -> PathBuf {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    env::temp_dir().join(format!(
        "txbase-edge-{label}-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ))
}

fn table(generation: u64, name: &str) -> XbfTable {
    XbfTable {
        generation,
        fields: vec![XbfField {
            name: "NAME".into(),
            ty: XbfType::String,
            nullable: false,
            primary_key: false,
            unique: false,
        }],
        records: vec![XbfRecord {
            deleted: false,
            values: vec![XbfValue::String(name.into())],
        }],
    }
}

#[test]
fn commits_and_reads_one_consistent_xbf_generation() {
    let store = MemoryObjectStore::new();
    let object_table = ObjectTable::new(store.clone(), "users").unwrap();
    let first = table(0, "Alice");
    assert_eq!(
        object_table.commit(&first).unwrap(),
        CommitResult::Committed { generation: 0 }
    );
    assert_eq!(object_table.read().unwrap(), Some(first));

    let second = table(1, "Bob");
    assert_eq!(
        object_table.commit(&second).unwrap(),
        CommitResult::Committed { generation: 1 }
    );
    assert_eq!(object_table.read().unwrap(), Some(second.clone()));

    let manifest = object_table.manifest().unwrap().unwrap();
    assert_eq!(manifest.generation, 1);
    assert_eq!(manifest.wal_head, 1);
    assert!(manifest.root.ends_with("snapshots/1.xbf"));
    let removed = object_table.cleanup_orphans().unwrap();
    assert!(removed.iter().any(|key| key.ends_with("snapshots/0.xbf")));
    assert_eq!(object_table.read().unwrap(), Some(second));
}

#[test]
fn compare_and_swap_rejects_concurrent_writers_and_allows_idempotent_retry() {
    let store = MemoryObjectStore::new();
    let first_writer = ObjectTable::new(store.clone(), "users").unwrap();
    let second_writer = ObjectTable::new(store.clone(), "users").unwrap();
    let first = table(1, "Alice");
    let conflicting = table(1, "Bob");

    assert!(matches!(
        first_writer.commit(&first).unwrap(),
        CommitResult::Committed { generation: 1 }
    ));
    assert_eq!(
        second_writer.commit(&first).unwrap(),
        CommitResult::AlreadyCommitted { generation: 1 }
    );
    assert!(matches!(
        second_writer.commit(&conflicting),
        Err(ObjectStoreError::Conflict(_))
    ));
    assert_eq!(first_writer.read().unwrap(), Some(first));
}

#[test]
fn rejects_a_manifest_that_points_to_a_different_snapshot_generation() {
    let store = MemoryObjectStore::new();
    let object_table = ObjectTable::new(store.clone(), "users").unwrap();
    let first = table(1, "Alice");
    object_table.commit(&first).unwrap();
    let old_manifest = store.get(object_table.manifest_key()).unwrap().unwrap();
    let second = table(2, "Bob");
    let second_root = "users/snapshots/2.xbf";
    store
        .put_if_absent(second_root, &encode(&second).unwrap())
        .unwrap();
    let mixed_manifest = Manifest {
        version: 1,
        generation: 1,
        root: second_root.into(),
        wal_head: 1,
    }
    .to_bytes()
    .unwrap();
    store
        .compare_and_swap(
            object_table.manifest_key(),
            Some(&old_manifest),
            &mixed_manifest,
        )
        .unwrap();

    assert!(matches!(
        object_table.read(),
        Err(ObjectStoreError::Invalid(message)) if message.contains("does not match")
    ));
}

#[test]
fn recovers_after_snapshot_and_wal_publication_before_manifest_cas() {
    let failure = Arc::new(AtomicBool::new(true));
    let store = FailingCasStore {
        inner: MemoryObjectStore::new(),
        failure: failure.clone(),
    };
    let object_table = ObjectTable::new(store.clone(), "users").unwrap();
    let pending = table(3, "Carol");

    assert!(matches!(
        object_table.commit(&pending),
        Err(ObjectStoreError::Unavailable(_))
    ));
    assert!(object_table.manifest().unwrap().is_none());
    assert_eq!(object_table.recover().unwrap(), 1);
    assert_eq!(object_table.read().unwrap(), Some(pending));
}

#[test]
fn recovers_pending_generations_in_numeric_order() {
    let store = MemoryObjectStore::new();
    let object_table = ObjectTable::new(store.clone(), "users").unwrap();
    object_table.commit(&table(0, "Base")).unwrap();

    for (generation, base_generation, name) in [(2, 0, "Two"), (10, 2, "Ten")] {
        let snapshot = table(generation, name);
        let root = format!("users/snapshots/{generation}.xbf");
        store
            .put_if_absent(&root, &encode(&snapshot).unwrap())
            .unwrap();
        let pending = json!({
            "version": 1,
            "base_generation": base_generation,
            "target_generation": generation,
            "root": root,
            "wal_head": generation,
        });
        store
            .put_if_absent(
                &format!("users/wal/{generation}.json"),
                &serde_json::to_vec(&pending).unwrap(),
            )
            .unwrap();
    }

    assert_eq!(object_table.recover().unwrap(), 2);
    assert_eq!(object_table.read().unwrap(), Some(table(10, "Ten")));
}

#[test]
fn filesystem_store_persists_objects_and_rejects_unsafe_keys() {
    let root = temporary_directory("store");
    {
        let store = FilesystemObjectStore::new(&root).unwrap();
        store
            .put_if_absent("users/snapshots/0.xbf", b"snapshot")
            .unwrap();
        store.put_if_absent("users/wal/0.json", b"pending").unwrap();
        assert_eq!(
            store.list("users/snapshots/").unwrap(),
            vec!["users/snapshots/0.xbf"]
        );
        assert_eq!(
            store.get("users/snapshots/0.xbf").unwrap(),
            Some(b"snapshot".to_vec())
        );
        assert!(matches!(
            store.get("../outside"),
            Err(ObjectStoreError::Invalid(_))
        ));
        assert!(matches!(
            store.get("users/../outside"),
            Err(ObjectStoreError::Invalid(_))
        ));
    }
    {
        let store = FilesystemObjectStore::new(&root).unwrap();
        assert_eq!(
            store.get("users/wal/0.json").unwrap(),
            Some(b"pending".to_vec())
        );
        store
            .compare_and_swap("users/manifest.json", None, br#"{"generation":0}"#)
            .unwrap();
        assert!(matches!(
            store.compare_and_swap("users/manifest.json", None, b"other"),
            Err(ObjectStoreError::Conflict(_))
        ));
        store.delete("users/wal/0.json").unwrap();
        assert_eq!(store.get("users/wal/0.json").unwrap(), None);
    }
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn filesystem_object_table_reopens_and_recovers_an_interrupted_commit() {
    let root = temporary_directory("table");
    let failure = Arc::new(AtomicBool::new(true));
    {
        let store = FilesystemObjectStore::new(&root).unwrap();
        let failing_store = FailingCasStore {
            inner: store,
            failure: failure.clone(),
        };
        let object_table = ObjectTable::new(failing_store, "users").unwrap();
        let pending = table(4, "Carol");
        assert!(matches!(
            object_table.commit(&pending),
            Err(ObjectStoreError::Unavailable(_))
        ));
    }
    {
        let store = FilesystemObjectStore::new(&root).unwrap();
        let object_table = ObjectTable::new(store, "users").unwrap();
        assert_eq!(object_table.recover().unwrap(), 1);
        assert_eq!(object_table.read().unwrap(), Some(table(4, "Carol")));
    }
    let _ = std::fs::remove_dir_all(root);
}

#[derive(Clone)]
struct FailingCasStore<S> {
    inner: S,
    failure: Arc<AtomicBool>,
}

impl<S: ObjectStore> ObjectStore for FailingCasStore<S> {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>, ObjectStoreError> {
        self.inner.get(key)
    }

    fn put_if_absent(&self, key: &str, bytes: &[u8]) -> Result<(), ObjectStoreError> {
        self.inner.put_if_absent(key, bytes)
    }

    fn compare_and_swap(
        &self,
        key: &str,
        expected: Option<&[u8]>,
        replacement: &[u8],
    ) -> Result<(), ObjectStoreError> {
        if self.failure.swap(false, Ordering::SeqCst) {
            return Err(ObjectStoreError::Unavailable(
                "injected manifest publication failure".into(),
            ));
        }
        self.inner.compare_and_swap(key, expected, replacement)
    }

    fn delete(&self, key: &str) -> Result<(), ObjectStoreError> {
        self.inner.delete(key)
    }

    fn list(&self, prefix: &str) -> Result<Vec<String>, ObjectStoreError> {
        self.inner.list(prefix)
    }
}
