use super::*;
use std::{
    fs,
    sync::{Arc, Barrier},
    thread,
};

fn fixture() -> Vec<u8> {
    include_str!("../../tests/fixtures/users.dbf.hex")
        .split_whitespace()
        .map(|token| u8::from_str_radix(token, 16).unwrap())
        .collect()
}

#[test]
fn rejects_a_stale_second_writer_without_overwriting_the_first_save() {
    let path = std::env::temp_dir().join(format!("txbase-two-writers-{}.dbf", std::process::id()));
    let wal_path = path.with_extension("txbase.wal");
    let lock_path = path.with_extension("txbase.lock");
    let state_path = path.with_extension("txbase.state");
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(&wal_path);
    let _ = fs::remove_file(&lock_path);
    let _ = fs::remove_file(&state_path);
    fs::write(&path, fixture()).unwrap();

    let mut first = DbfTable::from_path(&path).unwrap();
    let mut second = DbfTable::from_path(&path).unwrap();
    first
        .patch_record(
            1,
            serde_json::json!({"AGE": 31}).as_object().unwrap().clone(),
        )
        .unwrap();
    second
        .patch_record(
            1,
            serde_json::json!({"NAME": "Stale"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();

    first.save_with_wal(&path).unwrap();
    let error = second.save_with_wal(&path).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("DBF changed since the table was loaded")
    );

    let current = DbfTable::from_path(&path).unwrap();
    assert_eq!(current.active_record(1).unwrap().values["AGE"], 31);
    assert_eq!(current.active_record(1).unwrap().values["NAME"], "Alice");
    assert!(!wal_path.exists());

    fs::remove_file(path).unwrap();
    fs::remove_file(lock_path).unwrap();
    fs::remove_file(state_path).unwrap();
}

#[test]
fn rejects_a_stale_transaction_state_without_a_dbf_change() {
    let path = std::env::temp_dir().join(format!(
        "txbase-stale-transaction-state-{}.dbf",
        std::process::id()
    ));
    let wal_path = path.with_extension("txbase.wal");
    let lock_path = path.with_extension("txbase.lock");
    let state_path = path.with_extension("txbase.state");
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(&wal_path);
    let _ = fs::remove_file(&lock_path);
    let _ = fs::remove_file(&state_path);
    fs::write(&path, fixture()).unwrap();

    let mut current = DbfTable::from_path(&path).unwrap();
    let mut stale = DbfTable::from_path(&path).unwrap();
    current.save_with_wal(&path).unwrap();
    stale
        .patch_record(
            1,
            serde_json::json!({"AGE": 31}).as_object().unwrap().clone(),
        )
        .unwrap();

    let error = stale.save_with_wal(&path).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("transaction state changed since the table was loaded")
    );

    fs::remove_file(path).unwrap();
    fs::remove_file(lock_path).unwrap();
    fs::remove_file(state_path).unwrap();
}

#[test]
fn serializes_a_wave_of_stale_writers() {
    const WRITERS: usize = 4;
    let path = std::env::temp_dir().join(format!("txbase-writer-wave-{}.dbf", std::process::id()));
    let wal_path = path.with_extension("txbase.wal");
    let lock_path = path.with_extension("txbase.lock");
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(&wal_path);
    let _ = fs::remove_file(&lock_path);
    fs::write(&path, fixture()).unwrap();

    let barrier = Arc::new(Barrier::new(WRITERS));
    let mut handles = Vec::with_capacity(WRITERS);
    for writer in 0..WRITERS {
        let barrier = Arc::clone(&barrier);
        let path = path.clone();
        handles.push(thread::spawn(move || {
            let mut table = DbfTable::from_path(&path).unwrap();
            table
                .patch_record(
                    1,
                    serde_json::json!({"AGE": 40 + writer as i64})
                        .as_object()
                        .unwrap()
                        .clone(),
                )
                .unwrap();
            barrier.wait();
            table
                .save_with_wal(&path)
                .map_err(|error| error.to_string())
        }));
    }

    let results = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|result| {
                result.as_ref().err().is_some_and(|message| {
                    message.contains("DBF changed since the table was loaded")
                })
            })
            .count(),
        WRITERS - 1
    );

    let current = DbfTable::from_path(&path).unwrap();
    let age = current.active_record(1).unwrap().values["AGE"]
        .as_i64()
        .unwrap();
    assert!((40..40 + WRITERS as i64).contains(&age));
    assert!(!wal_path.exists());

    fs::remove_file(path).unwrap();
    fs::remove_file(lock_path).unwrap();
}
