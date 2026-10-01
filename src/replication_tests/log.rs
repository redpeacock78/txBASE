use super::*;

#[test]
fn entries_are_versioned_and_round_trip() {
    let entry = ReplicationEntry::new(1, 1, 1, "schema-v1".into(), vec![post(3, "Carol")]).unwrap();
    assert_eq!(entry.version, REPLICATION_ENTRY_VERSION);
    let serialized = serde_json::to_value(&entry).unwrap();
    assert!(serialized.get("operations").is_some());
    assert!(serialized.get("steps").is_none());
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
