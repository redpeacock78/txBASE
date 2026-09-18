use super::*;
use std::fs;

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
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(&wal_path);
    let _ = fs::remove_file(&lock_path);
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
}
