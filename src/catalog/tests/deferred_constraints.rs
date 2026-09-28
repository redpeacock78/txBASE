use super::*;
use crate::catalog::CatalogTransactionError;
use crate::xbase::{OperationIr, OperationMethod};
use serde_json::json;
use std::fs;

fn scalar_foreign_key(deferred: bool, on_delete: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "format": "txbase-schema",
        "version": 1,
        "fields": {
            "ID": {
                "references": "users.ID",
                "deferred": deferred,
                "on_delete": on_delete,
                "on_update": "no_action"
            }
        }
    }))
    .unwrap()
}

fn composite_foreign_key() -> Vec<u8> {
    serde_json::to_vec(&json!({
        "format": "txbase-schema",
        "version": 1,
        "constraints": {
            "foreign_keys": [{
                "fields": ["ID", "AGE"],
                "references": {"table": "users", "fields": ["ID", "AGE"]},
                "deferred": true,
                "on_delete": "no_action",
                "on_update": "no_action"
            }]
        }
    }))
    .unwrap()
}

fn scalar_parent_key() -> Vec<u8> {
    serde_json::to_vec(&json!({
        "format": "txbase-schema",
        "version": 1,
        "fields": {"ID": {"primary": true}}
    }))
    .unwrap()
}

fn composite_parent_key() -> Vec<u8> {
    serde_json::to_vec(&json!({
        "format": "txbase-schema",
        "version": 1,
        "constraints": {"unique": [["AGE", "ID"]]}
    }))
    .unwrap()
}

fn operation(method: OperationMethod, path: &str, body: Option<serde_json::Value>) -> OperationIr {
    OperationIr {
        method,
        path: path.into(),
        body,
    }
}

fn insert_record(path: &str, id: i64, age: i64) -> OperationIr {
    operation(
        OperationMethod::Post,
        path,
        Some(json!({
            "ID": id,
            "NAME": format!("record-{id}"),
            "AGE": age,
            "ACTIVE": true
        })),
    )
}

fn foreign_key_catalog(deferred: bool, on_delete: &str) -> (std::path::PathBuf, Catalog) {
    let root = temporary_catalog();
    fs::write(root.join("users.dbf"), fixture()).unwrap();
    fs::write(root.join("users.txschema.json"), scalar_parent_key()).unwrap();
    fs::write(root.join("posts.dbf"), fixture()).unwrap();
    fs::write(
        root.join("posts.txschema.json"),
        scalar_foreign_key(deferred, on_delete),
    )
    .unwrap();
    let catalog = Catalog::from_path(&root).unwrap();
    (root, catalog)
}

#[test]
fn deferred_scalar_foreign_key_is_checked_at_the_catalog_commit() {
    let (root, catalog) = foreign_key_catalog(true, "no_action");

    catalog
        .commit_operations_with_preconditions(
            &[
                insert_record("/posts/records", 3, 42),
                insert_record("/users/records", 3, 42),
            ],
            None,
            None,
        )
        .unwrap();
    assert!(
        catalog
            .open_table("posts")
            .unwrap()
            .active_record(3)
            .is_some()
    );
    assert!(
        catalog
            .open_table("users")
            .unwrap()
            .active_record(3)
            .is_some()
    );

    let error = catalog
        .commit_operations_with_preconditions(&[insert_record("/posts/records", 4, 43)], None, None)
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("foreign key ID has no matching users.ID")
    );
    assert!(
        catalog
            .open_table("posts")
            .unwrap()
            .active_record(4)
            .is_none()
    );
    assert_eq!(catalog.transaction_id().unwrap(), Some(1));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn deferred_composite_foreign_key_accepts_parent_inserted_later() {
    let root = temporary_catalog();
    fs::write(root.join("users.dbf"), fixture()).unwrap();
    fs::write(root.join("users.txschema.json"), composite_parent_key()).unwrap();
    fs::write(root.join("posts.dbf"), fixture()).unwrap();
    fs::write(root.join("posts.txschema.json"), composite_foreign_key()).unwrap();
    let catalog = Catalog::from_path(&root).unwrap();

    catalog
        .commit_operations_with_preconditions(
            &[
                insert_record("/posts/records", 3, 42),
                insert_record("/users/records", 3, 42),
            ],
            None,
            None,
        )
        .unwrap();
    assert!(
        catalog
            .open_table("posts")
            .unwrap()
            .active_record(3)
            .is_some()
    );
    assert!(
        catalog
            .open_table("users")
            .unwrap()
            .active_record(3)
            .is_some()
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn no_action_can_be_repaired_before_commit_but_restrict_is_immediate() {
    let (root, catalog) = foreign_key_catalog(true, "no_action");
    catalog
        .commit_operations_with_preconditions(
            &[
                operation(OperationMethod::Delete, "/users/records/1", None),
                operation(OperationMethod::Delete, "/posts/records/1", None),
            ],
            None,
            None,
        )
        .unwrap();
    assert!(
        catalog
            .open_table("users")
            .unwrap()
            .active_record(1)
            .is_none()
    );
    assert!(
        catalog
            .open_table("posts")
            .unwrap()
            .active_record(1)
            .is_none()
    );
    fs::remove_dir_all(root).unwrap();

    let (root, catalog) = foreign_key_catalog(true, "restrict");
    let error = catalog
        .commit_operations_with_preconditions(
            &[
                operation(OperationMethod::Delete, "/users/records/1", None),
                operation(OperationMethod::Delete, "/posts/records/1", None),
            ],
            None,
            None,
        )
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("RESTRICT prevents the parent change")
    );
    assert!(
        catalog
            .open_table("users")
            .unwrap()
            .active_record(1)
            .is_some()
    );
    assert!(
        catalog
            .open_table("posts")
            .unwrap()
            .active_record(1)
            .is_some()
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn immediate_foreign_key_rejects_an_intermediate_orphan() {
    let (root, catalog) = foreign_key_catalog(false, "no_action");
    let error = catalog
        .commit_operations_with_preconditions(
            &[
                insert_record("/posts/records", 3, 42),
                insert_record("/users/records", 3, 42),
            ],
            None,
            None,
        )
        .unwrap_err();
    assert!(
        error
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
fn failed_serializable_statement_aborts_without_publishing_partial_work() {
    let (root, catalog) = foreign_key_catalog(false, "no_action");
    let mut transaction = catalog.begin_serializable().unwrap();
    assert!(
        transaction
            .apply(&insert_record("/posts/records", 3, 42))
            .is_err()
    );
    assert!(
        transaction
            .apply(&insert_record("/users/records", 3, 42))
            .is_err()
    );
    assert!(matches!(
        transaction.commit(),
        Err(CatalogTransactionError::Invalid(_))
    ));

    let catalog = Catalog::from_path(&root).unwrap();
    assert!(
        catalog
            .open_table("posts")
            .unwrap()
            .active_record(3)
            .is_none()
    );
    assert!(
        catalog
            .open_table("users")
            .unwrap()
            .active_record(3)
            .is_none()
    );
    assert_eq!(catalog.transaction_id().unwrap(), None);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn serializable_transaction_defers_foreign_keys_until_commit() {
    let (root, catalog) = foreign_key_catalog(true, "no_action");
    let mut transaction = catalog.begin_serializable().unwrap();
    transaction
        .apply(&insert_record("/posts/records", 5, 42))
        .unwrap();
    transaction
        .apply(&insert_record("/users/records", 5, 42))
        .unwrap();
    assert_eq!(transaction.commit().unwrap(), 1);

    let catalog = Catalog::from_path(&root).unwrap();
    assert!(
        catalog
            .open_table("posts")
            .unwrap()
            .active_record(3)
            .is_some()
    );
    assert!(
        catalog
            .open_table("users")
            .unwrap()
            .active_record(3)
            .is_some()
    );
    fs::remove_dir_all(root).unwrap();
}
