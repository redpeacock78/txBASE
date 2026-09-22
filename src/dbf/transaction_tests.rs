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
    OperationIr {
        method: OperationMethod::Patch,
        path: "/records/1".into(),
        body: Some(json!({"NAME": name})),
    }
}

#[test]
fn snapshot_transaction_queries_private_changes_and_commits_once() {
    let destination = path("commit");
    cleanup(&destination);
    fs::write(&destination, fixture()).unwrap();

    let mut transaction = DbfTransaction::begin(&destination).unwrap();
    transaction.apply(&patch("transactional")).unwrap();
    let rows = transaction
        .execute(&crate::query::parse(br#"{}"#).unwrap())
        .unwrap();
    assert_eq!(rows[0]["NAME"], "transactional");
    assert_eq!(
        DbfTable::from_path(&destination).unwrap().active_json()[0]["NAME"],
        "Alice"
    );

    let committed = transaction.commit().unwrap();
    assert_eq!(committed.transaction_id(), Some(1));
    assert_eq!(
        DbfTable::from_path(&destination).unwrap().active_json()[0]["NAME"],
        "transactional"
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
