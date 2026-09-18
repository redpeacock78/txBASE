use super::*;
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
};

fn fixture() -> Vec<u8> {
    include_str!("../../tests/fixtures/users.dbf.hex")
        .split_whitespace()
        .map(|token| u8::from_str_radix(token, 16).unwrap())
        .collect()
}

fn write_wal_with_torn_tail(path: &Path, payload: &[u8]) {
    let mut wal = FileWal::open(path).unwrap();
    wal.append(payload).unwrap();
    wal.sync().unwrap();
    drop(wal);

    let mut file = OpenOptions::new().append(true).open(path).unwrap();
    file.write_all(b"TXWL").unwrap();
    file.sync_all().unwrap();
}

#[test]
fn recovers_snapshot_before_a_torn_wal_tail() {
    let path =
        std::env::temp_dir().join(format!("txbase-torn-recovery-{}.dbf", std::process::id()));
    let wal_path = path.with_extension("txbase.wal");
    let lock_path = path.with_extension("txbase.lock");
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(&wal_path);
    let _ = fs::remove_file(&lock_path);
    fs::write(&path, fixture()).unwrap();

    let mut pending = DbfTable::from_path(&path).unwrap();
    pending
        .patch_record(
            1,
            serde_json::json!({"NAME": "Recovered"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();
    let mut payload = SNAPSHOT_MAGIC.to_vec();
    payload.extend_from_slice(&pending.to_bytes());

    write_wal_with_torn_tail(&wal_path, &payload);

    let recovered = DbfTable::from_path(&path).unwrap();
    assert_eq!(
        recovered.active_record(1).unwrap().values["NAME"],
        "Recovered"
    );
    assert!(!wal_path.exists());

    fs::remove_file(path).unwrap();
    fs::remove_file(lock_path).unwrap();
}

#[test]
fn recovers_delta_before_a_torn_wal_tail() {
    let path = std::env::temp_dir().join(format!(
        "txbase-torn-delta-recovery-{}.dbf",
        std::process::id()
    ));
    let wal_path = path.with_extension("txbase.wal");
    let lock_path = path.with_extension("txbase.lock");
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(&wal_path);
    let _ = fs::remove_file(&lock_path);
    fs::write(&path, fixture()).unwrap();

    let mut pending = DbfTable::from_path(&path).unwrap();
    pending
        .patch_record(
            1,
            serde_json::json!({"NAME": "Delta"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();
    let full_payload = snapshot_payload(&pending.to_bytes());
    let payload = delta_payload(&pending, &path, None, full_payload.len())
        .unwrap()
        .expect("path-loaded mutation should fit in a delta");
    assert!(payload.starts_with(DELTA_MAGIC));
    write_wal_with_torn_tail(&wal_path, &payload);

    let recovered = DbfTable::from_path(&path).unwrap();
    assert_eq!(recovered.active_record(1).unwrap().values["NAME"], "Delta");
    assert!(!wal_path.exists());

    fs::remove_file(path).unwrap();
    fs::remove_file(lock_path).unwrap();
}
