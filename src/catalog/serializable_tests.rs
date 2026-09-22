use super::{Catalog, CatalogTransactionError};
use crate::xbase::{OperationIr, OperationMethod};
use fs2::FileExt;
use serde_json::json;
use std::fs;
use std::fs::OpenOptions;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_SERIALIZABLE_CATALOG_ID: AtomicUsize = AtomicUsize::new(0);

fn fixture() -> Vec<u8> {
    include_str!("../../tests/fixtures/users.dbf.hex")
        .split_whitespace()
        .map(|token| u8::from_str_radix(token, 16).unwrap())
        .collect()
}

fn temporary_catalog() -> PathBuf {
    let id = NEXT_SERIALIZABLE_CATALOG_ID.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "txbase-serializable-catalog-test-{}-{id}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir(&root).unwrap();
    root
}

#[test]
fn serializable_catalog_transaction_holds_all_locks_and_commits_once() {
    let root = temporary_catalog();
    fs::write(root.join("users.dbf"), fixture()).unwrap();
    fs::write(root.join("posts.dbf"), fixture()).unwrap();
    let catalog = Catalog::from_path(&root).unwrap();
    let mut transaction = catalog.begin_serializable().unwrap();

    let catalog_probe = OpenOptions::new()
        .read(true)
        .write(true)
        .open(root.join(".txbase.catalog.lock"))
        .unwrap();
    assert!(catalog_probe.try_lock_exclusive().is_err());
    drop(catalog_probe);
    for table in ["users", "posts"] {
        let table_probe = OpenOptions::new()
            .read(true)
            .write(true)
            .open(root.join(format!("{table}.txbase.lock")))
            .unwrap();
        assert!(table_probe.try_lock_exclusive().is_err());
        drop(table_probe);
    }

    for table in ["users", "posts"] {
        transaction
            .apply(&OperationIr {
                method: OperationMethod::Patch,
                path: format!("/{table}/records/1"),
                body: Some(json!({"NAME": "serial"})),
            })
            .unwrap();
    }
    assert_eq!(transaction.table_names(), vec!["posts", "users"]);
    assert_eq!(
        transaction
            .open_table("users")
            .unwrap()
            .active_record(1)
            .unwrap()
            .values["NAME"],
        "serial"
    );

    assert_eq!(transaction.commit().unwrap(), 1);
    let reloaded = Catalog::from_path(&root).unwrap();
    assert_eq!(reloaded.transaction_id().unwrap(), Some(1));
    for table in ["users", "posts"] {
        assert_eq!(
            reloaded
                .open_table(table)
                .unwrap()
                .active_record(1)
                .unwrap()
                .values["NAME"],
            "serial"
        );
    }
    assert_eq!(Catalog::cdc_events(&root, None).unwrap().len(), 1);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn serializable_catalog_transaction_rollback_releases_all_locks() {
    let root = temporary_catalog();
    fs::write(root.join("users.dbf"), fixture()).unwrap();
    fs::write(root.join("posts.dbf"), fixture()).unwrap();
    let catalog = Catalog::from_path(&root).unwrap();
    let transaction = catalog.begin_serializable().unwrap();
    transaction.rollback();

    let catalog_probe = OpenOptions::new()
        .read(true)
        .write(true)
        .open(root.join(".txbase.catalog.lock"))
        .unwrap();
    catalog_probe.try_lock_exclusive().unwrap();
    catalog_probe.unlock().unwrap();
    drop(catalog_probe);
    for table in ["users", "posts"] {
        let table_probe = OpenOptions::new()
            .read(true)
            .write(true)
            .open(root.join(format!("{table}.txbase.lock")))
            .unwrap();
        table_probe.try_lock_exclusive().unwrap();
        table_probe.unlock().unwrap();
        drop(table_probe);
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn serializable_catalog_transaction_rejects_table_set_change() {
    let root = temporary_catalog();
    fs::write(root.join("users.dbf"), fixture()).unwrap();
    let catalog = Catalog::from_path(&root).unwrap();
    let mut transaction = catalog.begin_serializable().unwrap();

    fs::write(root.join("posts.dbf"), fixture()).unwrap();
    transaction
        .apply(&OperationIr {
            method: OperationMethod::Patch,
            path: "/users/records/1".into(),
            body: Some(json!({"NAME": "not committed"})),
        })
        .unwrap();

    assert!(matches!(
        transaction.commit(),
        Err(CatalogTransactionError::TableSetChanged)
    ));
    assert_eq!(
        catalog
            .open_table("users")
            .unwrap()
            .active_record(1)
            .unwrap()
            .values["NAME"],
        "Alice"
    );
    assert!(
        Catalog::from_path(&root)
            .unwrap()
            .table_path("posts")
            .is_some()
    );
    fs::remove_dir_all(root).unwrap();
}
