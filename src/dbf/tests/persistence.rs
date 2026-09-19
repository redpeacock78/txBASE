use super::super::*;
use super::fixture;
use crate::index::{IndexDefinition, IndexFile};
use crate::xbase::{OperationIr, OperationMethod};
use fs2::FileExt;
use std::fs::OpenOptions;

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
fn recovers_latest_snapshot_from_wal_before_reading() {
    let path = std::env::temp_dir().join(format!("txbase-dbf-recovery-{}.dbf", std::process::id()));
    let wal_path = path.with_extension("txbase.wal");
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(&wal_path);

    let mut pending = DbfTable::from_bytes(&fixture()).unwrap();
    pending
        .insert_record(
            serde_json::json!({
                "ID": 3,
                "NAME": "Carol",
                "AGE": 42,
                "ACTIVE": true
            })
            .as_object()
            .unwrap()
            .clone(),
        )
        .unwrap();
    fs::write(&path, fixture()).unwrap();

    let mut wal = FileWal::open(&wal_path).unwrap();
    let mut payload = SNAPSHOT_MAGIC.to_vec();
    payload.extend_from_slice(&pending.to_bytes());
    wal.append(&payload).unwrap();
    wal.sync().unwrap();
    drop(wal);

    let recovered = DbfTable::from_path(&path).unwrap();
    assert_eq!(recovered.active_record(3).unwrap().values["NAME"], "Carol");
    assert!(!wal_path.exists());

    fs::remove_file(path).unwrap();
}

#[test]
fn recovery_refreshes_an_existing_index_sidecar() {
    let path =
        std::env::temp_dir().join(format!("txbase-index-recovery-{}.dbf", std::process::id()));
    let wal_path = path.with_extension("txbase.wal");
    let index_path = path.with_extension("txidx");
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(&wal_path);
    let _ = fs::remove_file(&index_path);

    let original = fixture();
    let mut pending = DbfTable::from_bytes(&original).unwrap();
    pending
        .insert_record(
            serde_json::json!({
                "ID": 3,
                "NAME": "Carol",
                "AGE": 42,
                "ACTIVE": true
            })
            .as_object()
            .unwrap()
            .clone(),
        )
        .unwrap();
    fs::write(&path, &original).unwrap();
    IndexFile::build(&path, vec![IndexDefinition::for_field("NAME")])
        .unwrap()
        .save(&path)
        .unwrap();

    let mut wal = FileWal::open(&wal_path).unwrap();
    let mut payload = SNAPSHOT_MAGIC.to_vec();
    payload.extend_from_slice(&pending.to_bytes());
    wal.append(&payload).unwrap();
    wal.sync().unwrap();
    drop(wal);

    let recovered = DbfTable::from_path(&path).unwrap();
    assert_eq!(recovered.active_record(3).unwrap().values["NAME"], "Carol");
    assert_eq!(
        IndexFile::load(&path)
            .unwrap()
            .lookup_eq("NAME", &serde_json::json!("Carol"))
            .unwrap(),
        vec![3]
    );
    assert!(!wal_path.exists());

    fs::remove_file(path).unwrap();
    fs::remove_file(index_path).unwrap();
}

#[test]
fn recovery_replays_the_dbf_and_index_target_from_one_wal() {
    let path = std::env::temp_dir().join(format!(
        "txbase-index-wal-bundle-{}.dbf",
        std::process::id()
    ));
    let wal_path = path.with_extension("txbase.wal");
    let index_path = path.with_extension("txidx");
    let lock_path = path.with_extension("txbase.lock");
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(&wal_path);
    let _ = fs::remove_file(&index_path);
    let _ = fs::remove_file(&lock_path);

    let original = fixture();
    fs::write(&path, &original).unwrap();
    IndexFile::build(&path, vec![IndexDefinition::for_field("NAME")])
        .unwrap()
        .save(&path)
        .unwrap();

    let mut pending = DbfTable::from_bytes(&original).unwrap();
    pending
        .insert_record(
            serde_json::json!({
                "ID": 3,
                "NAME": "Carol",
                "AGE": 42,
                "ACTIVE": true
            })
            .as_object()
            .unwrap()
            .clone(),
        )
        .unwrap();
    let index_payload =
        crate::index::pending_snapshot_payload(&path, &pending, &pending.to_bytes(), None)
            .unwrap()
            .expect("existing index should produce a target payload");

    let mut wal = FileWal::open(&wal_path).unwrap();
    wal.append(&snapshot_payload(&pending.to_bytes())).unwrap();
    wal.append(&index_payload).unwrap();
    wal.sync().unwrap();
    drop(wal);

    fs::write(&path, pending.to_bytes()).unwrap();
    let recovered = DbfTable::from_path(&path).unwrap();
    assert_eq!(recovered.active_record(3).unwrap().values["NAME"], "Carol");
    assert_eq!(
        IndexFile::load(&path)
            .unwrap()
            .lookup_eq("NAME", &serde_json::json!("Carol"))
            .unwrap(),
        vec![3]
    );
    assert!(!wal_path.exists());

    fs::remove_file(path).unwrap();
    fs::remove_file(index_path).unwrap();
    fs::remove_file(lock_path).unwrap();
}

#[test]
fn recovers_dbf_delta_from_wal_before_reading() {
    let path = std::env::temp_dir().join(format!(
        "txbase-dbf-delta-recovery-{}.dbf",
        std::process::id()
    ));
    let wal_path = path.with_extension("txbase.wal");
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(&wal_path);

    fs::write(&path, fixture()).unwrap();
    let mut table = DbfTable::from_path(&path).unwrap();
    table
        .patch_record(
            1,
            serde_json::json!({"NAME": "Delta"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();
    let full_payload = snapshot_payload(&table.to_bytes());
    let payload = delta_payload(&table, &path, None, full_payload.len())
        .unwrap()
        .expect("path-loaded mutation should fit in a delta");
    assert!(payload.starts_with(DELTA_MAGIC));
    assert!(payload.len() < full_payload.len());

    let mut wal = FileWal::open(&wal_path).unwrap();
    wal.append(&payload).unwrap();
    wal.sync().unwrap();
    drop(wal);

    let recovered = DbfTable::from_path(&path).unwrap();
    assert_eq!(recovered.active_record(1).unwrap().values["NAME"], "Delta");
    assert!(!wal_path.exists());

    fs::remove_file(path).unwrap();
}

#[test]
fn replays_operation_intent_when_state_payload_is_missing() {
    let path = std::env::temp_dir().join(format!(
        "txbase-operation-recovery-{}.dbf",
        std::process::id()
    ));
    let wal_path = path.with_extension("txbase.wal");
    let lock_path = path.with_extension("txbase.lock");
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(&wal_path);
    let _ = fs::remove_file(&lock_path);
    fs::write(&path, fixture()).unwrap();

    let operation = OperationIr {
        method: OperationMethod::Patch,
        path: "/records/1".into(),
        body: Some(serde_json::json!({"$inc": {"AGE": 1}})),
    };
    let mut wal = FileWal::open(&wal_path).unwrap();
    wal.append(&operation_payload(&operation).unwrap()).unwrap();
    wal.sync().unwrap();
    drop(wal);

    let recovered = DbfTable::from_path(&path).unwrap();
    assert_eq!(recovered.active_record(1).unwrap().values["AGE"], 30);
    assert!(!wal_path.exists());

    fs::remove_file(path).unwrap();
    fs::remove_file(lock_path).unwrap();
}

#[test]
fn recovers_dbf_and_memo_from_txdm_snapshot() {
    let path = std::env::temp_dir().join(format!(
        "txbase-dbf-memo-recovery-{}.dbf",
        std::process::id()
    ));
    let memo_path = path.with_extension("dbt");
    let wal_path = path.with_extension("txbase.wal");
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(&memo_path);
    let _ = fs::remove_file(&wal_path);

    let mut bytes = fixture();
    bytes[0] = 0x83;
    bytes[64 + 11] = b'M';
    let record_start = usize::from(u16::from_le_bytes([bytes[8], bytes[9]]));
    let memo_start = record_start + 4;
    bytes[memo_start..memo_start + 10].copy_from_slice(b"         1");
    fs::write(&path, &bytes).unwrap();

    let mut memo_bytes = vec![0; DBT_BLOCK_SIZE * 2];
    let text = b"memo before crash";
    memo_bytes[DBT_BLOCK_SIZE..DBT_BLOCK_SIZE + text.len()].copy_from_slice(text);
    memo_bytes[DBT_BLOCK_SIZE + text.len()] = EOF_MARKER;
    fs::write(&memo_path, memo_bytes).unwrap();

    let mut table = DbfTable::from_path(&path).unwrap();
    table
        .patch_record(
            1,
            serde_json::json!({"NAME": "memo after crash"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();
    let mut prepared = table.clone();
    let memo = prepared.apply_memo_updates(&path).unwrap().unwrap();
    let payload = memo_snapshot_payload(&prepared.to_bytes(), &memo).unwrap();
    let mut wal = FileWal::open(&wal_path).unwrap();
    wal.append(&payload).unwrap();
    wal.sync().unwrap();
    drop(wal);

    let recovered = DbfTable::from_path(&path).unwrap();
    assert_eq!(
        recovered.active_record(1).unwrap().values["NAME"],
        "memo after crash"
    );
    assert_eq!(
        &recovered.to_bytes()[memo_start..memo_start + 10],
        b"         2"
    );
    assert!(!wal_path.exists());

    fs::remove_file(path).unwrap();
    fs::remove_file(memo_path).unwrap();
}

#[test]
fn recovers_dbf_and_memo_delta_from_wal_before_reading() {
    let path = std::env::temp_dir().join(format!(
        "txbase-dbf-memo-delta-recovery-{}.dbf",
        std::process::id()
    ));
    let memo_path = path.with_extension("dbt");
    let wal_path = path.with_extension("txbase.wal");
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(&memo_path);
    let _ = fs::remove_file(&wal_path);

    let mut bytes = fixture();
    bytes[0] = 0x83;
    bytes[64 + 11] = b'M';
    let record_start = usize::from(u16::from_le_bytes([bytes[8], bytes[9]]));
    let memo_start = record_start + 4;
    bytes[memo_start..memo_start + 10].copy_from_slice(b"         1");
    fs::write(&path, &bytes).unwrap();

    let mut memo_bytes = vec![0; DBT_BLOCK_SIZE * 2];
    let text = b"memo before delta";
    memo_bytes[DBT_BLOCK_SIZE..DBT_BLOCK_SIZE + text.len()].copy_from_slice(text);
    memo_bytes[DBT_BLOCK_SIZE + text.len()] = EOF_MARKER;
    fs::write(&memo_path, memo_bytes).unwrap();

    let mut table = DbfTable::from_path(&path).unwrap();
    table
        .patch_record(
            1,
            serde_json::json!({"NAME": "memo after delta"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();
    let mut prepared = table.clone();
    let memo = prepared.apply_memo_updates(&path).unwrap().unwrap();
    let full_payload = memo_snapshot_payload(&prepared.to_bytes(), &memo).unwrap();
    let payload = delta_payload(&prepared, &path, Some(&memo), full_payload.len())
        .unwrap()
        .expect("path-loaded memo mutation should fit in a delta");
    assert!(payload.starts_with(DELTA_MAGIC));
    assert!(payload.len() < full_payload.len());

    let mut wal = FileWal::open(&wal_path).unwrap();
    wal.append(&payload).unwrap();
    wal.sync().unwrap();
    drop(wal);

    let recovered = DbfTable::from_path(&path).unwrap();
    assert_eq!(
        recovered.active_record(1).unwrap().values["NAME"],
        "memo after delta"
    );
    assert_eq!(
        &recovered.to_bytes()[memo_start..memo_start + 10],
        b"         2"
    );
    assert!(!wal_path.exists());

    fs::remove_file(path).unwrap();
    fs::remove_file(memo_path).unwrap();
}
