use super::*;
use std::fs::{self, OpenOptions};

fn fixture() -> Vec<u8> {
    include_str!("../../tests/fixtures/users.dbf.hex")
        .split_whitespace()
        .map(|token| u8::from_str_radix(token, 16).unwrap())
        .collect()
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

    let mut wal = FileWal::open(&wal_path).unwrap();
    wal.append(&payload).unwrap();
    wal.sync().unwrap();
    drop(wal);

    let mut file = OpenOptions::new().append(true).open(&wal_path).unwrap();
    file.write_all(b"TXWL").unwrap();
    file.sync_all().unwrap();
    drop(file);

    let recovered = DbfTable::from_path(&path).unwrap();
    assert_eq!(
        recovered.active_record(1).unwrap().values["NAME"],
        "Recovered"
    );
    assert!(!wal_path.exists());

    fs::remove_file(path).unwrap();
    fs::remove_file(lock_path).unwrap();
}
