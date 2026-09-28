use super::{
    RaftCatalogStateMachine, RaftCommand, RaftCommandPrecondition, RaftRejection, RaftResponse,
    RaftResponseResult, TypeConfig,
};
use crate::catalog::Catalog;
use crate::replication::raft::RAFT_STATE_SIDECAR_NAME;
use crate::xbase::{OperationIr, OperationMethod, TransactionStep};
use openraft::storage::RaftStateMachine;
use openraft::{
    BasicNode, CommittedLeaderId, Entry, EntryPayload, LogId, Membership, RaftSnapshotBuilder,
};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::runtime::{Builder, Runtime};

static NEXT_TEMP_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct TempDirectory(PathBuf);

impl TempDirectory {
    fn new() -> Self {
        let sequence = NEXT_TEMP_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "txbase-raft-state-machine-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn runtime() -> Runtime {
    Builder::new_multi_thread()
        .worker_threads(1)
        .build()
        .unwrap()
}

fn users_dbf() -> Vec<u8> {
    include_str!("../../../tests/fixtures/users.dbf.hex")
        .split_whitespace()
        .map(|token| u8::from_str_radix(token, 16).unwrap())
        .collect()
}

fn seed_catalog(root: &std::path::Path) -> Catalog {
    fs::create_dir_all(root).unwrap();
    fs::write(root.join("users.dbf"), users_dbf()).unwrap();
    let catalog = Catalog::from_path(root).unwrap();
    catalog
        .commit_steps_with_preconditions(&[insert_record(900)], None, None)
        .unwrap();
    catalog
}

fn insert_record(id: u64) -> TransactionStep {
    TransactionStep::Mutation(OperationIr {
        method: OperationMethod::Post,
        path: "/users/records".into(),
        body: Some(json!({
            "ID": id,
            "NAME": format!("User {id}"),
            "AGE": 30,
            "ACTIVE": true
        })),
    })
}

fn command(
    catalog: &Catalog,
    client: &str,
    sequence: u64,
    id: u64,
    tag: Option<&str>,
) -> RaftCommand {
    let actual_tag = catalog.schema_representation().unwrap().1;
    RaftCommand::new(
        client.into(),
        sequence,
        tag.unwrap_or(&actual_tag).into(),
        None,
        vec![insert_record(id)],
    )
    .unwrap()
}

fn entry(index: u64, payload: EntryPayload<TypeConfig>) -> Entry<TypeConfig> {
    Entry {
        log_id: LogId::new(CommittedLeaderId::new(1, 1), index),
        payload,
    }
}

fn applied(response: Option<RaftResponse>) -> u64 {
    match response.unwrap().result {
        RaftResponseResult::Applied { transaction_id } => transaction_id,
        RaftResponseResult::Rejected { reason } => panic!("unexpected rejection: {reason:?}"),
    }
}

#[test]
fn command_retries_are_durable_and_sequence_errors_do_not_mutate() {
    let directory = TempDirectory::new();
    let catalog = seed_catalog(&directory.0);
    let mut machine = RaftCatalogStateMachine::initialize(catalog.clone()).unwrap();
    let first = command(&catalog, "client-a", 1, 901, None);
    let runtime = runtime();
    assert_eq!(
        applied(
            runtime
                .block_on(machine.apply(vec![entry(0, EntryPayload::Normal(first.clone()),)]))
                .unwrap()[0]
                .clone()
        ),
        2
    );

    drop(machine);
    let catalog = Catalog::from_path(&directory.0).unwrap();
    let mut machine = RaftCatalogStateMachine::open(catalog.clone()).unwrap();
    let retry = runtime
        .block_on(machine.apply(vec![entry(1, EntryPayload::Normal(first.clone()))]))
        .unwrap()
        .remove(0)
        .unwrap();
    assert_eq!(
        retry.result,
        RaftResponseResult::Applied { transaction_id: 2 }
    );
    assert_eq!(catalog.transaction_id().unwrap(), Some(2));

    let conflict = command(&catalog, "client-a", 1, 902, None);
    let conflict_response = runtime
        .block_on(machine.apply(vec![entry(2, EntryPayload::Normal(conflict))]))
        .unwrap()
        .remove(0)
        .unwrap();
    assert_eq!(
        conflict_response.result,
        RaftResponseResult::Rejected {
            reason: RaftRejection::ClientSequenceConflict
        }
    );

    let gap = command(&catalog, "client-a", 3, 903, None);
    let gap_response = runtime
        .block_on(machine.apply(vec![entry(3, EntryPayload::Normal(gap))]))
        .unwrap()
        .remove(0)
        .unwrap();
    assert_eq!(
        gap_response.result,
        RaftResponseResult::Rejected {
            reason: RaftRejection::ClientSequenceGap
        }
    );

    let stale_tag = command(&catalog, "client-a", 2, 904, Some("stale-tag"));
    let rejected = runtime
        .block_on(machine.apply(vec![entry(4, EntryPayload::Normal(stale_tag.clone()))]))
        .unwrap()
        .remove(0)
        .unwrap();
    assert_eq!(
        rejected.result,
        RaftResponseResult::Rejected {
            reason: RaftRejection::CatalogChanged
        }
    );
    let retry = runtime
        .block_on(machine.apply(vec![entry(5, EntryPayload::Normal(stale_tag))]))
        .unwrap()
        .remove(0)
        .unwrap();
    assert_eq!(retry, rejected);
    assert_eq!(catalog.transaction_id().unwrap(), Some(2));

    let next = command(&catalog, "client-a", 3, 905, None);
    assert_eq!(
        applied(
            runtime
                .block_on(machine.apply(vec![entry(6, EntryPayload::Normal(next),)]))
                .unwrap()[0]
                .clone()
        ),
        3
    );
    assert_eq!(catalog.transaction_id().unwrap(), Some(3));
    let users = catalog.open_table("users").unwrap();
    assert_eq!(users.active_record(4).unwrap().values["ID"], 901);
    assert_eq!(users.active_record(5).unwrap().values["ID"], 905);

    let table_precondition = RaftCommand::new(
        "client-a".into(),
        4,
        catalog.schema_representation().unwrap().1,
        Some(RaftCommandPrecondition::Table {
            name: "users".into(),
            if_match: Some("stale-etag".into()),
            if_none_match: None,
        }),
        vec![insert_record(906)],
    )
    .unwrap();
    let rejected = runtime
        .block_on(machine.apply(vec![entry(
            7,
            EntryPayload::Normal(table_precondition.clone()),
        )]))
        .unwrap()
        .remove(0)
        .unwrap();
    assert_eq!(
        rejected.result,
        RaftResponseResult::Rejected {
            reason: RaftRejection::PreconditionFailed
        }
    );
    let retry = runtime
        .block_on(machine.apply(vec![entry(8, EntryPayload::Normal(table_precondition))]))
        .unwrap()
        .remove(0)
        .unwrap();
    assert_eq!(retry, rejected);

    let old = command(&catalog, "client-a", 1, 907, None);
    let old_response = runtime
        .block_on(machine.apply(vec![entry(9, EntryPayload::Normal(old))]))
        .unwrap()
        .remove(0)
        .unwrap();
    assert_eq!(
        old_response.result,
        RaftResponseResult::Rejected {
            reason: RaftRejection::ClientSequenceTooOld
        }
    );
    assert_eq!(catalog.transaction_id().unwrap(), Some(3));
}

#[test]
fn blank_and_membership_entries_advance_only_the_raft_position() {
    let directory = TempDirectory::new();
    let catalog = seed_catalog(&directory.0);
    let mut machine = RaftCatalogStateMachine::initialize(catalog.clone()).unwrap();
    let membership = Membership::new(
        vec![BTreeSet::from([1])],
        BTreeMap::from([(1, BasicNode::new("127.0.0.1:9000"))]),
    );
    let results = runtime()
        .block_on(machine.apply(vec![
            entry(0, EntryPayload::Blank),
            entry(1, EntryPayload::Membership(membership)),
        ]))
        .unwrap();
    assert_eq!(results, vec![None, None]);
    let (last_applied, last_membership) = runtime().block_on(machine.applied_state()).unwrap();
    assert_eq!(last_applied.unwrap().index, 1);
    assert_eq!(last_membership.log_id().as_ref().unwrap().index, 1);
    assert_eq!(catalog.transaction_id().unwrap(), Some(1));
}

#[test]
fn snapshots_transfer_catalog_membership_and_retry_state_together() {
    let leader_directory = TempDirectory::new();
    let follower_directory = TempDirectory::new();
    let leader_catalog = seed_catalog(&leader_directory.0);
    let mut leader = RaftCatalogStateMachine::initialize(leader_catalog.clone()).unwrap();
    let command = command(&leader_catalog, "client-a", 1, 901, None);
    runtime()
        .block_on(leader.apply(vec![entry(0, EntryPayload::Normal(command))]))
        .unwrap();
    let mut builder = runtime().block_on(leader.get_snapshot_builder());
    let snapshot = runtime().block_on(builder.build_snapshot()).unwrap();

    fs::create_dir_all(&follower_directory.0).unwrap();
    let follower_catalog = Catalog::from_path(&follower_directory.0).unwrap();
    let mut follower = RaftCatalogStateMachine::open(follower_catalog.clone()).unwrap();
    let input = Box::new(Cursor::new(snapshot.snapshot.get_ref().clone()));
    runtime()
        .block_on(follower.install_snapshot(&snapshot.meta, input))
        .unwrap();
    let installed_catalog = Catalog::from_path(&follower_directory.0).unwrap();
    assert_eq!(installed_catalog.transaction_id().unwrap(), Some(2));
    assert!(
        installed_catalog
            .open_table("users")
            .unwrap()
            .active_record(4)
            .is_some()
    );

    let restored = runtime()
        .block_on(follower.get_current_snapshot())
        .unwrap()
        .unwrap();
    assert_eq!(restored.meta, snapshot.meta);
    assert_eq!(
        follower_catalog
            .read_sidecar_bytes(RAFT_STATE_SIDECAR_NAME)
            .unwrap(),
        leader_catalog
            .read_sidecar_bytes(RAFT_STATE_SIDECAR_NAME)
            .unwrap()
    );
}

#[test]
fn startup_fails_closed_for_catalogs_without_raft_state() {
    let directory = TempDirectory::new();
    let catalog = seed_catalog(&directory.0);
    assert!(RaftCatalogStateMachine::open(catalog).is_err());
    assert!(RaftCatalogStateMachine::initialize(Catalog::from_path(&directory.0).unwrap()).is_ok());
}

#[test]
fn genesis_fingerprint_adoption_requires_an_empty_catalog() {
    let empty_directory = TempDirectory::new();
    let empty =
        RaftCatalogStateMachine::open(Catalog::from_path(&empty_directory.0).unwrap()).unwrap();
    assert!(empty.can_adopt_genesis_fingerprint().unwrap());

    let populated_directory = TempDirectory::new();
    let populated =
        RaftCatalogStateMachine::initialize(seed_catalog(&populated_directory.0)).unwrap();
    assert!(!populated.can_adopt_genesis_fingerprint().unwrap());
}

#[test]
fn empty_genesis_initializes_and_records_its_durable_marker() {
    let directory = TempDirectory::new();
    let catalog = Catalog::from_path(&directory.0).unwrap();
    let mut machine = RaftCatalogStateMachine::open(catalog).unwrap();
    let applied = runtime()
        .block_on(machine.apply(vec![entry(0, EntryPayload::Blank)]))
        .unwrap();
    assert_eq!(applied, vec![None]);
    assert_eq!(
        runtime()
            .block_on(machine.applied_state())
            .unwrap()
            .0
            .unwrap()
            .index,
        0
    );
}
