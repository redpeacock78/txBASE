use super::DbfTable;
use crate::dbf::DbfTransaction;
use crate::query::QueryExecutor;
use crate::xbase::{OperationIr, OperationMethod};
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};

fn fixture() -> Vec<u8> {
    include_str!("../../tests/fixtures/users.dbf.hex")
        .split_whitespace()
        .map(|token| u8::from_str_radix(token, 16).unwrap())
        .collect()
}

fn path(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "txbase-db-transaction-{}-{label}.dbf",
        std::process::id()
    ))
}

fn cleanup(path: &Path) {
    let targets = [
        path.to_path_buf(),
        path.with_extension("txbase.wal"),
        path.with_extension("txbase.state"),
        path.with_extension("txbase.mvcc"),
        path.with_extension("txbase.lock"),
        path.with_extension("txidx"),
        path.with_extension("txschema.json"),
    ];
    for target in targets {
        let _ = fs::remove_file(target);
    }
}

fn patch(name: &str) -> OperationIr {
    patch_record(1, name)
}

fn patch_record(number: usize, name: &str) -> OperationIr {
    OperationIr {
        method: OperationMethod::Patch,
        path: format!("/records/{number}"),
        body: Some(json!({"NAME": name})),
    }
}

fn add_third_record(path: &Path) {
    let mut table = DbfTable::from_path(path).unwrap();
    table
        .insert_record(
            json!({
                "ID": 3,
                "NAME": "Carol",
                "AGE": 42,
                "ACTIVE": true
            })
            .as_object()
            .unwrap()
            .clone(),
        )
        .unwrap();
    table.save_with_wal(path).unwrap();
}

#[test]
fn snapshot_transaction_queries_private_changes_and_commits_once() {
    let destination = path("commit");
    cleanup(&destination);
    fs::write(&destination, fixture()).unwrap();

    let mut transaction = DbfTransaction::begin(&destination).unwrap();
    transaction.apply(&patch("committed")).unwrap();
    let rows = transaction
        .execute(&crate::query::parse(br#"{}"#).unwrap())
        .unwrap();
    assert_eq!(rows[0]["NAME"], "committed");
    assert_eq!(
        DbfTable::from_path(&destination).unwrap().active_json()[0]["NAME"],
        "Alice"
    );

    let committed = transaction.commit().unwrap();
    assert_eq!(committed.transaction_id(), Some(1));
    assert_eq!(
        DbfTable::from_path(&destination).unwrap().active_json()[0]["NAME"],
        "committed"
    );
    cleanup(&destination);
}

#[test]
fn snapshot_transaction_rejects_a_concurrent_commit_without_overwriting_it() {
    let destination = path("conflict");
    cleanup(&destination);
    fs::write(&destination, fixture()).unwrap();

    let mut transaction = DbfTransaction::begin(&destination).unwrap();
    let mut concurrent = DbfTable::from_path(&destination).unwrap();
    concurrent
        .patch_record(
            1,
            json!({"NAME": "concurrent"}).as_object().unwrap().clone(),
        )
        .unwrap();
    concurrent.save_with_wal(&destination).unwrap();
    transaction.apply(&patch("stale")).unwrap();

    let error = transaction.commit().unwrap_err();
    assert!(
        error
            .to_string()
            .contains("DBF changed since the table was loaded")
    );
    assert_eq!(
        DbfTable::from_path(&destination).unwrap().active_json()[0]["NAME"],
        "concurrent"
    );
    cleanup(&destination);
}

#[test]
fn snapshot_transaction_merges_disjoint_row_changes_when_explicitly_requested() {
    let destination = path("row-merge");
    cleanup(&destination);
    fs::write(&destination, fixture()).unwrap();
    add_third_record(&destination);

    let mut transaction = DbfTransaction::begin(&destination).unwrap();
    transaction
        .apply(&OperationIr {
            method: OperationMethod::Patch,
            path: "/records/1".into(),
            body: Some(json!({"AGE": 31})),
        })
        .unwrap();
    let mut concurrent = DbfTransaction::begin(&destination).unwrap();
    concurrent.apply(&patch_record(3, "Bobby")).unwrap();
    concurrent.commit().unwrap();

    transaction.commit_with_row_merge().unwrap();
    let current = DbfTable::from_path(&destination).unwrap();
    assert_eq!(current.active_record(1).unwrap().values["AGE"], 31);
    assert_eq!(current.active_record(3).unwrap().values["NAME"], "Bobby");
    cleanup(&destination);
}

#[test]
fn snapshot_transaction_merges_a_disjoint_row_delete_when_explicitly_requested() {
    let destination = path("row-delete-merge");
    cleanup(&destination);
    fs::write(&destination, fixture()).unwrap();
    add_third_record(&destination);

    let mut transaction = DbfTransaction::begin(&destination).unwrap();
    transaction
        .apply(&OperationIr {
            method: OperationMethod::Delete,
            path: "/records/1".into(),
            body: None,
        })
        .unwrap();
    let mut concurrent = DbfTransaction::begin(&destination).unwrap();
    concurrent.apply(&patch_record(3, "Bobby")).unwrap();
    concurrent.commit().unwrap();

    transaction.commit_with_row_merge().unwrap();
    let current = DbfTable::from_path(&destination).unwrap();
    assert!(current.active_record(1).is_none());
    assert_eq!(current.active_record(3).unwrap().values["NAME"], "Bobby");
    cleanup(&destination);
}

#[test]
fn snapshot_transaction_rejects_an_insert_during_row_merge() {
    let destination = path("row-insert-conflict");
    cleanup(&destination);
    fs::write(&destination, fixture()).unwrap();

    let mut transaction = DbfTransaction::begin(&destination).unwrap();
    transaction.apply(&patch("stale")).unwrap();
    let mut concurrent = DbfTransaction::begin(&destination).unwrap();
    concurrent
        .apply(&OperationIr {
            method: OperationMethod::Post,
            path: "/records".into(),
            body: Some(json!({
                "ID": 3,
                "NAME": "Carol",
                "AGE": 42,
                "ACTIVE": true
            })),
        })
        .unwrap();
    concurrent.commit().unwrap();

    let error = transaction.commit_with_row_merge().unwrap_err();
    assert!(
        error
            .to_string()
            .contains("unchanged schema, layout, and record count")
    );
    assert_eq!(
        DbfTable::from_path(&destination).unwrap().records().len(),
        3
    );
    cleanup(&destination);
}

#[test]
fn snapshot_transaction_rejects_a_same_row_merge_conflict() {
    let destination = path("row-merge-conflict");
    cleanup(&destination);
    fs::write(&destination, fixture()).unwrap();

    let mut transaction = DbfTransaction::begin(&destination).unwrap();
    transaction.apply(&patch("first")).unwrap();
    let mut concurrent = DbfTransaction::begin(&destination).unwrap();
    concurrent.apply(&patch("other")).unwrap();
    concurrent.commit().unwrap();

    let error = transaction.commit_with_row_merge().unwrap_err();
    assert!(
        error
            .to_string()
            .contains("row-level merge conflict at record 1")
    );
    assert_eq!(
        DbfTable::from_path(&destination)
            .unwrap()
            .active_record(1)
            .unwrap()
            .values["NAME"],
        "other"
    );
    cleanup(&destination);
}

#[test]
fn rollback_discards_the_private_snapshot() {
    let destination = path("rollback");
    cleanup(&destination);
    fs::write(&destination, fixture()).unwrap();

    let mut transaction = DbfTransaction::begin(&destination).unwrap();
    transaction.apply(&patch("discarded")).unwrap();
    transaction.rollback();

    assert_eq!(
        DbfTable::from_path(&destination).unwrap().active_json()[0]["NAME"],
        "Alice"
    );
    cleanup(&destination);
}
