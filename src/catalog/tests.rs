use super::*;
use crate::xbase::{OperationIr, OperationMethod};
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_CATALOG_ID: AtomicUsize = AtomicUsize::new(0);

fn fixture() -> Vec<u8> {
    include_str!("../../tests/fixtures/users.dbf.hex")
        .split_whitespace()
        .map(|token| u8::from_str_radix(token, 16).unwrap())
        .collect()
}

fn foreign_key_metadata() -> Vec<u8> {
    serde_json::to_vec(&json!({
        "format": "txbase-schema",
        "version": 1,
        "fields": {
            "ID": {"references": "users.ID"}
        }
    }))
    .unwrap()
}

fn temporary_catalog() -> PathBuf {
    let id = NEXT_CATALOG_ID.fetch_add(1, Ordering::Relaxed);
    let root =
        std::env::temp_dir().join(format!("txbase-catalog-test-{}-{id}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir(&root).unwrap();
    root
}

#[test]
fn discovers_direct_dbf_tables_and_ignores_other_files() {
    let root = temporary_catalog();
    fs::write(root.join("users.dbf"), fixture()).unwrap();
    fs::write(root.join("posts.DBF"), fixture()).unwrap();
    fs::write(root.join("users.dbt"), b"memo").unwrap();
    fs::create_dir(root.join("nested")).unwrap();
    fs::write(root.join("nested").join("ignored.dbf"), fixture()).unwrap();
    fs::write(root.join("nested").join("ignored.dbf"), fixture()).unwrap();

    let catalog = Catalog::from_path(&root).unwrap();

    assert_eq!(catalog.table_names(), vec!["posts", "users"]);
    assert_eq!(
        catalog.table_path("users").unwrap(),
        root.join("users.dbf").as_path()
    );
    assert_eq!(catalog.open_table("users").unwrap().active_json().len(), 1);

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn schema_and_verify_cover_every_discovered_table() {
    let root = temporary_catalog();
    fs::write(root.join("users.dbf"), fixture()).unwrap();
    fs::write(root.join("posts.dbf"), fixture()).unwrap();

    let catalog = Catalog::from_path(&root).unwrap();
    catalog.verify().unwrap();
    let schema = catalog.schema_json().unwrap();

    assert_eq!(schema["format"], "txbase-catalog");
    assert_eq!(schema["tables"].as_array().unwrap().len(), 2);
    assert_eq!(schema["tables"][0]["name"], "posts");
    assert_eq!(schema["tables"][1]["schema"]["format"], "dbf");

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn representation_tag_is_stable_and_changes_with_catalog_schema() {
    let schema = json!({
        "format": "txbase-catalog",
        "transaction_id": null,
        "tables": []
    });
    let same_schema = json!({
        "format": "txbase-catalog",
        "transaction_id": null,
        "tables": []
    });
    let changed_schema = json!({
        "format": "txbase-catalog",
        "transaction_id": 1,
        "tables": []
    });

    let tag = super::representation_tag(&schema);
    assert_eq!(tag, super::representation_tag(&same_schema));
    assert_ne!(tag, super::representation_tag(&changed_schema));
    assert!(tag.starts_with("\"txbase-catalog-"));
    assert!(tag.ends_with('"'));
}

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
fn catalog_foreign_keys_validate_mutations_and_parent_removal() {
    let root = temporary_catalog();
    fs::write(root.join("users.dbf"), fixture()).unwrap();
    fs::write(root.join("posts.dbf"), fixture()).unwrap();
    fs::write(root.join("posts.txschema.json"), foreign_key_metadata()).unwrap();
    let catalog = Catalog::from_path(&root).unwrap();

    let orphan = catalog
        .commit_operations_with_preconditions(
            &[OperationIr {
                method: OperationMethod::Post,
                path: "/posts/records".into(),
                body: Some(json!({
                    "ID": 3,
                    "NAME": "Orphan",
                    "AGE": 42,
                    "ACTIVE": true
                })),
            }],
            None,
            None,
        )
        .unwrap_err();
    assert!(
        orphan
            .to_string()
            .contains("foreign key ID has no matching users.ID")
    );
    assert!(
        catalog
            .open_table("posts")
            .unwrap()
            .active_record(3)
            .is_none()
    );

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
        .unwrap();

    let delete_parent = catalog
        .commit_operations_with_preconditions(
            &[OperationIr {
                method: OperationMethod::Delete,
                path: "/users/records/1".into(),
                body: None,
            }],
            None,
            None,
        )
        .unwrap_err();
    assert!(
        delete_parent
            .to_string()
            .contains("foreign key ID has no matching users.ID")
    );
    assert!(
        catalog
            .open_table("users")
            .unwrap()
            .active_record(1)
            .is_some()
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn verify_reports_a_malformed_table_by_name() {
    let root = temporary_catalog();
    fs::write(root.join("broken.dbf"), b"not a DBF").unwrap();

    let error = Catalog::from_path(&root).unwrap().verify().unwrap_err();

    assert!(error.to_string().contains("table broken"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn verify_reports_a_stale_index_sidecar_by_table_name() {
    let root = temporary_catalog();
    let path = root.join("users.dbf");
    fs::write(&path, fixture()).unwrap();
    crate::index::IndexFile::build(
        &path,
        vec![crate::index::IndexDefinition::for_field("NAME")],
    )
    .unwrap()
    .save(&path)
    .unwrap();
    let mut changed = DbfTable::from_path(&path).unwrap();
    changed
        .patch_record(1, json!({"NAME": "Changed"}).as_object().unwrap().clone())
        .unwrap();
    fs::write(&path, changed.to_bytes()).unwrap();

    let error = Catalog::from_path(&root).unwrap().verify().unwrap_err();
    assert!(error.to_string().contains("table users"));
    assert!(error.to_string().contains("index sidecar is invalid"));

    fs::remove_dir_all(root).unwrap();
}
