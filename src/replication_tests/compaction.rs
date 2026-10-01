use super::*;

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
fn follower_watermarks_gate_compaction_and_survive_restart() {
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
    assert_eq!(catalog.transaction_id().unwrap(), Some(3));
    assert!(
        catalog
            .read_sidecar_bytes(REPLICATION_PROGRESS_SIDECAR_NAME)
            .unwrap()
            .is_some()
    );

    drop(log);
    let reopened = ReplicationLog::open(&catalog, 1).unwrap();
    assert_eq!(reopened.base_index(), 2);
    assert_eq!(reopened.follower_count(), 2);
    assert_eq!(reopened.safe_compaction_index(), Some(2));

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
fn follower_progress_sidecar_rejects_malformed_state_on_restart() {
    let root = catalog_root("progress-sidecar-invalid");
    let catalog = Catalog::from_path(&root).unwrap();
    let mut log = ReplicationLog::open(&catalog, 1).unwrap();
    log.propose(&catalog, vec![post(3, "Carol")]).unwrap();
    let (_, schema_tag) = catalog.schema_representation().unwrap();
    log.acknowledge_follower(
        &catalog,
        ReplicationProgress::new("follower-a".into(), 1, 1, 1, schema_tag).unwrap(),
    )
    .unwrap();
    let valid = catalog
        .read_sidecar_bytes(REPLICATION_PROGRESS_SIDECAR_NAME)
        .unwrap();
    catalog
        .replace_sidecar_without_transaction(
            REPLICATION_PROGRESS_SIDECAR_NAME,
            valid,
            b"TXRG\x01{\"followers\":{\"follower-a\":{\"version\":1}}}".to_vec(),
        )
        .unwrap();
    drop(log);

    assert!(matches!(
        ReplicationLog::open(&catalog, 1),
        Err(ReplicationError::Serialization(_))
    ));

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn follower_progress_sidecar_requires_a_replication_log_sidecar() {
    let root = catalog_root("progress-sidecar-orphan");
    let catalog = Catalog::from_path(&root).unwrap();
    catalog
        .replace_sidecar_without_transaction(
            REPLICATION_PROGRESS_SIDECAR_NAME,
            None,
            b"TXRG\x01{\"followers\":{}}".to_vec(),
        )
        .unwrap();

    assert!(matches!(
        ReplicationLog::open(&catalog, 1),
        Err(ReplicationError::Invalid(message))
            if message.contains("requires a replication log sidecar")
    ));

    fs::remove_dir_all(root).unwrap();
}
