use super::*;
use std::collections::BTreeMap;

#[test]
fn catalog_history_round_trips_binary_table_images() {
    let snapshot = Snapshot {
        transaction_id: 1,
        tables: [(
            "users".into(),
            TableSnapshot {
                file_name: "users.dbf".into(),
                dbf: b"dbf".to_vec(),
                memo: Some((2, b"memo".to_vec())),
                schema: Some(b"schema".to_vec()),
            },
        )]
        .into_iter()
        .collect(),
    };

    let bytes = append_snapshot(None, snapshot.clone()).unwrap();
    assert_eq!(decode(&bytes).unwrap(), vec![snapshot]);
}

#[test]
fn catalog_history_rejects_trailing_bytes_and_non_monotonic_ids() {
    let first = Snapshot {
        transaction_id: 1,
        tables: BTreeMap::new(),
    };
    let bytes = append_snapshot(None, first.clone()).unwrap();
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert!(decode(&trailing).is_err());

    assert!(append_snapshot(Some(&bytes), first).is_err());
}

#[test]
fn catalog_history_rejects_invalid_table_filenames() {
    let snapshot = Snapshot {
        transaction_id: 1,
        tables: [(
            "users".into(),
            TableSnapshot {
                file_name: "../users.dbf".into(),
                dbf: Vec::new(),
                memo: None,
                schema: None,
            },
        )]
        .into_iter()
        .collect(),
    };

    let bytes = encode(&[snapshot]).unwrap();
    assert!(decode(&bytes).is_err());
}
