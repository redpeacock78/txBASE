use super::*;
use std::fs::{self, OpenOptions};
use std::io::Write;

#[test]
fn memory_wal_assigns_monotonic_lsn_and_syncs() {
    let mut wal = MemoryWal::default();
    assert_eq!(wal.append(b"one").unwrap(), Lsn(0));
    assert_eq!(wal.append(b"two").unwrap(), Lsn(1));
    assert!(!wal.is_synced());
    wal.sync().unwrap();
    assert!(wal.is_synced());
    assert_eq!(wal.records()[1].1, b"two");
}

#[test]
fn file_wal_reopens_and_truncates_a_torn_tail() {
    let path = std::env::temp_dir().join(format!("txbase-wal-test-{}.log", std::process::id()));
    let _ = fs::remove_file(&path);
    let valid_length;
    {
        let mut wal = FileWal::open(&path).unwrap();
        assert_eq!(wal.append(b"commit").unwrap(), Lsn(0));
        wal.sync().unwrap();
        valid_length = fs::metadata(&path).unwrap().len();
    }
    {
        let mut file = OpenOptions::new().append(true).open(&path).unwrap();
        file.write_all(&WAL_MAGIC).unwrap();
    }
    let wal = FileWal::open(&path).unwrap();
    assert_eq!(wal.records()[0].1, b"commit");
    assert_eq!(fs::metadata(&path).unwrap().len(), valid_length);
    fs::remove_file(path).unwrap();
}

#[test]
fn file_wal_inspect_reports_but_does_not_truncate_a_torn_tail() {
    let path = std::env::temp_dir().join(format!(
        "txbase-wal-inspect-test-{}.log",
        std::process::id()
    ));
    let valid_length;
    {
        let mut wal = FileWal::open(&path).unwrap();
        wal.append(b"commit").unwrap();
        wal.sync().unwrap();
        valid_length = fs::metadata(&path).unwrap().len();
    }
    {
        let mut file = OpenOptions::new().append(true).open(&path).unwrap();
        file.write_all(&WAL_MAGIC).unwrap();
    }

    let inspection = FileWal::inspect(&path).unwrap();
    assert_eq!(
        inspection.file_bytes as u64,
        valid_length + WAL_MAGIC.len() as u64
    );
    assert_eq!(inspection.valid_bytes as u64, valid_length);
    assert!(inspection.truncated_tail);
    assert_eq!(
        inspection.records,
        vec![WalRecordInfo {
            lsn: Lsn(0),
            length: 6,
        }]
    );
    assert_eq!(
        fs::metadata(&path).unwrap().len(),
        inspection.file_bytes as u64
    );
    fs::remove_file(path).unwrap();
}

#[test]
fn file_wal_inspect_does_not_create_a_missing_file() {
    let path = std::env::temp_dir().join(format!(
        "txbase-wal-inspect-missing-{}.log",
        std::process::id()
    ));
    let _ = fs::remove_file(&path);

    assert!(FileWal::inspect(&path).is_err());
    assert!(!path.exists());
}

#[test]
fn file_wal_rejects_corrupt_complete_records() {
    let cases = [
        ("magic", {
            let mut bytes = b"BAD!".to_vec();
            bytes.extend_from_slice(&0u32.to_le_bytes());
            bytes.extend_from_slice(&0u64.to_le_bytes());
            bytes
        }),
        ("size", {
            let mut bytes = WAL_MAGIC.to_vec();
            bytes.extend_from_slice(&((MAX_WAL_RECORD_SIZE as u32) + 1).to_le_bytes());
            bytes.extend_from_slice(&0u64.to_le_bytes());
            bytes
        }),
        ("lsn", {
            let mut bytes = WAL_MAGIC.to_vec();
            bytes.extend_from_slice(&0u32.to_le_bytes());
            bytes.extend_from_slice(&1u64.to_le_bytes());
            bytes
        }),
    ];

    for (name, bytes) in cases {
        let path = std::env::temp_dir().join(format!(
            "txbase-wal-corrupt-{}-{name}.log",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);
        fs::write(&path, bytes).unwrap();
        assert!(matches!(
            FileWal::open(&path),
            Err(TransactionError::Invalid(_))
        ));
        assert!(matches!(
            FileWal::inspect(&path),
            Err(TransactionError::Invalid(_))
        ));
        fs::remove_file(path).unwrap();
    }
}

#[test]
fn snapshot_engine_commits_and_rejects_reuse() {
    let mut engine = InMemoryTransactionEngine::default();
    let transaction = engine.begin(IsolationLevel::Snapshot).unwrap();
    assert_eq!(transaction.snapshot.lsn, Lsn(0));
    assert_eq!(engine.commit(transaction).unwrap(), Lsn(0));
    assert_eq!(engine.current_lsn(), Lsn(0));
    assert_eq!(engine.wal().records().len(), 1);
    assert!(engine.commit(transaction).is_err());
}

#[test]
fn snapshot_engine_rolls_back_active_transaction() {
    let mut engine = InMemoryTransactionEngine::default();
    let transaction = engine.begin(IsolationLevel::Snapshot).unwrap();
    engine.rollback(transaction).unwrap();
    assert!(engine.rollback(transaction).is_err());
    assert_eq!(engine.wal().records()[0].1[0], b'R');
}

#[test]
fn snapshot_engine_resumes_transaction_ids_from_wal() {
    let mut engine = InMemoryTransactionEngine::default();
    let first = engine.begin(IsolationLevel::Snapshot).unwrap();
    assert_eq!(first.id, TransactionId(1));
    engine.commit(first).unwrap();

    let wal = engine.into_wal();
    let mut resumed = InMemoryTransactionEngine::new(wal);
    let second = resumed.begin(IsolationLevel::Snapshot).unwrap();
    assert_eq!(second.id, TransactionId(2));
}

#[test]
fn file_snapshot_engine_resumes_transaction_ids_after_reopen() {
    let path = std::env::temp_dir().join(format!(
        "txbase-transaction-id-test-{}.wal",
        std::process::id()
    ));
    let _ = fs::remove_file(&path);
    {
        let wal = FileWal::open(&path).unwrap();
        let mut engine = FileTransactionEngine::new(wal);
        let transaction = engine.begin(IsolationLevel::Snapshot).unwrap();
        assert_eq!(transaction.id, TransactionId(1));
        engine.commit(transaction).unwrap();
    }
    {
        let wal = FileWal::open(&path).unwrap();
        let mut engine = FileTransactionEngine::new(wal);
        let transaction = engine.begin(IsolationLevel::Snapshot).unwrap();
        assert_eq!(transaction.id, TransactionId(2));
    }
    fs::remove_file(path).unwrap();
}
