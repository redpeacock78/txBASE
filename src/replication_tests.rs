use super::*;
use serde_json::json;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

fn fixture() -> Vec<u8> {
    include_str!("../tests/fixtures/users.dbf.hex")
        .split_whitespace()
        .map(|token| u8::from_str_radix(token, 16).unwrap())
        .collect()
}

fn catalog_root(label: &str) -> PathBuf {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "txbase-replication-{label}-{}-{id}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir(&root).unwrap();
    fs::write(root.join("users.dbf"), fixture()).unwrap();
    root
}

fn post(record_id: i64, name: &str) -> OperationIr {
    OperationIr {
        method: OperationMethod::Post,
        path: "/users/records".into(),
        body: Some(json!({
            "ID": record_id,
            "NAME": name,
            "AGE": 42,
            "ACTIVE": true
        })),
    }
}

#[test]
fn entries_are_versioned_and_round_trip() {
    let entry = ReplicationEntry::new(1, 1, 1, "schema-v1".into(), vec![post(3, "Carol")]).unwrap();
    entry.validate().unwrap();
    let decoded = ReplicationEntry::from_json(&entry.to_json().unwrap()).unwrap();
    assert_eq!(decoded, entry);

    let mut unknown = serde_json::to_value(entry).unwrap();
    unknown["extra"] = json!(true);
    let error = ReplicationEntry::from_json(&serde_json::to_vec(&unknown).unwrap()).unwrap_err();
    assert!(error.to_string().contains("unknown field"));

    let error = ReplicationEntry::new(
        1,
        1,
        1,
        "schema-v1".into(),
        vec![OperationIr {
            method: OperationMethod::Get,
            path: "/users/records".into(),
            body: None,
        }],
    )
    .unwrap_err();
    assert!(error.to_string().contains("read operation"));

    let oversized = vec![b' '; MAX_REPLICATION_ENTRY_BYTES + 1];
    assert!(matches!(
        ReplicationEntry::from_json(&oversized),
        Err(ReplicationError::Invalid(message))
            if message.contains("JSON exceeds")
    ));

    let oversized_entry = ReplicationEntry::new(
        1,
        1,
        1,
        "x".repeat(MAX_REPLICATION_ENTRY_BYTES),
        vec![post(3, "Carol")],
    )
    .unwrap();
    assert!(matches!(
        oversized_entry.to_json(),
        Err(ReplicationError::Invalid(message))
            if message.contains("JSON exceeds")
    ));
}

#[test]
fn log_position_and_sequence_are_validated() {
    let log = ReplicationLog::with_position(3, 4, 8).unwrap();
    assert_eq!(log.term(), 3);
    assert_eq!(log.last_index(), 4);
    assert_eq!(log.last_transaction_id(), 8);
    assert!(log.entries().is_empty());
    log.validate().unwrap();

    let entry =
        ReplicationEntry::new(3, 6, 10, "schema-v1".into(), vec![post(3, "Carol")]).unwrap();
    let mut serialized = serde_json::to_value(&log).unwrap();
    serialized["entries"] = json!([entry]);
    let error = ReplicationLog::from_json(&serde_json::to_vec(&serialized).unwrap()).unwrap_err();
    assert!(error.to_string().contains("index gap"));
}

#[test]
fn authority_and_follower_commit_the_same_atomic_entry() {
    let leader_root = catalog_root("leader");
    let follower_root = catalog_root("follower");
    let leader = Catalog::from_path(&leader_root).unwrap();
    let follower = Catalog::from_path(&follower_root).unwrap();
    let mut authority = ReplicationLog::new(1).unwrap();
    let mut replica = ReplicationLog::new(1).unwrap();

    let entry = authority.propose(&leader, vec![post(3, "Carol")]).unwrap();
    assert_eq!(
        replica.receive(&follower, entry.clone()).unwrap(),
        ApplyOutcome::Applied {
            index: 1,
            transaction_id: 1
        }
    );
    assert_eq!(
        replica.receive(&follower, entry).unwrap(),
        ApplyOutcome::Duplicate {
            index: 1,
            transaction_id: 1
        }
    );
    assert_eq!(
        leader.open_table("users").unwrap().active_record(3),
        follower.open_table("users").unwrap().active_record(3)
    );
    assert_eq!(leader.transaction_id().unwrap(), Some(1));
    assert_eq!(follower.transaction_id().unwrap(), Some(1));

    fs::remove_dir_all(leader_root).unwrap();
    fs::remove_dir_all(follower_root).unwrap();
}

#[test]
fn a_partitioned_follower_rejects_gaps_until_recovery() {
    let leader_root = catalog_root("gap-leader");
    let follower_root = catalog_root("gap-follower");
    let leader = Catalog::from_path(&leader_root).unwrap();
    let follower = Catalog::from_path(&follower_root).unwrap();
    let mut authority = ReplicationLog::new(1).unwrap();
    let mut replica = ReplicationLog::new(1).unwrap();
    let first = authority.propose(&leader, vec![post(3, "Carol")]).unwrap();
    let second = authority.propose(&leader, vec![post(4, "Dave")]).unwrap();

    assert!(matches!(
        replica.receive(&follower, second.clone()),
        Err(ReplicationError::IndexGap {
            expected: 1,
            actual: 2
        })
    ));
    assert!(matches!(
        replica.receive(&follower, first),
        Ok(ApplyOutcome::Applied { index: 1, .. })
    ));
    assert!(matches!(
        replica.receive(&follower, second),
        Ok(ApplyOutcome::Applied { index: 2, .. })
    ));
    assert!(
        follower
            .open_table("users")
            .unwrap()
            .active_record(4)
            .is_some()
    );

    fs::remove_dir_all(leader_root).unwrap();
    fs::remove_dir_all(follower_root).unwrap();
}

#[test]
fn serialized_log_can_resume_after_restart() {
    let root = catalog_root("restart");
    let catalog = Catalog::from_path(&root).unwrap();
    let mut log = ReplicationLog::new(7).unwrap();
    log.propose(&catalog, vec![post(3, "Carol")]).unwrap();
    let recovered = ReplicationLog::from_json(&log.to_json().unwrap()).unwrap();
    assert_eq!(recovered.last_index(), 1);
    assert_eq!(recovered.last_transaction_id(), 1);

    let mut recovered = recovered;
    let entry = recovered.propose(&catalog, vec![post(4, "Dave")]).unwrap();
    assert_eq!(entry.term, 7);
    assert_eq!(entry.index, 2);
    assert_eq!(entry.transaction_id, 2);
    assert_eq!(catalog.transaction_id().unwrap(), Some(2));

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn txrp_sidecar_reopens_and_continues_with_the_catalog() {
    let root = catalog_root("durable");
    let catalog = Catalog::from_path(&root).unwrap();
    let mut log = ReplicationLog::open(&catalog, 7).unwrap();
    log.propose(&catalog, vec![post(3, "Carol")]).unwrap();

    let sidecar = fs::read(root.join(REPLICATION_SIDECAR_NAME)).unwrap();
    assert!(sidecar.starts_with(b"TXRP\x01"));
    assert_eq!(ReplicationLog::from_sidecar_bytes(&sidecar).unwrap(), log);

    let reopened_catalog = Catalog::from_path(&root).unwrap();
    let mut reopened = ReplicationLog::open(&reopened_catalog, 7).unwrap();
    let entry = reopened
        .propose(&reopened_catalog, vec![post(4, "Dave")])
        .unwrap();
    assert_eq!(entry.index, 2);
    assert_eq!(entry.transaction_id, 2);
    assert_eq!(reopened_catalog.transaction_id().unwrap(), Some(2));
    assert_eq!(reopened.last_index(), 2);

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn stale_txrp_sidecar_rejects_a_new_catalog_commit() {
    let root = catalog_root("sidecar-precondition");
    let catalog = Catalog::from_path(&root).unwrap();
    let mut log = ReplicationLog::open(&catalog, 1).unwrap();
    log.propose(&catalog, vec![post(3, "Carol")]).unwrap();

    let stale = ReplicationLog::new(1).unwrap().to_sidecar_bytes().unwrap();
    fs::write(root.join(REPLICATION_SIDECAR_NAME), stale).unwrap();
    assert!(matches!(
        log.propose(&catalog, vec![post(4, "Dave")]),
        Err(ReplicationError::SidecarStateMismatch)
    ));
    assert_eq!(catalog.transaction_id().unwrap(), Some(1));
    assert!(
        catalog
            .open_table("users")
            .unwrap()
            .active_record(4)
            .is_none()
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn malformed_txrp_sidecar_is_rejected_before_replay() {
    let root = catalog_root("malformed-sidecar");
    fs::write(root.join(REPLICATION_SIDECAR_NAME), b"not-txrp").unwrap();
    let catalog = Catalog::from_path(&root).unwrap();

    let error = ReplicationLog::open(&catalog, 1).unwrap_err();
    assert!(error.to_string().contains("sidecar header is invalid"));

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn local_follower_reads_are_limited_to_the_applied_log() {
    let root = catalog_root("follower-read");
    let catalog = Catalog::from_path(&root).unwrap();
    let mut log = ReplicationLog::open(&catalog, 1).unwrap();
    log.propose(&catalog, vec![post(3, "Carol")]).unwrap();
    log.propose(&catalog, vec![post(4, "Dave")]).unwrap();

    let first = log.read_at(&catalog, 1).unwrap();
    assert!(
        first
            .open_table("users")
            .unwrap()
            .active_record(3)
            .is_some()
    );
    assert!(
        first
            .open_table("users")
            .unwrap()
            .active_record(4)
            .is_none()
    );

    let latest = log.read_applied(&catalog).unwrap();
    assert!(
        latest
            .open_table("users")
            .unwrap()
            .active_record(4)
            .is_some()
    );
    assert!(matches!(
        log.read_at(&catalog, 3),
        Err(ReplicationError::ReadUnavailable {
            requested: 3,
            applied: 2
        })
    ));

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn follower_reads_reject_a_log_and_catalog_position_mismatch() {
    let root = catalog_root("follower-read-state");
    let catalog = Catalog::from_path(&root).unwrap();
    let mut log = ReplicationLog::open(&catalog, 1).unwrap();
    log.propose(&catalog, vec![post(3, "Carol")]).unwrap();
    let restored = ReplicationLog::new(1).unwrap();

    assert!(matches!(
        restored.read_applied(&catalog),
        Err(ReplicationError::CatalogStateMismatch {
            expected: 0,
            actual: 1
        })
    ));

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn replication_snapshot_round_trips_and_validates_the_catalog_image() {
    let root = catalog_root("snapshot-round-trip");
    let catalog = Catalog::from_path(&root).unwrap();
    let mut log = ReplicationLog::open(&catalog, 1).unwrap();
    log.propose(&catalog, vec![post(3, "Carol")]).unwrap();

    let snapshot = log.snapshot(&catalog).unwrap();
    let decoded = ReplicationSnapshot::from_json(&snapshot.to_json().unwrap()).unwrap();
    assert_eq!(decoded, snapshot);

    let mut unknown = serde_json::to_value(snapshot).unwrap();
    unknown["extra"] = json!(true);
    let error = ReplicationSnapshot::from_json(&serde_json::to_vec(&unknown).unwrap()).unwrap_err();
    assert!(error.to_string().contains("unknown field"));

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn snapshot_installation_compacts_history_and_resumes_replication() {
    let leader_root = catalog_root("snapshot-leader");
    let follower_root = catalog_root("snapshot-follower");
    let leader = Catalog::from_path(&leader_root).unwrap();
    let mut follower = Catalog::from_path(&follower_root).unwrap();
    let mut authority = ReplicationLog::open(&leader, 1).unwrap();
    let mut replica = ReplicationLog::open(&follower, 1).unwrap();
    authority.propose(&leader, vec![post(3, "Carol")]).unwrap();
    authority.propose(&leader, vec![post(4, "Dave")]).unwrap();

    for extension in ["mvcc", "cdc", "wal", "state", "idx"] {
        fs::write(
            follower_root.join(format!("users.txbase.{extension}")),
            b"stale",
        )
        .unwrap();
    }

    let snapshot = authority.snapshot(&leader).unwrap();
    assert_eq!(
        replica.install_snapshot(&mut follower, snapshot).unwrap(),
        ApplyOutcome::SnapshotInstalled {
            index: 2,
            transaction_id: 2
        }
    );
    let duplicate = authority.snapshot(&leader).unwrap();
    assert_eq!(
        replica.install_snapshot(&mut follower, duplicate).unwrap(),
        ApplyOutcome::SnapshotDuplicate {
            index: 2,
            transaction_id: 2
        }
    );
    assert!(replica.entries().is_empty());
    assert_eq!(replica.last_index(), 2);
    assert_eq!(replica.last_transaction_id(), 2);
    assert_eq!(Catalog::mvcc_versions(&follower_root).unwrap(), vec![2]);
    for extension in ["mvcc", "cdc", "wal", "state", "idx"] {
        assert!(
            !follower_root
                .join(format!("users.txbase.{extension}"))
                .exists()
        );
    }
    assert!(
        follower
            .open_table("users")
            .unwrap()
            .active_record(4)
            .is_some()
    );

    drop(replica);
    let mut replica = ReplicationLog::open(&follower, 1).unwrap();
    assert_eq!(replica.last_index(), 2);
    assert_eq!(replica.last_transaction_id(), 2);
    let entry = replica.propose(&follower, vec![post(5, "Eve")]).unwrap();
    assert_eq!(entry.index, 3);
    assert_eq!(entry.transaction_id, 3);
    assert_eq!(follower.transaction_id().unwrap(), Some(3));

    fs::remove_dir_all(leader_root).unwrap();
    fs::remove_dir_all(follower_root).unwrap();
}

#[test]
fn snapshot_installation_rejects_stale_and_conflicting_images() {
    let leader_root = catalog_root("snapshot-stale-leader");
    let follower_root = catalog_root("snapshot-stale-follower");
    let conflict_root = catalog_root("snapshot-conflict-leader");
    let leader = Catalog::from_path(&leader_root).unwrap();
    let mut follower = Catalog::from_path(&follower_root).unwrap();
    let conflict_catalog = Catalog::from_path(&conflict_root).unwrap();
    let mut authority = ReplicationLog::open(&leader, 1).unwrap();
    let mut conflict_authority = ReplicationLog::open(&conflict_catalog, 1).unwrap();
    let mut replica = ReplicationLog::open(&follower, 1).unwrap();
    authority.propose(&leader, vec![post(3, "Carol")]).unwrap();
    let first = authority.snapshot(&leader).unwrap();
    authority.propose(&leader, vec![post(4, "Dave")]).unwrap();
    let latest = authority.snapshot(&leader).unwrap();
    replica.install_snapshot(&mut follower, latest).unwrap();

    assert!(matches!(
        replica.install_snapshot(&mut follower, first),
        Err(ReplicationError::SnapshotStale {
            requested: 1,
            current: 2
        })
    ));

    conflict_authority
        .propose(&conflict_catalog, vec![post(3, "Mallory")])
        .unwrap();
    conflict_authority
        .propose(&conflict_catalog, vec![post(4, "Nina")])
        .unwrap();
    let conflicting = conflict_authority.snapshot(&conflict_catalog).unwrap();
    assert!(matches!(
        replica.install_snapshot(&mut follower, conflicting),
        Err(ReplicationError::SnapshotConflict { transaction_id: 2 })
    ));

    fs::remove_dir_all(leader_root).unwrap();
    fs::remove_dir_all(follower_root).unwrap();
    fs::remove_dir_all(conflict_root).unwrap();
}

#[test]
fn authority_compaction_preserves_suffix_and_reopens_without_advancing_catalog() {
    let root = catalog_root("compaction");
    let catalog = Catalog::from_path(&root).unwrap();
    let mut log = ReplicationLog::open(&catalog, 1).unwrap();
    log.propose(&catalog, vec![post(3, "Carol")]).unwrap();
    log.propose(&catalog, vec![post(4, "Dave")]).unwrap();
    log.propose(&catalog, vec![post(5, "Eve")]).unwrap();

    let snapshot = log.snapshot_at(&catalog, 2).unwrap();
    assert_eq!(snapshot.last_index, 2);
    assert_eq!(snapshot.last_transaction_id, 2);
    log.compact_through(&catalog, snapshot).unwrap();

    assert_eq!(log.base_index(), 2);
    assert_eq!(log.base_transaction_id(), 2);
    assert_eq!(log.entries().len(), 1);
    assert_eq!(log.entries()[0].index, 3);
    assert_eq!(log.entries()[0].transaction_id, 3);
    assert_eq!(catalog.transaction_id().unwrap(), Some(3));
    assert!(matches!(
        log.read_at(&catalog, 1),
        Err(ReplicationError::ReadHistoryUnavailable {
            requested: 1,
            base_transaction_id: 2
        })
    ));
    assert!(
        log.read_at(&catalog, 2)
            .unwrap()
            .open_table("users")
            .unwrap()
            .active_record(4)
            .is_some()
    );

    drop(log);
    let mut reopened = ReplicationLog::open(&catalog, 1).unwrap();
    assert_eq!(reopened.base_index(), 2);
    assert_eq!(reopened.last_index(), 3);
    let next = reopened.propose(&catalog, vec![post(6, "Frank")]).unwrap();
    assert_eq!(next.index, 4);
    assert_eq!(next.transaction_id, 4);
    assert_eq!(catalog.transaction_id().unwrap(), Some(4));

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn authority_compaction_rejects_unavailable_and_divergent_snapshots() {
    let root = catalog_root("compaction-reject");
    let conflict_root = catalog_root("compaction-conflict");
    let catalog = Catalog::from_path(&root).unwrap();
    let conflict_catalog = Catalog::from_path(&conflict_root).unwrap();
    let mut log = ReplicationLog::open(&catalog, 1).unwrap();
    log.propose(&catalog, vec![post(3, "Carol")]).unwrap();
    log.propose(&catalog, vec![post(4, "Dave")]).unwrap();
    log.propose(&catalog, vec![post(5, "Eve")]).unwrap();

    assert!(matches!(
        log.snapshot_at(&catalog, 4),
        Err(ReplicationError::SnapshotUnavailable {
            requested: 4,
            applied: 3
        })
    ));

    let mut conflict_log = ReplicationLog::open(&conflict_catalog, 1).unwrap();
    conflict_log
        .propose(&conflict_catalog, vec![post(3, "Mallory")])
        .unwrap();
    conflict_log
        .propose(&conflict_catalog, vec![post(4, "Nina")])
        .unwrap();
    let conflicting = conflict_log.snapshot_at(&conflict_catalog, 2).unwrap();
    let before = log.to_sidecar_bytes().unwrap();
    assert!(matches!(
        log.compact_through(&catalog, conflicting),
        Err(ReplicationError::SnapshotConflict { transaction_id: 2 })
    ));
    assert_eq!(log.base_index(), 0);
    assert_eq!(log.to_sidecar_bytes().unwrap(), before);
    assert_eq!(catalog.transaction_id().unwrap(), Some(3));

    fs::remove_dir_all(root).unwrap();
    fs::remove_dir_all(conflict_root).unwrap();
}

#[test]
fn follower_watermarks_gate_compaction_and_require_re_registration_after_restart() {
    let root = catalog_root("watermarks");
    let catalog = Catalog::from_path(&root).unwrap();
    let mut log = ReplicationLog::open(&catalog, 1).unwrap();
    log.propose(&catalog, vec![post(3, "Carol")]).unwrap();
    log.propose(&catalog, vec![post(4, "Dave")]).unwrap();
    log.propose(&catalog, vec![post(5, "Eve")]).unwrap();
    let (_, schema_tag) = catalog.schema_representation().unwrap();

    let snapshot = log.snapshot_at(&catalog, 2).unwrap();
    assert!(matches!(
        log.compact_through_acknowledged(&catalog, snapshot.clone()),
        Err(ReplicationError::NoFollowerProgress)
    ));

    let progress =
        ReplicationProgress::new("follower-a".into(), 1, 1, 1, schema_tag.clone()).unwrap();
    assert_eq!(
        log.acknowledge_follower(&catalog, progress.clone())
            .unwrap(),
        ReplicationProgressOutcome::Accepted
    );
    assert_eq!(
        log.acknowledge_follower(&catalog, progress).unwrap(),
        ReplicationProgressOutcome::Duplicate
    );
    let progress =
        ReplicationProgress::new("follower-b".into(), 1, 1, 1, schema_tag.clone()).unwrap();
    assert_eq!(
        log.acknowledge_follower(&catalog, progress).unwrap(),
        ReplicationProgressOutcome::Accepted
    );
    assert_eq!(log.safe_compaction_index(), Some(1));
    assert!(matches!(
        log.compact_through_acknowledged(&catalog, snapshot.clone()),
        Err(ReplicationError::CompactionNotAcknowledged {
            requested: 2,
            acknowledged: 1
        })
    ));

    let progress =
        ReplicationProgress::new("follower-a".into(), 1, 2, 2, schema_tag.clone()).unwrap();
    assert_eq!(
        log.acknowledge_follower(&catalog, progress).unwrap(),
        ReplicationProgressOutcome::Accepted
    );
    assert_eq!(log.safe_compaction_index(), Some(1));
    let progress = ReplicationProgress::new("follower-b".into(), 1, 2, 2, schema_tag).unwrap();
    assert_eq!(
        log.acknowledge_follower(&catalog, progress).unwrap(),
        ReplicationProgressOutcome::Accepted
    );
    assert_eq!(log.safe_compaction_index(), Some(2));
    log.compact_through_acknowledged(&catalog, snapshot)
        .unwrap();
    assert_eq!(log.base_index(), 2);
    assert_eq!(log.follower_count(), 2);

    let reopened = ReplicationLog::from_sidecar_bytes(&log.to_sidecar_bytes().unwrap()).unwrap();
    assert_eq!(reopened.base_index(), 2);
    assert_eq!(reopened.follower_count(), 0);

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn follower_watermarks_reject_invalid_positions_and_regressions() {
    let root = catalog_root("watermark-validation");
    let catalog = Catalog::from_path(&root).unwrap();
    let mut log = ReplicationLog::open(&catalog, 1).unwrap();
    log.propose(&catalog, vec![post(3, "Carol")]).unwrap();
    log.propose(&catalog, vec![post(4, "Dave")]).unwrap();
    let (_, schema_tag) = catalog.schema_representation().unwrap();

    let ahead = ReplicationProgress::new("follower-a".into(), 1, 3, 3, schema_tag.clone()).unwrap();
    assert!(matches!(
        log.acknowledge_follower(&catalog, ahead),
        Err(ReplicationError::ProgressUnavailable {
            requested: 3,
            applied: 2
        })
    ));

    let wrong_term =
        ReplicationProgress::new("follower-b".into(), 2, 1, 1, schema_tag.clone()).unwrap();
    assert!(matches!(
        log.acknowledge_follower(&catalog, wrong_term),
        Err(ReplicationError::TermMismatch {
            expected: 1,
            actual: 2
        })
    ));
    let wrong_schema =
        ReplicationProgress::new("follower-b".into(), 1, 1, 1, "other-schema".into()).unwrap();
    assert!(matches!(
        log.acknowledge_follower(&catalog, wrong_schema),
        Err(ReplicationError::SchemaMismatch { .. })
    ));
    let transaction_gap =
        ReplicationProgress::new("follower-b".into(), 1, 1, 2, schema_tag.clone()).unwrap();
    assert!(matches!(
        log.acknowledge_follower(&catalog, transaction_gap),
        Err(ReplicationError::TransactionGap {
            expected: 1,
            actual: 2
        })
    ));

    let accepted =
        ReplicationProgress::new("follower-a".into(), 1, 1, 1, schema_tag.clone()).unwrap();
    log.acknowledge_follower(&catalog, accepted).unwrap();
    let regression = ReplicationProgress::new("follower-a".into(), 1, 0, 0, schema_tag).unwrap();
    assert!(matches!(
        log.acknowledge_follower(&catalog, regression),
        Err(ReplicationError::ProgressRegression {
            follower_id,
            previous_index: 1,
            requested_index: 0
        }) if follower_id == "follower-a"
    ));

    let invalid_id =
        ReplicationProgress::new("follower/a".into(), 1, 1, 1, "schema".into()).unwrap_err();
    assert!(matches!(invalid_id, ReplicationError::Invalid(_)));

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn follower_progress_reports_the_applied_log_position_and_round_trips() {
    let root = catalog_root("progress-for");
    let catalog = Catalog::from_path(&root).unwrap();
    let mut log = ReplicationLog::open(&catalog, 7).unwrap();
    log.propose(&catalog, vec![post(3, "Carol")]).unwrap();
    log.propose(&catalog, vec![post(4, "Dave")]).unwrap();

    let progress = log.progress_for(&catalog, "follower-a".into()).unwrap();
    assert_eq!(progress.version, REPLICATION_PROGRESS_VERSION);
    assert_eq!(progress.follower_id, "follower-a");
    assert_eq!(progress.term, 7);
    assert_eq!(progress.index, 2);
    assert_eq!(progress.transaction_id, 2);
    assert!(!progress.schema_tag.is_empty());
    assert_eq!(
        ReplicationProgress::from_json(&progress.to_json().unwrap()).unwrap(),
        progress
    );

    let oversized = vec![b' '; MAX_REPLICATION_PROGRESS_BYTES + 1];
    assert!(matches!(
        ReplicationProgress::from_json(&oversized),
        Err(ReplicationError::Invalid(message))
            if message.contains("JSON exceeds")
    ));

    let oversized_progress = ReplicationProgress::new(
        "follower-a".into(),
        1,
        0,
        0,
        "x".repeat(MAX_REPLICATION_PROGRESS_BYTES),
    )
    .unwrap();
    assert!(matches!(
        oversized_progress.to_json(),
        Err(ReplicationError::Invalid(message))
            if message.contains("JSON exceeds")
    ));

    let invalid = log.progress_for(&catalog, "follower/a".into()).unwrap_err();
    assert!(matches!(invalid, ReplicationError::Invalid(_)));

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn replication_entry_batches_page_contiguous_positions_and_boundaries() {
    let root = catalog_root("entry-batch");
    let catalog = Catalog::from_path(&root).unwrap();
    let mut log = ReplicationLog::new(8).unwrap();
    log.propose(&catalog, vec![post(3, "Carol")]).unwrap();
    log.propose(&catalog, vec![post(4, "Dave")]).unwrap();
    log.propose(&catalog, vec![post(5, "Eve")]).unwrap();

    let first = log.entry_batch(0, 2).unwrap();
    assert_eq!(first.after_index, 0);
    assert_eq!(
        first
            .entries
            .iter()
            .map(|entry| entry.index)
            .collect::<Vec<_>>(),
        [1, 2]
    );
    assert_eq!(first.next_after, Some(2));
    assert_eq!(
        ReplicationEntryBatch::from_json(&first.to_json().unwrap()).unwrap(),
        first
    );

    let second = log.entry_batch(first.next_after.unwrap(), 2).unwrap();
    assert_eq!(
        second
            .entries
            .iter()
            .map(|entry| entry.index)
            .collect::<Vec<_>>(),
        [3]
    );
    assert_eq!(second.next_after, None);
    assert!(
        log.entry_batch(log.last_index(), 1)
            .unwrap()
            .entries
            .is_empty()
    );
    assert!(matches!(
        log.entry_batch(log.last_index() + 1, 1),
        Err(ReplicationError::EntriesUnavailable {
            requested: 4,
            applied: 3
        })
    ));
    assert!(matches!(
        log.entry_batch(0, 0),
        Err(ReplicationError::Invalid(_))
    ));

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn replication_entry_batches_reject_empty_or_beyond_last_pages() {
    let root = catalog_root("entry-batch-validation");
    let catalog = Catalog::from_path(&root).unwrap();
    let mut log = ReplicationLog::new(8).unwrap();
    log.propose(&catalog, vec![post(3, "Carol")]).unwrap();
    log.propose(&catalog, vec![post(4, "Dave")]).unwrap();

    let page = log.entry_batch(0, 2).unwrap();
    let mut empty = page.clone();
    empty.entries.clear();
    empty.next_after = None;
    assert!(matches!(
        empty.validate(),
        Err(ReplicationError::Invalid(message))
            if message.contains("cannot be empty before the last index")
    ));

    let mut beyond_last = page.clone();
    beyond_last.last_index = 1;
    beyond_last.last_transaction_id = 1;
    assert!(matches!(
        beyond_last.validate(),
        Err(ReplicationError::Invalid(message))
            if message.contains("beyond last_index")
    ));

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn replication_entry_batch_applies_in_order_and_replays_safely() {
    let leader_root = catalog_root("entry-batch-receive-leader");
    let follower_root = catalog_root("entry-batch-receive-follower");
    let leader = Catalog::from_path(&leader_root).unwrap();
    let follower = Catalog::from_path(&follower_root).unwrap();
    let mut authority = ReplicationLog::new(1).unwrap();
    let mut replica = ReplicationLog::new(1).unwrap();
    authority.propose(&leader, vec![post(3, "Carol")]).unwrap();
    authority.propose(&leader, vec![post(4, "Dave")]).unwrap();
    authority.propose(&leader, vec![post(5, "Eve")]).unwrap();

    let first = authority.entry_batch(0, 2).unwrap();
    assert_eq!(
        replica.receive_batch(&follower, first.clone()).unwrap(),
        vec![
            ApplyOutcome::Applied {
                index: 1,
                transaction_id: 1
            },
            ApplyOutcome::Applied {
                index: 2,
                transaction_id: 2
            }
        ]
    );
    assert_eq!(
        replica.receive_batch(&follower, first).unwrap(),
        vec![
            ApplyOutcome::Duplicate {
                index: 1,
                transaction_id: 1
            },
            ApplyOutcome::Duplicate {
                index: 2,
                transaction_id: 2
            }
        ]
    );
    assert_eq!(
        replica
            .receive_batch(&follower, authority.entry_batch(2, 2).unwrap())
            .unwrap(),
        vec![ApplyOutcome::Applied {
            index: 3,
            transaction_id: 3
        }]
    );

    fs::remove_dir_all(leader_root).unwrap();
    fs::remove_dir_all(follower_root).unwrap();
}

#[test]
fn snapshot_install_rejects_a_stale_txrp_sidecar_without_mutating_catalog() {
    let leader_root = catalog_root("snapshot-sidecar-leader");
    let follower_root = catalog_root("snapshot-sidecar-follower");
    let leader = Catalog::from_path(&leader_root).unwrap();
    let mut follower = Catalog::from_path(&follower_root).unwrap();
    let mut authority = ReplicationLog::open(&leader, 1).unwrap();
    authority.propose(&leader, vec![post(3, "Carol")]).unwrap();
    let snapshot = authority.snapshot(&leader).unwrap();
    let sidecar_after = ReplicationLog::with_position(
        snapshot.term,
        snapshot.last_index,
        snapshot.last_transaction_id,
    )
    .unwrap()
    .to_sidecar_bytes()
    .unwrap();

    let error = follower
        .install_snapshot_with_sidecar(
            &snapshot.catalog,
            0,
            crate::replication::REPLICATION_SIDECAR_NAME,
            Some(b"stale".to_vec()),
            sidecar_after,
        )
        .unwrap_err();
    assert!(matches!(
        error,
        crate::catalog::CatalogTransactionError::SidecarPreconditionFailed { name }
            if name == crate::replication::REPLICATION_SIDECAR_NAME
    ));
    assert_eq!(follower.transaction_id().unwrap(), None);
    assert!(
        follower
            .open_table("users")
            .unwrap()
            .active_record(3)
            .is_none()
    );
    assert!(
        follower
            .read_sidecar_bytes(crate::replication::REPLICATION_SIDECAR_NAME)
            .unwrap()
            .is_none()
    );
    assert!(!follower_root.join(".txbase.catalog.txn").exists());

    fs::remove_dir_all(leader_root).unwrap();
    fs::remove_dir_all(follower_root).unwrap();
}

#[test]
fn schema_and_term_mismatches_are_rejected_before_commit() {
    let root = catalog_root("validation");
    let catalog = Catalog::from_path(&root).unwrap();
    let mut log = ReplicationLog::new(1).unwrap();
    let bad_schema =
        ReplicationEntry::new(1, 1, 1, "wrong-schema".into(), vec![post(3, "Carol")]).unwrap();
    assert!(matches!(
        log.receive(&catalog, bad_schema),
        Err(ReplicationError::SchemaMismatch { .. })
    ));
    let bad_term =
        ReplicationEntry::new(2, 1, 1, "wrong-schema".into(), vec![post(3, "Carol")]).unwrap();
    assert!(matches!(
        log.receive(&catalog, bad_term),
        Err(ReplicationError::TermMismatch { .. })
    ));
    let bad_transaction_id =
        ReplicationEntry::new(1, 1, 2, "wrong-schema".into(), vec![post(3, "Carol")]).unwrap();
    assert!(matches!(
        log.receive(&catalog, bad_transaction_id),
        Err(ReplicationError::TransactionGap { .. })
    ));
    assert_eq!(catalog.transaction_id().unwrap(), None);

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn conflicting_duplicate_is_rejected_without_a_second_commit() {
    let leader_root = catalog_root("conflict-leader");
    let follower_root = catalog_root("conflict-follower");
    let leader = Catalog::from_path(&leader_root).unwrap();
    let follower = Catalog::from_path(&follower_root).unwrap();
    let mut authority = ReplicationLog::new(1).unwrap();
    let mut replica = ReplicationLog::new(1).unwrap();
    let first = authority.propose(&leader, vec![post(3, "Carol")]).unwrap();
    replica.receive(&follower, first.clone()).unwrap();

    let mut conflicting = first;
    conflicting.operations[0].body = Some(json!({
        "ID": 3,
        "NAME": "Mallory",
        "AGE": 42,
        "ACTIVE": true
    }));
    assert!(matches!(
        replica.receive(&follower, conflicting),
        Err(ReplicationError::ConflictingDuplicate { index: 1 })
    ));
    assert_eq!(
        follower
            .open_table("users")
            .unwrap()
            .active_record(3)
            .unwrap()
            .values["NAME"],
        "Carol"
    );
    assert_eq!(follower.transaction_id().unwrap(), Some(1));

    fs::remove_dir_all(leader_root).unwrap();
    fs::remove_dir_all(follower_root).unwrap();
}

#[test]
fn duplicate_delivery_requires_the_catalog_to_be_at_the_same_position() {
    let leader_root = catalog_root("duplicate-state-leader");
    let follower_root = catalog_root("duplicate-state-follower");
    let leader = Catalog::from_path(&leader_root).unwrap();
    let follower = Catalog::from_path(&follower_root).unwrap();
    let mut authority = ReplicationLog::new(1).unwrap();
    let entry = authority.propose(&leader, vec![post(3, "Carol")]).unwrap();
    let mut recovered = ReplicationLog::from_json(&authority.to_json().unwrap()).unwrap();

    assert!(matches!(
        recovered.receive(&follower, entry),
        Err(ReplicationError::CatalogStateMismatch {
            expected: 1,
            actual: 0
        })
    ));
    assert_eq!(follower.transaction_id().unwrap(), None);

    fs::remove_dir_all(leader_root).unwrap();
    fs::remove_dir_all(follower_root).unwrap();
}
