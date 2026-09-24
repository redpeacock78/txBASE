use super::*;
use crate::xbase::{OperationIr, OperationMethod};
use serde_json::json;
use std::fs;

#[test]
fn commits_named_operations_across_tables() {
    let root = temporary_catalog();
    fs::write(root.join("users.dbf"), fixture()).unwrap();
    fs::write(root.join("posts.dbf"), fixture()).unwrap();
    let catalog = Catalog::from_path(&root).unwrap();

    let transaction_id = catalog
        .commit_operations_with_preconditions(
            &[
                OperationIr {
                    method: OperationMethod::Post,
                    path: "/users/records".into(),
                    body: Some(json!({
                        "ID": 3,
                        "NAME": "Carol",
                        "AGE": 42,
                        "ACTIVE": true
                    })),
                },
                OperationIr {
                    method: OperationMethod::Patch,
                    path: "/posts/records/1".into(),
                    body: Some(json!({"$inc": {"AGE": 1}})),
                },
            ],
            None,
            None,
        )
        .unwrap();
    assert_eq!(transaction_id, 1);
    assert_eq!(catalog.transaction_id().unwrap(), Some(1));
    assert_eq!(catalog.schema_json().unwrap()["transaction_id"], 1);
    let reloaded = Catalog::from_path(&root).unwrap();
    assert_eq!(reloaded.transaction_id().unwrap(), Some(1));

    assert!(
        reloaded
            .open_table("users")
            .unwrap()
            .active_record(3)
            .is_some()
    );
    assert_eq!(
        reloaded
            .open_table("posts")
            .unwrap()
            .active_record(1)
            .unwrap()
            .values["AGE"],
        30
    );
    assert!(!root.join(".txbase.catalog.txn").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn catalog_cdc_records_one_atomic_event_for_changed_tables() {
    let root = temporary_catalog();
    fs::write(root.join("users.dbf"), fixture()).unwrap();
    fs::write(root.join("posts.dbf"), fixture()).unwrap();
    let catalog = Catalog::from_path(&root).unwrap();

    catalog
        .commit_operations_with_preconditions(
            &[
                OperationIr {
                    method: OperationMethod::Post,
                    path: "/users/records".into(),
                    body: Some(json!({
                        "ID": 3,
                        "NAME": "Carol",
                        "AGE": 42,
                        "ACTIVE": true
                    })),
                },
                OperationIr {
                    method: OperationMethod::Patch,
                    path: "/posts/records/1".into(),
                    body: Some(json!({"$inc": {"AGE": 1}})),
                },
            ],
            None,
            None,
        )
        .unwrap();

    let events = Catalog::cdc_events(&root, None).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].transaction_id, 1);
    assert_eq!(
        events[0].tables["users"].changes[0]
            .after
            .as_ref()
            .unwrap()
            .values["ID"],
        3
    );
    assert_eq!(
        events[0].tables["posts"].changes[0]
            .after
            .as_ref()
            .unwrap()
            .values["AGE"],
        30
    );
    assert!(Catalog::cdc_events(&root, Some(1)).unwrap().is_empty());

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn catalog_mvcc_preserves_consistent_cross_table_snapshots() {
    let root = temporary_catalog();
    let users = root.join("users.dbf");
    let posts = root.join("posts.dbf");
    fs::write(&users, fixture()).unwrap();
    fs::write(&posts, fixture()).unwrap();
    let catalog = Catalog::from_path(&root).unwrap();

    assert_eq!(
        catalog
            .commit_operations_with_preconditions(
                &[
                    OperationIr {
                        method: OperationMethod::Post,
                        path: "/users/records".into(),
                        body: Some(json!({
                            "ID": 3,
                            "NAME": "Carol",
                            "AGE": 42,
                            "ACTIVE": true
                        })),
                    },
                    OperationIr {
                        method: OperationMethod::Post,
                        path: "/posts/records".into(),
                        body: Some(json!({
                            "ID": 3,
                            "NAME": "Carol",
                            "AGE": 42,
                            "ACTIVE": true
                        })),
                    },
                ],
                None,
                None,
            )
            .unwrap(),
        1
    );
    assert_eq!(
        catalog
            .commit_operations_with_preconditions(
                &[OperationIr {
                    method: OperationMethod::Patch,
                    path: "/users/records/1".into(),
                    body: Some(json!({"NAME": "Current"})),
                }],
                None,
                None,
            )
            .unwrap(),
        2
    );

    assert_eq!(Catalog::mvcc_versions(&root).unwrap(), vec![1, 2]);
    let first = Catalog::from_path_at(&root, 1).unwrap();
    let second = Catalog::from_path_at(&root, 2).unwrap();
    assert_eq!(first.transaction_id().unwrap(), Some(1));
    assert!(
        first
            .open_table("users")
            .unwrap()
            .active_record(3)
            .is_some()
    );
    assert!(
        first
            .open_table("posts")
            .unwrap()
            .active_record(3)
            .is_some()
    );
    assert_eq!(
        second
            .open_table("users")
            .unwrap()
            .active_record(1)
            .unwrap()
            .values["NAME"],
        "Current"
    );
    assert!(
        second
            .open_table("posts")
            .unwrap()
            .active_record(3)
            .is_some()
    );

    let mut current_users = DbfTable::from_path(&users).unwrap();
    current_users
        .patch_record(1, json!({"NAME": "Later"}).as_object().unwrap().clone())
        .unwrap();
    current_users.save_with_wal(&users).unwrap();
    assert_eq!(
        first
            .open_table("users")
            .unwrap()
            .active_record(1)
            .unwrap()
            .values["NAME"],
        "Alice"
    );

    let error = first
        .commit_operations_with_preconditions(
            &[OperationIr {
                method: OperationMethod::Patch,
                path: "/users/records/1".into(),
                body: Some(json!({"NAME": "Rejected"})),
            }],
            None,
            None,
        )
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("historical catalog snapshots are read-only")
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn catalog_read_transaction_keeps_a_read_only_cross_table_image() {
    let root = temporary_catalog();
    let users = root.join("users.dbf");
    let posts = root.join("posts.dbf");
    fs::write(&users, fixture()).unwrap();
    fs::write(&posts, fixture()).unwrap();
    let catalog = Catalog::from_path(&root).unwrap();

    assert_eq!(
        catalog
            .commit_operations_with_preconditions(
                &[OperationIr {
                    method: OperationMethod::Patch,
                    path: "/posts/records/1".into(),
                    body: Some(json!({"NAME": "Snapshot"})),
                }],
                None,
                None,
            )
            .unwrap(),
        1
    );
    let snapshot = catalog.begin_read().unwrap();
    assert_eq!(snapshot.transaction_id(), Some(1));
    assert_eq!(snapshot.table_names(), vec!["posts", "users"]);
    assert_eq!(snapshot.schema_json()["transaction_id"], 1);

    assert_eq!(
        catalog
            .commit_operations_with_preconditions(
                &[OperationIr {
                    method: OperationMethod::Patch,
                    path: "/users/records/1".into(),
                    body: Some(json!({"NAME": "Current"})),
                }],
                None,
                None,
            )
            .unwrap(),
        2
    );

    let request = crate::query::join::parse(
        br#"{
            "from":"users",
            "join":{
                "type":"inner",
                "table":"posts",
                "on":{"users.ID":{"$eq":{"$field":"posts.ID"}}}
            },
            "projection":{"users.NAME":1,"posts.NAME":1}
        }"#,
    )
    .unwrap();
    let snapshot_rows = snapshot.execute_join(&request).unwrap();
    let current_rows = crate::query::join::execute(&catalog, &request).unwrap();
    assert_eq!(snapshot_rows[0]["users.NAME"], "Alice");
    assert_eq!(snapshot_rows[0]["posts.NAME"], "Snapshot");
    assert_eq!(current_rows[0]["users.NAME"], "Current");
    assert_eq!(current_rows[0]["posts.NAME"], "Snapshot");

    let mut read_only = snapshot.open_table("users").unwrap();
    read_only
        .patch_record(1, json!({"NAME": "blocked"}).as_object().unwrap().clone())
        .unwrap();
    let error = read_only.save_with_wal(&users).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("historical MVCC snapshots are read-only")
    );
    assert_eq!(
        DbfTable::from_path(&users)
            .unwrap()
            .active_record(1)
            .unwrap()
            .values["NAME"],
        "Current"
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn catalog_mvcc_gc_retains_latest_commits_and_allows_future_appends() {
    let root = temporary_catalog();
    fs::write(root.join("users.dbf"), fixture()).unwrap();
    fs::write(root.join("posts.dbf"), fixture()).unwrap();
    let catalog = Catalog::from_path(&root).unwrap();

    for operation in [
        OperationIr {
            method: OperationMethod::Post,
            path: "/users/records".into(),
            body: Some(json!({
                "ID": 3,
                "NAME": "Carol",
                "AGE": 42,
                "ACTIVE": true
            })),
        },
        OperationIr {
            method: OperationMethod::Patch,
            path: "/users/records/1".into(),
            body: Some(json!({"NAME": "Current"})),
        },
        OperationIr {
            method: OperationMethod::Patch,
            path: "/posts/records/1".into(),
            body: Some(json!({"AGE": 31})),
        },
    ] {
        catalog
            .commit_operations_with_preconditions(&[operation], None, None)
            .unwrap();
    }

    assert_eq!(Catalog::gc_mvcc(&root, 2).unwrap(), vec![2, 3]);
    assert!(Catalog::from_path_at(&root, 1).is_err());
    assert_eq!(
        Catalog::from_path_at(&root, 2)
            .unwrap()
            .open_table("users")
            .unwrap()
            .active_record(1)
            .unwrap()
            .values["NAME"],
        "Current"
    );
    assert!(Catalog::gc_mvcc(&root, 0).is_err());

    catalog
        .commit_operations_with_preconditions(
            &[OperationIr {
                method: OperationMethod::Post,
                path: "/posts/records".into(),
                body: Some(json!({
                    "ID": 3,
                    "NAME": "Carol",
                    "AGE": 42,
                    "ACTIVE": true
                })),
            }],
            None,
            None,
        )
        .unwrap();
    assert_eq!(Catalog::mvcc_versions(&root).unwrap(), vec![2, 3, 4]);

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn catalog_transaction_refreshes_indexes_for_each_changed_table() {
    let root = temporary_catalog();
    let users = root.join("users.dbf");
    let posts = root.join("posts.dbf");
    fs::write(&users, fixture()).unwrap();
    fs::write(&posts, fixture()).unwrap();
    for path in [&users, &posts] {
        crate::index::IndexFile::build(
            path,
            vec![crate::index::IndexDefinition::for_field("NAME")],
        )
        .unwrap()
        .save(path)
        .unwrap();
    }
    let catalog = Catalog::from_path(&root).unwrap();

    catalog
        .commit_operations_with_preconditions(
            &[
                OperationIr {
                    method: OperationMethod::Patch,
                    path: "/users/records/1".into(),
                    body: Some(json!({"NAME": "Users new"})),
                },
                OperationIr {
                    method: OperationMethod::Patch,
                    path: "/posts/records/1".into(),
                    body: Some(json!({"NAME": "Posts new"})),
                },
            ],
            None,
            None,
        )
        .unwrap();

    assert!(
        crate::index::IndexFile::load(&users)
            .unwrap()
            .lookup_eq("NAME", &json!("Users new"))
            .unwrap()
            .contains(&1)
    );
    assert!(
        crate::index::IndexFile::load(&posts)
            .unwrap()
            .lookup_eq("NAME", &json!("Posts new"))
            .unwrap()
            .contains(&1)
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejects_a_failed_named_transaction_without_persisting_earlier_tables() {
    let root = temporary_catalog();
    fs::write(root.join("users.dbf"), fixture()).unwrap();
    fs::write(root.join("posts.dbf"), fixture()).unwrap();
    let before_users = fs::read(root.join("users.dbf")).unwrap();
    let before_posts = fs::read(root.join("posts.dbf")).unwrap();
    let catalog = Catalog::from_path(&root).unwrap();

    let error = catalog
        .commit_operations_with_preconditions(
            &[
                OperationIr {
                    method: OperationMethod::Post,
                    path: "/users/records".into(),
                    body: Some(json!({
                        "ID": 3,
                        "NAME": "Carol",
                        "AGE": 42,
                        "ACTIVE": true
                    })),
                },
                OperationIr {
                    method: OperationMethod::Patch,
                    path: "/posts/records/999".into(),
                    body: Some(json!({"NAME": "never committed"})),
                },
            ],
            None,
            None,
        )
        .unwrap_err();
    assert!(matches!(error, CatalogTransactionError::Invalid(_)));
    assert_eq!(fs::read(root.join("users.dbf")).unwrap(), before_users);
    assert_eq!(fs::read(root.join("posts.dbf")).unwrap(), before_posts);
    assert!(
        catalog
            .open_table("users")
            .unwrap()
            .active_record(3)
            .is_none()
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn sidecar_maintenance_is_atomic_without_advancing_catalog_transaction() {
    let root = temporary_catalog();
    fs::write(root.join("users.dbf"), fixture()).unwrap();
    let catalog = Catalog::from_path(&root).unwrap();

    catalog
        .replace_sidecar_without_transaction(".txbase.replication", None, b"version-1".to_vec())
        .unwrap();
    assert_eq!(catalog.transaction_id().unwrap(), None);
    assert_eq!(
        catalog.read_sidecar_bytes(".txbase.replication").unwrap(),
        Some(b"version-1".to_vec())
    );
    assert!(!root.join(".txbase.catalog.txn").exists());

    let error = catalog
        .replace_sidecar_without_transaction(".txbase.replication", None, b"version-2".to_vec())
        .unwrap_err();
    assert!(matches!(
        error,
        CatalogTransactionError::SidecarPreconditionFailed { name }
            if name == ".txbase.replication"
    ));
    assert_eq!(catalog.transaction_id().unwrap(), None);
    assert_eq!(
        catalog.read_sidecar_bytes(".txbase.replication").unwrap(),
        Some(b"version-1".to_vec())
    );

    catalog
        .replace_sidecar_without_transaction(
            ".txbase.replication",
            Some(b"version-1".to_vec()),
            b"version-2".to_vec(),
        )
        .unwrap();
    assert_eq!(catalog.transaction_id().unwrap(), None);
    assert_eq!(
        catalog.read_sidecar_bytes(".txbase.replication").unwrap(),
        Some(b"version-2".to_vec())
    );

    fs::remove_dir_all(root).unwrap();
}
