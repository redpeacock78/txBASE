#[path = "../examples/wasi_query_stream/object_store.rs"]
mod wasi_filesystem_store;

use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

use txbase::edge::{ObjectStore, ObjectStoreError};
use wasi_filesystem_store::FilesystemObjectStore;

#[test]
fn filesystem_store_supports_single_writer_object_operations() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "txbase-wasi-object-store-{}-{nonce}",
        std::process::id()
    ));
    let store = FilesystemObjectStore::new(root.clone()).unwrap();

    assert_eq!(store.get("users/a").unwrap(), None);
    store.put_if_absent("users/a", b"first").unwrap();
    store.put_if_absent("users/a", b"first").unwrap();
    assert!(matches!(
        store.put_if_absent("users/a", b"different"),
        Err(ObjectStoreError::Conflict(_))
    ));
    store.put_if_absent("users/b", b"other").unwrap();
    assert_eq!(
        store.list("users/").unwrap(),
        vec!["users/a".to_owned(), "users/b".to_owned()]
    );

    store
        .compare_and_swap("users/a", Some(b"first".as_slice()), b"second")
        .unwrap();
    assert!(matches!(
        store.compare_and_swap("users/a", Some(b"first".as_slice()), b"third"),
        Err(ObjectStoreError::Conflict(_))
    ));
    assert_eq!(store.get("users/a").unwrap(), Some(b"second".to_vec()));

    store.delete("users/a").unwrap();
    store.delete("users/a").unwrap();
    assert_eq!(store.get("users/a").unwrap(), None);
    fs::remove_dir_all(root).unwrap();
}
