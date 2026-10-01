use super::*;

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
