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
