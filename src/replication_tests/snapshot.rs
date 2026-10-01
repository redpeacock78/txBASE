use super::*;

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
    replica.propose(&follower, vec![post(3, "Carol")]).unwrap();
    let (_, follower_schema_tag) = follower.schema_representation().unwrap();
    replica
        .acknowledge_follower(
            &follower,
            ReplicationProgress::new("follower-a".into(), 1, 1, 1, follower_schema_tag).unwrap(),
        )
        .unwrap();

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
    assert_eq!(replica.follower_count(), 0);
    assert_eq!(replica.last_index(), 2);
    assert_eq!(replica.last_transaction_id(), 2);
    assert!(
        !follower_root
            .join(REPLICATION_PROGRESS_SIDECAR_NAME)
            .exists()
    );
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
        .install_snapshot_with_sidecars(
            &snapshot.catalog,
            0,
            vec![(
                crate::replication::REPLICATION_SIDECAR_NAME,
                Some(b"stale".to_vec()),
                Some(sidecar_after),
            )],
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
