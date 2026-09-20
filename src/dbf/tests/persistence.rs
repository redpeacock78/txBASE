use super::super::*;
use super::fixture;
use crate::index::{IndexDefinition, IndexFile};
use fs2::FileExt;
use std::fs::OpenOptions;

#[test]
fn wal_commit_ids_persist_and_resume_after_reload() {
    let path = std::env::temp_dir().join(format!(
        "txbase-transaction-id-reload-{}.dbf",
        std::process::id()
    ));
    let state_path = path.with_extension("txbase.state");
    let wal_path = path.with_extension("txbase.wal");
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(&state_path);
    let _ = fs::remove_file(&wal_path);
    fs::write(&path, fixture()).unwrap();

    let mut first = DbfTable::from_path(&path).unwrap();
    first
        .patch_record(
            1,
            serde_json::json!({"AGE": 30}).as_object().unwrap().clone(),
        )
        .unwrap();
    first.save_with_wal(&path).unwrap();
    assert_eq!(first.transaction_id(), Some(1));

    let mut second = DbfTable::from_path(&path).unwrap();
    assert_eq!(second.transaction_id(), Some(1));
    second
        .patch_record(
            1,
            serde_json::json!({"AGE": 31}).as_object().unwrap().clone(),
        )
        .unwrap();
    second.save_with_wal(&path).unwrap();
    assert_eq!(second.transaction_id(), Some(2));
    assert_eq!(
        DbfTable::from_path(&path).unwrap().transaction_id(),
        Some(2)
    );

    fs::remove_file(path).unwrap();
    fs::remove_file(state_path).unwrap();
}

#[test]
fn snapshot_recovery_persists_the_wal_transaction_id() {
    let path = std::env::temp_dir().join(format!(
        "txbase-transaction-id-recovery-{}.dbf",
        std::process::id()
    ));
    let state_path = path.with_extension("txbase.state");
    let wal_path = path.with_extension("txbase.wal");
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(&state_path);
    let _ = fs::remove_file(&wal_path);
    fs::write(&path, fixture()).unwrap();

    let mut pending = DbfTable::from_bytes(&fixture()).unwrap();
    pending
        .patch_record(
            1,
            serde_json::json!({"AGE": 32}).as_object().unwrap().clone(),
        )
        .unwrap();
    let mut wal = FileWal::open(&wal_path).unwrap();
    wal.append(&transaction_id_payload(7)).unwrap();
    wal.append(&snapshot_payload(&pending.to_bytes())).unwrap();
    wal.sync().unwrap();
    drop(wal);

    let recovered = DbfTable::from_path(&path).unwrap();
    assert_eq!(recovered.transaction_id(), Some(7));
    assert!(!wal_path.exists());
    assert!(state_path.exists());

    fs::remove_file(path).unwrap();
    fs::remove_file(state_path).unwrap();
}

#[test]
fn rejects_stale_dbf_before_save() {
    let path = std::env::temp_dir().join(format!("txbase-stale-save-{}.dbf", std::process::id()));
    let wal_path = path.with_extension("txbase.wal");
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(&wal_path);

    let original = fixture();
    fs::write(&path, &original).unwrap();
    let mut table = DbfTable::from_path(&path).unwrap();
    table
        .patch_record(
            1,
            serde_json::json!({"AGE": 31}).as_object().unwrap().clone(),
        )
        .unwrap();

    let mut external = original;
    let record_start = usize::from(u16::from_le_bytes([external[8], external[9]]));
    external[record_start + 14..record_start + 17].copy_from_slice(b" 30");
    fs::write(&path, &external).unwrap();

    let error = table.save_with_wal(&path).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("DBF changed since the table was loaded")
    );
    assert_eq!(fs::read(&path).unwrap(), external);
    assert!(!wal_path.exists());

    fs::remove_file(path).unwrap();
}

#[test]
fn table_lock_serializes_file_handles() {
    let path = std::env::temp_dir().join(format!("txbase-table-lock-{}.dbf", std::process::id()));
    let lock_path = path.with_extension("txbase.lock");
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(&lock_path);

    let lock = super::lock::TableLock::acquire(&path).unwrap();
    let probe = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)
        .unwrap();
    assert!(probe.try_lock_exclusive().is_err());

    drop(lock);
    probe.try_lock_exclusive().unwrap();
    fs2::FileExt::unlock(&probe).unwrap();
    fs::remove_file(lock_path).unwrap();
}

#[test]
fn rejects_stale_memo_sidecar_before_save() {
    let path = std::env::temp_dir().join(format!("txbase-stale-memo-{}.dbf", std::process::id()));
    let memo_path = path.with_extension("dbt");
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(&memo_path);

    let mut bytes = fixture();
    bytes[0] = 0x83;
    bytes[64 + 11] = b'M';
    let record_start = usize::from(u16::from_le_bytes([bytes[8], bytes[9]]));
    bytes[record_start + 4..record_start + 14].copy_from_slice(b"         1");
    fs::write(&path, &bytes).unwrap();

    let mut memo = vec![0; DBT_BLOCK_SIZE * 2];
    memo[DBT_BLOCK_SIZE..DBT_BLOCK_SIZE + 4].copy_from_slice(b"memo");
    memo[DBT_BLOCK_SIZE + 4] = EOF_MARKER;
    fs::write(&memo_path, &memo).unwrap();

    let mut table = DbfTable::from_path(&path).unwrap();
    table
        .patch_record(
            1,
            serde_json::json!({"AGE": 31}).as_object().unwrap().clone(),
        )
        .unwrap();
    memo[DBT_BLOCK_SIZE] = b'X';
    fs::write(&memo_path, memo).unwrap();

    let error = table.save_with_wal(&path).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("memo sidecar changed since the table was loaded")
    );

    fs::remove_file(path).unwrap();
    fs::remove_file(memo_path).unwrap();
}

#[test]
fn rejects_an_invalid_index_before_saving_the_dbf() {
    let path = std::env::temp_dir().join(format!(
        "txbase-invalid-index-save-{}.dbf",
        std::process::id()
    ));
    let index_path = path.with_extension("txidx");
    let wal_path = path.with_extension("txbase.wal");
    let lock_path = path.with_extension("txbase.lock");
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(&index_path);
    let _ = fs::remove_file(&wal_path);
    let _ = fs::remove_file(&lock_path);

    let original = fixture();
    fs::write(&path, &original).unwrap();
    fs::write(&index_path, b"{}").unwrap();
    let mut table = DbfTable::from_path(&path).unwrap();
    table
        .patch_record(
            1,
            serde_json::json!({"AGE": 31}).as_object().unwrap().clone(),
        )
        .unwrap();

    let error = table.save_with_wal(&path).unwrap_err();
    assert!(error.to_string().contains("index sidecar error"));
    assert_eq!(fs::read(&path).unwrap(), original);
    assert!(!wal_path.exists());

    fs::remove_file(path).unwrap();
    fs::remove_file(index_path).unwrap();
    fs::remove_file(lock_path).unwrap();
}
