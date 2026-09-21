use super::{DbfFieldSpec, DbfTable};
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
