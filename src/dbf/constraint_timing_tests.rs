use super::schema_metadata_tests::{cleanup, fixture, temporary_path};
use super::{DbfTable, DbfTransaction};
use crate::ConstraintMode;
use crate::xbase::{OperationIr, OperationMethod};
use serde_json::json;
use std::fs;

fn schema(constraints: serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "format": "txbase-schema",
        "version": 2,
        "fields": {},
        "constraints": constraints
    }))
    .unwrap()
}

fn patch(record: usize, values: serde_json::Value) -> OperationIr {
    OperationIr {
        method: OperationMethod::Patch,
        path: format!("/records/{record}"),
        body: Some(values),
    }
}

fn write_table(path: &std::path::Path, schema: Vec<u8>) {
    cleanup(path);
    fs::write(path, fixture()).unwrap();
    fs::write(path.with_extension("txschema.json"), schema).unwrap();
}

fn write_table_with_records(path: &std::path::Path, schema: Vec<u8>) {
    cleanup(path);
    fs::write(path, fixture()).unwrap();
    let mut transaction = DbfTransaction::begin(path).unwrap();
    transaction
        .apply(&OperationIr {
            method: OperationMethod::Post,
            path: "/records".into(),
            body: Some(json!({
                "ID": 3,
                "NAME": "Bob",
                "AGE": 42,
                "ACTIVE": true
            })),
        })
        .unwrap();
    transaction.commit().unwrap();
    fs::write(path.with_extension("txschema.json"), schema).unwrap();
}

#[test]
fn initially_deferred_unique_allows_a_key_swap_before_commit() {
    let path = temporary_path();
    write_table_with_records(
        &path,
        schema(json!({
            "deferrable": [{
                "name": "users_name_unique",
                "kind": "unique",
                "fields": ["NAME"],
                "deferred": true
            }]
        })),
    );

    let mut transaction = DbfTransaction::begin(&path).unwrap();
    transaction
        .apply(&patch(1, json!({"NAME": "Bob"})))
        .unwrap();
    transaction
        .apply(&patch(3, json!({"NAME": "Alice"})))
        .unwrap();
    transaction.commit().unwrap();

    let table = DbfTable::from_path(&path).unwrap();
    assert_eq!(table.active_record(1).unwrap().values["NAME"], "Bob");
    assert_eq!(table.active_record(3).unwrap().values["NAME"], "Alice");
    cleanup(&path);
}

#[test]
fn initially_immediate_unique_can_be_deferred_and_checked_before_commit() {
    let path = temporary_path();
    write_table_with_records(
        &path,
        schema(json!({
            "deferrable": [{
                "name": "users_name_unique",
                "kind": "unique",
                "fields": ["NAME"]
            }]
        })),
    );

    let mut transaction = DbfTransaction::begin(&path).unwrap();
    assert!(
        transaction
            .apply(&patch(1, json!({"NAME": "Bob"})))
            .is_err()
    );
    transaction
        .set_constraints(&["users_name_unique"], ConstraintMode::Deferred)
        .unwrap();
    transaction
        .apply(&patch(1, json!({"NAME": "Bob"})))
        .unwrap();
    transaction
        .apply(&patch(3, json!({"NAME": "Alice"})))
        .unwrap();
    transaction
        .set_constraints(&["users_name_unique"], ConstraintMode::Immediate)
        .unwrap();
    transaction.commit().unwrap();

    let table = DbfTable::from_path(&path).unwrap();
    assert_eq!(table.active_record(1).unwrap().values["NAME"], "Bob");
    assert_eq!(table.active_record(3).unwrap().values["NAME"], "Alice");
    cleanup(&path);
}

#[test]
fn failed_immediate_transition_keeps_the_constraint_deferred() {
    let path = temporary_path();
    write_table_with_records(
        &path,
        schema(json!({
            "deferrable": [{
                "name": "users_name_unique",
                "kind": "unique",
                "fields": ["NAME"],
                "deferred": true
            }]
        })),
    );

    let mut transaction = DbfTransaction::begin(&path).unwrap();
    transaction
        .apply(&patch(1, json!({"NAME": "Bob"})))
        .unwrap();
    assert!(
        transaction
            .set_constraints(&["users_name_unique"], ConstraintMode::Immediate)
            .is_err()
    );

    transaction
        .apply(&patch(3, json!({"NAME": "Alice"})))
        .unwrap();
    transaction
        .set_constraints(&["users_name_unique"], ConstraintMode::Immediate)
        .unwrap();
    transaction.commit().unwrap();

    let table = DbfTable::from_path(&path).unwrap();
    assert_eq!(table.active_record(1).unwrap().values["NAME"], "Bob");
    assert_eq!(table.active_record(3).unwrap().values["NAME"], "Alice");
    cleanup(&path);
}

#[test]
fn deferred_check_is_enforced_at_commit_without_publishing_invalid_rows() {
    let path = temporary_path();
    write_table(
        &path,
        schema(json!({
            "deferrable": [{
                "name": "users_age_nonnegative",
                "kind": "check",
                "predicate": {"AGE": {"$gte": 0}},
                "deferred": true
            }]
        })),
    );
    let original = fs::read(&path).unwrap();
    let original_age = DbfTable::from_path(&path)
        .unwrap()
        .active_record(1)
        .unwrap()
        .values["AGE"]
        .clone();

    let mut transaction = DbfTransaction::begin(&path).unwrap();
    transaction.apply(&patch(1, json!({"AGE": -1}))).unwrap();
    assert!(transaction.commit().is_err());

    assert_eq!(fs::read(&path).unwrap(), original);
    assert_eq!(
        DbfTable::from_path(&path)
            .unwrap()
            .active_record(1)
            .unwrap()
            .values["AGE"],
        original_age
    );
    cleanup(&path);
}

#[test]
fn schema_v1_rejects_version_2_named_constraints() {
    let path = temporary_path();
    write_table(
        &path,
        serde_json::to_vec(&json!({
            "format": "txbase-schema",
            "version": 1,
            "fields": {},
            "constraints": {
                "deferrable": [{
                    "name": "users_name_unique",
                    "kind": "unique",
                    "fields": ["NAME"]
                }]
            }
        }))
        .unwrap(),
    );

    let error = DbfTable::from_path(&path).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("named deferrable constraints require schema metadata version 2")
    );
    cleanup(&path);
}

#[test]
fn schema_v1_keeps_anonymous_composite_foreign_keys_with_matching_local_fields() {
    let path = temporary_path();
    write_table(
        &path,
        serde_json::to_vec(&json!({
            "format": "txbase-schema",
            "version": 1,
            "fields": {},
            "constraints": {
                "foreign_keys": [
                    {
                        "fields": ["ID", "AGE"],
                        "references": {"table": "users", "fields": ["ID", "AGE"]},
                        "deferred": true
                    },
                    {
                        "fields": ["ID", "AGE"],
                        "references": {"table": "groups", "fields": ["ID", "AGE"]},
                        "deferred": true
                    }
                ]
            }
        }))
        .unwrap(),
    );

    let table = DbfTable::from_path(&path).unwrap();
    let foreign_keys = table.foreign_keys().unwrap();
    assert_eq!(foreign_keys.len(), 2);
    assert_ne!(foreign_keys[0].name, foreign_keys[1].name);
    cleanup(&path);
}

#[test]
fn schema_v2_rejects_duplicate_constraint_names() {
    let path = temporary_path();
    write_table(
        &path,
        serde_json::to_vec(&json!({
            "format": "txbase-schema",
            "version": 2,
            "fields": {
                "ID": {
                    "references": "users.ID",
                    "constraint_name": "users_name_unique",
                    "deferrable": true
                }
            },
            "constraints": {
                "deferrable": [{
                    "name": "users_name_unique",
                    "kind": "unique",
                    "fields": ["NAME"]
                }]
            }
        }))
        .unwrap(),
    );

    let error = DbfTable::from_path(&path).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("duplicate schema constraint name: users_name_unique")
    );
    cleanup(&path);
}

#[test]
fn deferrable_primary_key_still_rejects_null_immediately() {
    let path = temporary_path();
    write_table(
        &path,
        schema(json!({
            "deferrable": [{
                "name": "users_id_primary",
                "kind": "primary_key",
                "fields": ["ID"],
                "deferred": true
            }]
        })),
    );

    let mut transaction = DbfTransaction::begin(&path).unwrap();
    assert!(transaction.apply(&patch(1, json!({"ID": null}))).is_err());
    transaction.rollback();
    cleanup(&path);
}
