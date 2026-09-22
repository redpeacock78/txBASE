use super::mvcc::Record;
use super::mvcc_codec::{decode_record, encode_row_history};
use super::row_mvcc::RowChange;
use super::{DbfFieldSpec, DbfTable, RowId};
use serde_json::json;
use std::fs;

#[test]
fn persistent_snapshots_keep_each_committed_table_image() {
    let path =
        std::env::temp_dir().join(format!("txbase-mvcc-snapshot-{}.dbf", std::process::id()));
    for extension in ["txbase.mvcc", "txbase.state", "txbase.wal", "txbase.lock"] {
        let _ = fs::remove_file(path.with_extension(extension));
    }
    let _ = fs::remove_file(&path);

    let table = DbfTable::empty(&[
        DbfFieldSpec::new("ID", b'N', 4, 0),
        DbfFieldSpec::new("NAME", b'C', 16, 0),
    ])
    .unwrap();
    table.save_to(&path).unwrap();

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
        .insert_record(json!({"ID": 2, "NAME": "Bob"}).as_object().unwrap().clone())
        .unwrap();
    table.save_with_wal(&path).unwrap();

    assert_eq!(DbfTable::mvcc_versions(&path).unwrap(), vec![1, 2]);
    assert_eq!(
        DbfTable::from_path_at(&path, 1).unwrap().active_json(),
        vec![json!({"ID": 1, "NAME": "Alice"})]
    );
    assert_eq!(
        DbfTable::from_path_at(&path, 2).unwrap().active_json(),
        vec![
            json!({"ID": 1, "NAME": "Alice"}),
            json!({"ID": 2, "NAME": "Bob"}),
        ]
    );
    assert!(DbfTable::from_path_at(&path, 3).is_err());

    for extension in ["txbase.mvcc", "txbase.state", "txbase.wal", "txbase.lock"] {
        let _ = fs::remove_file(path.with_extension(extension));
    }
    fs::remove_file(path).unwrap();
}

#[test]
fn gc_retains_latest_table_snapshots_and_rejects_zero() {
    let path = std::env::temp_dir().join(format!("txbase-mvcc-gc-{}.dbf", std::process::id()));
    for extension in [
        "txbase.mvcc",
        "txbase.state",
        "txbase.wal",
        "txbase.lock",
        "txbase.mvcc.gc.tmp",
    ] {
        let _ = fs::remove_file(path.with_extension(extension));
    }
    let _ = fs::remove_file(&path);

    DbfTable::empty(&[
        DbfFieldSpec::new("ID", b'N', 4, 0),
        DbfFieldSpec::new("NAME", b'C', 16, 0),
    ])
    .unwrap()
    .save_to(&path)
    .unwrap();
    for (id, name) in [(1, "Alice"), (2, "Bob"), (3, "Carol")] {
        let mut table = DbfTable::from_path(&path).unwrap();
        table
            .insert_record(json!({"ID": id, "NAME": name}).as_object().unwrap().clone())
            .unwrap();
        table.save_with_wal(&path).unwrap();
    }

    assert_eq!(DbfTable::gc_mvcc(&path, 2).unwrap(), vec![2, 3]);
    assert!(DbfTable::from_path_at(&path, 1).is_err());
    assert_eq!(
        DbfTable::from_path_at(&path, 2).unwrap().active_json(),
        vec![
            json!({"ID": 1, "NAME": "Alice"}),
            json!({"ID": 2, "NAME": "Bob"}),
        ]
    );
    assert!(DbfTable::gc_mvcc(&path, 0).is_err());

    let mut table = DbfTable::from_path(&path).unwrap();
    table
        .insert_record(
            json!({"ID": 4, "NAME": "Dave"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();
    table.save_with_wal(&path).unwrap();
    assert_eq!(DbfTable::mvcc_versions(&path).unwrap(), vec![2, 3, 4]);

    for extension in [
        "txbase.mvcc",
        "txbase.state",
        "txbase.wal",
        "txbase.lock",
        "txbase.mvcc.gc.tmp",
    ] {
        let _ = fs::remove_file(path.with_extension(extension));
    }
    fs::remove_file(path).unwrap();
}

#[test]
fn row_mvcc_retains_independent_older_row_versions_and_compacts_with_gc() {
    let path = std::env::temp_dir().join(format!(
        "txbase-mvcc-row-history-{}.dbf",
        std::process::id()
    ));
    for extension in [
        "txbase.mvcc",
        "txbase.state",
        "txbase.wal",
        "txbase.lock",
        "txbase.mvcc.gc.tmp",
    ] {
        let _ = fs::remove_file(path.with_extension(extension));
    }
    let _ = fs::remove_file(&path);

    DbfTable::empty(&[
        DbfFieldSpec::new("ID", b'N', 4, 0),
        DbfFieldSpec::new("NAME", b'C', 16, 0),
    ])
    .unwrap()
    .save_to(&path)
    .unwrap();

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

    let mut table = DbfTable::from_path(&path).unwrap();
    table.recall_record(1).unwrap();
    table.save_with_wal(&path).unwrap();

    let history = DbfTable::mvcc_row_versions(&path, 1).unwrap();
    assert_eq!(
        history
            .iter()
            .map(|version| (version.transaction_id, version.deleted))
            .collect::<Vec<_>>(),
        vec![(1, false), (2, false), (3, true), (4, false)]
    );
    let id = RowId {
        epoch: history[0].id.epoch,
        record_number: 1,
    };
    assert_eq!(
        DbfTable::mvcc_read_row(&path, 2, id)
            .unwrap()
            .unwrap()
            .values["NAME"],
        json!("Bob")
    );
    assert!(
        DbfTable::mvcc_read_row(&path, 3, id)
            .unwrap()
            .unwrap()
            .deleted
    );

    assert_eq!(
        DbfTable::gc_mvcc_with_row_retention(&path, 2, 1).unwrap(),
        vec![3, 4]
    );
    assert_eq!(
        DbfTable::mvcc_row_versions(&path, 1)
            .unwrap()
            .iter()
            .map(|version| version.transaction_id)
            .collect::<Vec<_>>(),
        vec![2, 3, 4]
    );
    assert_eq!(
        DbfTable::mvcc_read_row(&path, 2, id)
            .unwrap()
            .unwrap()
            .values["NAME"],
        json!("Bob")
    );
    assert!(DbfTable::mvcc_read_row(&path, 1, id).is_err());
    assert!(
        !DbfTable::mvcc_read_row(&path, 4, id)
            .unwrap()
            .unwrap()
            .deleted
    );

    for extension in [
        "txbase.mvcc",
        "txbase.state",
        "txbase.wal",
        "txbase.lock",
        "txbase.mvcc.gc.tmp",
    ] {
        let _ = fs::remove_file(path.with_extension(extension));
    }
    fs::remove_file(path).unwrap();
}

#[test]
fn row_mvcc_separates_physical_record_ids_after_pack() {
    let path =
        std::env::temp_dir().join(format!("txbase-mvcc-row-pack-{}.dbf", std::process::id()));
    for extension in ["txbase.mvcc", "txbase.state", "txbase.wal", "txbase.lock"] {
        let _ = fs::remove_file(path.with_extension(extension));
    }
    let _ = fs::remove_file(&path);

    DbfTable::empty(&[
        DbfFieldSpec::new("ID", b'N', 4, 0),
        DbfFieldSpec::new("NAME", b'C', 16, 0),
    ])
    .unwrap()
    .save_to(&path)
    .unwrap();
    let mut table = DbfTable::from_path(&path).unwrap();
    for (id, name) in [(1, "Alice"), (2, "Bob")] {
        table
            .insert_record(json!({"ID": id, "NAME": name}).as_object().unwrap().clone())
            .unwrap();
    }
    table.save_with_wal(&path).unwrap();

    let mut table = DbfTable::from_path(&path).unwrap();
    table.delete_record(1).unwrap();
    table.save_with_wal(&path).unwrap();

    let mut table = DbfTable::from_path(&path).unwrap();
    table.pack().unwrap();
    table.save_with_wal(&path).unwrap();

    let history = DbfTable::mvcc_row_versions(&path, 1).unwrap();
    let old_id = history
        .iter()
        .find(|version| version.transaction_id == 1)
        .unwrap()
        .id;
    let new_id = history
        .iter()
        .find(|version| version.transaction_id == 3 && !version.deleted)
        .unwrap()
        .id;
    assert!(new_id.epoch > old_id.epoch);
    assert!(
        DbfTable::mvcc_read_row(&path, 3, old_id)
            .unwrap()
            .unwrap()
            .deleted
    );
    assert_eq!(
        DbfTable::mvcc_read_row(&path, 3, new_id)
            .unwrap()
            .unwrap()
            .values["NAME"],
        json!("Bob")
    );

    for extension in ["txbase.mvcc", "txbase.state", "txbase.wal", "txbase.lock"] {
        let _ = fs::remove_file(path.with_extension(extension));
    }
    fs::remove_file(path).unwrap();
}

#[test]
fn mvcc_row_history_codec_rejects_empty_and_trailing_records() {
    let change = RowChange {
        epoch: 1,
        record_number: 1,
        deleted: false,
        values: json!({"ID": 1}).as_object().unwrap().clone(),
    };
    let payload = encode_row_history(7, std::slice::from_ref(&change)).unwrap();
    match decode_record(&payload).unwrap() {
        Record::RowHistory {
            transaction_id,
            changes,
        } => {
            assert_eq!(transaction_id, 7);
            assert_eq!(changes, vec![change]);
        }
        record => panic!("expected row history, got {record:?}"),
    }

    assert!(encode_row_history(7, &[]).is_err());

    let mut empty = payload[..18].to_vec();
    empty[14..18].copy_from_slice(&0u32.to_le_bytes());
    let error = decode_record(&empty).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("row-history record must contain a change")
    );

    let mut trailing = payload;
    trailing.push(0);
    let error = decode_record(&trailing).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("row-history record has trailing bytes")
    );
}
