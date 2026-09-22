use super::*;
use crate::xbase::{OperationIr, OperationMethod};
use serde_json::json;
use std::fs;

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

fn foreign_key_metadata_with_actions(on_delete: &str, on_update: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "format": "txbase-schema",
        "version": 1,
        "fields": {
            "ID": {
                "references": "users.ID",
                "on_delete": on_delete,
                "on_update": on_update
            }
        }
    }))
    .unwrap()
}

fn composite_foreign_key_metadata() -> Vec<u8> {
    serde_json::to_vec(&json!({
        "format": "txbase-schema",
        "version": 1,
        "fields": {},
        "constraints": {
            "foreign_keys": [{
                "fields": ["ID", "AGE"],
                "references": {
                    "table": "users",
                    "fields": ["ID", "AGE"]
                }
            }]
        }
    }))
    .unwrap()
}

fn composite_foreign_key_metadata_with_actions(on_delete: &str, on_update: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "format": "txbase-schema",
        "version": 1,
        "fields": {},
        "constraints": {
            "foreign_keys": [{
                "fields": ["ID", "AGE"],
                "references": {
                    "table": "users",
                    "fields": ["ID", "AGE"]
                },
                "on_delete": on_delete,
                "on_update": on_update
            }]
        }
    }))
    .unwrap()
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
fn catalog_composite_foreign_keys_validate_tuples_and_parent_removal() {
    let root = temporary_catalog();
    fs::write(root.join("users.dbf"), fixture()).unwrap();
    fs::write(root.join("posts.dbf"), fixture()).unwrap();
    fs::write(
        root.join("posts.txschema.json"),
        composite_foreign_key_metadata(),
    )
    .unwrap();
    let catalog = Catalog::from_path(&root).unwrap();

    catalog
        .commit_operations_with_preconditions(
            &[OperationIr {
                method: OperationMethod::Patch,
                path: "/posts/records/1".into(),
                body: Some(json!({"NAME": "Child"})),
            }],
            None,
            None,
        )
        .unwrap();

    let orphan = catalog
        .commit_operations_with_preconditions(
            &[OperationIr {
                method: OperationMethod::Post,
                path: "/posts/records".into(),
                body: Some(json!({
                    "ID": 1,
                    "NAME": "Orphan",
                    "AGE": 999,
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
            .contains("foreign key (ID, AGE) has no matching users.(ID, AGE)")
    );

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
            .contains("foreign key (ID, AGE) has no matching users.(ID, AGE)")
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
fn catalog_foreign_key_actions_cascade_parent_changes_and_deletes() {
    let root = temporary_catalog();
    fs::write(root.join("users.dbf"), fixture()).unwrap();
    fs::write(root.join("posts.dbf"), fixture()).unwrap();
    fs::write(
        root.join("posts.txschema.json"),
        foreign_key_metadata_with_actions("cascade", "cascade"),
    )
    .unwrap();
    let catalog = Catalog::from_path(&root).unwrap();

    catalog
        .commit_operations_with_preconditions(
            &[OperationIr {
                method: OperationMethod::Patch,
                path: "/users/records/1".into(),
                body: Some(json!({"ID": 9})),
            }],
            None,
            None,
        )
        .unwrap();
    assert_eq!(
        catalog
            .open_table("posts")
            .unwrap()
            .active_record(1)
            .unwrap()
            .values["ID"],
        9
    );

    catalog
        .commit_operations_with_preconditions(
            &[OperationIr {
                method: OperationMethod::Delete,
                path: "/users/records/1".into(),
                body: None,
            }],
            None,
            None,
        )
        .unwrap();
    assert!(
        catalog
            .open_table("posts")
            .unwrap()
            .active_record(1)
            .is_none()
    );
    assert!(
        catalog
            .open_table("users")
            .unwrap()
            .active_record(1)
            .is_none()
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn catalog_foreign_key_set_null_action_clears_children_atomically() {
    let root = temporary_catalog();
    fs::write(root.join("users.dbf"), fixture()).unwrap();
    fs::write(root.join("posts.dbf"), fixture()).unwrap();
    fs::write(
        root.join("posts.txschema.json"),
        foreign_key_metadata_with_actions("set_null", "restrict"),
    )
    .unwrap();
    let catalog = Catalog::from_path(&root).unwrap();

    catalog
        .commit_operations_with_preconditions(
            &[OperationIr {
                method: OperationMethod::Delete,
                path: "/users/records/1".into(),
                body: None,
            }],
            None,
            None,
        )
        .unwrap();

    assert!(
        catalog
            .open_table("posts")
            .unwrap()
            .active_record(1)
            .unwrap()
            .values["ID"]
            .is_null()
    );
    assert!(
        catalog
            .open_table("users")
            .unwrap()
            .active_record(1)
            .is_none()
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn catalog_composite_foreign_key_actions_follow_parent_tuples() {
    let root = temporary_catalog();
    fs::write(root.join("users.dbf"), fixture()).unwrap();
    fs::write(root.join("posts.dbf"), fixture()).unwrap();
    fs::write(
        root.join("posts.txschema.json"),
        composite_foreign_key_metadata_with_actions("cascade", "cascade"),
    )
    .unwrap();
    let catalog = Catalog::from_path(&root).unwrap();

    catalog
        .commit_operations_with_preconditions(
            &[OperationIr {
                method: OperationMethod::Patch,
                path: "/users/records/1".into(),
                body: Some(json!({"AGE": 42})),
            }],
            None,
            None,
        )
        .unwrap();
    assert_eq!(
        catalog
            .open_table("posts")
            .unwrap()
            .active_record(1)
            .unwrap()
            .values["AGE"],
        42
    );

    catalog
        .commit_operations_with_preconditions(
            &[OperationIr {
                method: OperationMethod::Delete,
                path: "/users/records/1".into(),
                body: None,
            }],
            None,
            None,
        )
        .unwrap();
    assert!(
        catalog
            .open_table("posts")
            .unwrap()
            .active_record(1)
            .is_none()
    );

    fs::remove_dir_all(root).unwrap();
}
