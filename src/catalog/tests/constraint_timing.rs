use super::*;
use crate::ConstraintMode;
use crate::catalog::CatalogTransactionError;
use crate::xbase::{OperationIr, OperationMethod};
use serde_json::json;
use std::fs;

fn parent_schema() -> Vec<u8> {
    serde_json::to_vec(&json!({
        "format": "txbase-schema",
        "version": 1,
        "fields": {"ID": {"primary": true}}
    }))
    .unwrap()
}

fn child_schema(deferred: bool) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "format": "txbase-schema",
        "version": 2,
        "fields": {
            "ID": {
                "references": "users.ID",
                "constraint_name": "posts_user_fk",
                "deferrable": true,
                "deferred": deferred
            }
        }
    }))
    .unwrap()
}

fn composite_parent_schema() -> Vec<u8> {
    serde_json::to_vec(&json!({
        "format": "txbase-schema",
        "version": 1,
        "fields": {},
        "constraints": {"unique": [["ID", "AGE"]]}
    }))
    .unwrap()
}

fn composite_child_schema() -> Vec<u8> {
    serde_json::to_vec(&json!({
        "format": "txbase-schema",
        "version": 2,
        "fields": {},
        "constraints": {
            "foreign_keys": [{
                "name": "posts_user_fk",
                "fields": ["ID", "AGE"],
                "references": {"table": "users", "fields": ["ID", "AGE"]},
                "deferrable": true
            }]
        }
    }))
    .unwrap()
}

fn insert(table: &str, id: i64) -> OperationIr {
    OperationIr {
        method: OperationMethod::Post,
        path: format!("/{table}/records"),
        body: Some(json!({
            "ID": id,
            "NAME": format!("record-{id}"),
            "AGE": 42,
            "ACTIVE": true
        })),
    }
}

fn foreign_key_catalog(deferred: bool) -> (PathBuf, Catalog) {
    let root = temporary_catalog();
    fs::write(root.join("users.dbf"), fixture()).unwrap();
    fs::write(root.join("users.txschema.json"), parent_schema()).unwrap();
    fs::write(root.join("posts.dbf"), fixture()).unwrap();
    fs::write(root.join("posts.txschema.json"), child_schema(deferred)).unwrap();
    let catalog = Catalog::from_path(&root).unwrap();
    (root, catalog)
}

#[test]
fn set_all_constraints_defers_then_validates_catalog_foreign_keys() {
    let (root, catalog) = foreign_key_catalog(false);
    let mut transaction = catalog.begin_serializable().unwrap();

    transaction
        .set_all_constraints(ConstraintMode::Deferred)
        .unwrap();
    transaction.apply(&insert("posts", 3)).unwrap();
    transaction.apply(&insert("users", 3)).unwrap();
    transaction
        .set_all_constraints(ConstraintMode::Immediate)
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

#[test]
fn switching_a_deferred_foreign_key_to_immediate_rejects_orphans_atomically() {
    let (root, catalog) = foreign_key_catalog(true);
    let mut transaction = catalog.begin_serializable().unwrap();

    transaction.apply(&insert("posts", 3)).unwrap();
    assert!(
        transaction
            .set_constraints("posts", &["posts_user_fk"], ConstraintMode::Immediate)
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
    assert_eq!(catalog.transaction_id().unwrap(), None);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn named_composite_foreign_key_can_be_deferred_and_checked_before_commit() {
    let root = temporary_catalog();
    fs::write(root.join("users.dbf"), fixture()).unwrap();
    fs::write(root.join("users.txschema.json"), composite_parent_schema()).unwrap();
    fs::write(root.join("posts.dbf"), fixture()).unwrap();
    fs::write(root.join("posts.txschema.json"), composite_child_schema()).unwrap();
    let catalog = Catalog::from_path(&root).unwrap();
    let mut transaction = catalog.begin_serializable().unwrap();

    transaction
        .set_constraints("posts", &["posts_user_fk"], ConstraintMode::Deferred)
        .unwrap();
    transaction.apply(&insert("posts", 3)).unwrap();
    transaction.apply(&insert("users", 3)).unwrap();
    transaction
        .set_constraints("posts", &["posts_user_fk"], ConstraintMode::Immediate)
        .unwrap();
    transaction.commit().unwrap();

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
