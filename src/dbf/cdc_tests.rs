use super::cdc::{self, ChangeEvent, ChangeRecord, ChangeState};
use super::wal::{snapshot_payload, transaction_id_payload};
use super::{DbfFieldSpec, DbfTable};
use crate::transaction::{FileWal, Wal};
use serde_json::json;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

fn path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("txbase-cdc-{name}-{}.dbf", std::process::id()))
}

fn cleanup(path: &Path) {
    for extension in [
        "txbase.cdc",
        "txbase.state",
        "txbase.wal",
        "txbase.mvcc",
        "txbase.lock",
    ] {
        let _ = fs::remove_file(path.with_extension(extension));
    }
    let _ = fs::remove_file(path);
}

fn empty_table(path: &Path) {
    cleanup(path);
    DbfTable::empty(&[
        DbfFieldSpec::new("ID", b'N', 4, 0),
        DbfFieldSpec::new("NAME", b'C', 16, 0),
    ])
    .unwrap()
    .save_to(path)
    .unwrap();
}

#[test]
fn cdc_records_initial_rows_when_wal_creates_a_new_path() {
    let path = path("initial");
    cleanup(&path);
    let mut table = DbfTable::empty(&[DbfFieldSpec::new("ID", b'N', 4, 0)]).unwrap();
    table
        .insert_record(json!({"ID": 1}).as_object().unwrap().clone())
        .unwrap();
    table.save_with_wal(&path).unwrap();

    let events = DbfTable::cdc_events(&path, None).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].changes[0].before, None);
    assert_eq!(
        events[0].changes[0].after.as_ref().unwrap().values["ID"],
        json!(1)
    );

    cleanup(&path);
}

#[test]
fn cdc_records_committed_row_diffs_in_transaction_order() {
    let path = path("ordered");
    empty_table(&path);

    let mut table = DbfTable::from_path(&path).unwrap();
    table
        .insert_record(
            json!({"ID": 1, "NAME": "Alice"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();
    table.save_with_wal(&path).unwrap();

    let mut table = DbfTable::from_path(&path).unwrap();
    table
        .patch_record(1, json!({"NAME": "Bob"}).as_object().unwrap().clone())
        .unwrap();
    table.save_with_wal(&path).unwrap();

    let mut table = DbfTable::from_path(&path).unwrap();
    table.delete_record(1).unwrap();
    table.save_with_wal(&path).unwrap();

    let events = DbfTable::cdc_events(&path, None).unwrap();
    assert_eq!(
        events
            .iter()
            .map(|event| event.transaction_id)
            .collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    assert_eq!(events[0].changes[0].before, None);
    assert_eq!(
        events[0].changes[0].after.as_ref().unwrap().values["NAME"],
        json!("Alice")
    );
    assert_eq!(
        events[1].changes[0].before.as_ref().unwrap().values["NAME"],
        json!("Alice")
    );
    assert_eq!(
        events[1].changes[0].after.as_ref().unwrap().values["NAME"],
        json!("Bob")
    );
    assert!(events[2].changes[0].after.as_ref().unwrap().deleted);
    assert_eq!(
        DbfTable::cdc_events(&path, Some(1))
            .unwrap()
            .iter()
            .map(|event| event.transaction_id)
            .collect::<Vec<_>>(),
        vec![2, 3]
    );

    cleanup(&path);
}

#[test]
fn recovery_publishes_a_pending_cdc_event_after_snapshot_replacement() {
    let path = path("recovery");
    empty_table(&path);
    let before = DbfTable::from_path(&path).unwrap();
    let mut after = before.clone();
    after
        .insert_record(
            json!({"ID": 1, "NAME": "Recovered"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();
    let event = cdc::event_for_tables(1, &before, &after, false).unwrap();
    let wal_path = path.with_extension("txbase.wal");
    let mut wal = FileWal::open(&wal_path).unwrap();
    wal.append(&transaction_id_payload(1)).unwrap();
    wal.append(&cdc::payload(&event).unwrap()).unwrap();
    wal.append(&snapshot_payload(&after.to_bytes())).unwrap();
    wal.sync().unwrap();
    drop(wal);

    let recovered = DbfTable::from_path(&path).unwrap();
    assert_eq!(
        recovered.active_json(),
        vec![json!({"ID": 1, "NAME": "Recovered"})]
    );
    assert_eq!(DbfTable::cdc_events(&path, None).unwrap(), vec![event]);
    assert!(!wal_path.exists());

    cleanup(&path);
}

#[test]
fn cdc_append_is_idempotent_and_rejects_conflicting_reuse() {
    let path = path("idempotent");
    cleanup(&path);
    let event = ChangeEvent {
        transaction_id: 7,
        reset: false,
        changes: vec![ChangeRecord {
            record_number: 1,
            before: None,
            after: Some(ChangeState {
                deleted: false,
                values: json!({"ID": 1}).as_object().unwrap().clone(),
            }),
        }],
    };
    cdc::append(&path, &event).unwrap();
    cdc::append(&path, &event).unwrap();
    assert_eq!(cdc::read(&path, None).unwrap(), vec![event.clone()]);

    let mut conflicting = event;
    conflicting.changes[0]
        .after
        .as_mut()
        .unwrap()
        .values
        .insert("ID".into(), json!(2));
    assert!(cdc::append(&path, &conflicting).is_err());

    cleanup(&path);
}

#[test]
fn cdc_reader_repairs_a_torn_final_record() {
    let path = path("torn");
    cleanup(&path);
    let event = ChangeEvent {
        transaction_id: 1,
        reset: false,
        changes: Vec::new(),
    };
    cdc::append(&path, &event).unwrap();
    let cdc_path = cdc::path_for(&path);
    let valid_length = fs::metadata(&cdc_path).unwrap().len();
    let mut file = OpenOptions::new().append(true).open(&cdc_path).unwrap();
    file.write_all(b"TXWL").unwrap();
    file.sync_all().unwrap();
    drop(file);

    assert_eq!(cdc::read(&path, None).unwrap(), vec![event]);
    assert_eq!(fs::metadata(&cdc_path).unwrap().len(), valid_length);

    cleanup(&path);
}
