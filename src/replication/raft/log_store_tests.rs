use super::*;
use crate::catalog::Catalog;
use crate::replication::raft::RaftCatalogStateMachine;
use openraft::entry::RaftEntry;
use openraft::testing::{StoreBuilder, Suite};
use openraft::{CommittedLeaderId, Entry, ErrorSubject, ErrorVerb, LogId, StorageError, Vote};
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEMP_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct TempDirectory(PathBuf);

impl TempDirectory {
    fn new() -> io::Result<Self> {
        let sequence = NEXT_TEMP_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("txbase-raft-log-{}-{sequence}", std::process::id()));
        fs::create_dir_all(&path)?;
        Ok(Self(path))
    }
}

impl Drop for TempDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn log_id(index: u64) -> LogId<u64> {
    LogId::new(CommittedLeaderId::new(1, 1), index)
}

fn blank_entry(index: u64) -> Entry<TypeConfig> {
    Entry::new_blank(log_id(index))
}

#[test]
fn journal_recovers_vote_commit_and_purged_position_after_restart() {
    let directory = TempDirectory::new().unwrap();
    let path = directory.0.join("raft");
    {
        let mut store = StoreInner::open(&path).unwrap();
        store
            .append_entries(vec![blank_entry(5), blank_entry(6), blank_entry(7)])
            .unwrap();
        store.save_vote(Vote::new(2, 1)).unwrap();
        store.save_committed(Some(log_id(6))).unwrap();
        store.truncate(log_id(7)).unwrap();
        store.purge(log_id(5)).unwrap();
    }

    let store = StoreInner::open(&path).unwrap();
    assert_eq!(store.state.vote, Some(Vote::new(2, 1)));
    assert_eq!(store.state.committed, Some(log_id(6)));
    assert_eq!(store.state.entries.len(), 1);
    assert!(store.state.entries.contains_key(&6));
    assert_eq!(
        store.log_state(),
        LogState {
            last_purged_log_id: Some(log_id(5)),
            last_log_id: Some(log_id(6)),
        }
    );
}

#[test]
fn journal_replays_conflicting_append_as_suffix_replacement() {
    let directory = TempDirectory::new().unwrap();
    let path = directory.0.join("raft");
    let replacement_log_id = LogId::new(CommittedLeaderId::new(2, 2), 1);
    {
        let mut store = StoreInner::open(&path).unwrap();
        store
            .append_entries(vec![blank_entry(0), blank_entry(1), blank_entry(2)])
            .unwrap();
        store
            .append_entries(vec![Entry::new_blank(replacement_log_id)])
            .unwrap();
    }

    let store = StoreInner::open(&path).unwrap();
    assert_eq!(store.state.entries.len(), 2);
    assert!(store.state.entries.contains_key(&0));
    assert_eq!(
        store.state.entries.get(&1).unwrap().log_id,
        replacement_log_id
    );
}

#[test]
fn append_discards_a_purged_prefix_and_keeps_the_new_suffix() {
    let directory = TempDirectory::new().unwrap();
    let mut store = StoreInner::open(&directory.0).unwrap();
    store
        .append_entries((0..=2).map(blank_entry).collect())
        .unwrap();
    store.purge(log_id(1)).unwrap();

    let replacement = Entry::new_blank(LogId::new(CommittedLeaderId::new(2, 1), 2));
    store
        .append_entries(vec![blank_entry(1), replacement.clone()])
        .unwrap();
    drop(store);

    let store = StoreInner::open(&directory.0).unwrap();

    assert_eq!(Some(&replacement), store.state.entries.get(&2));
    assert_eq!(
        vec![2],
        store.state.entries.keys().copied().collect::<Vec<_>>()
    );
}

#[test]
fn journal_discards_an_incomplete_final_frame() {
    let directory = TempDirectory::new().unwrap();
    let path = directory.0.join("raft");
    let valid_length;
    {
        let mut store = StoreInner::open(&path).unwrap();
        store.append_entries(vec![blank_entry(0)]).unwrap();
        valid_length = store.journal.metadata().unwrap().len();
    }
    let journal_path = journal_path(&path, 0);
    fs::OpenOptions::new()
        .append(true)
        .open(&journal_path)
        .unwrap()
        .write_all(&[0x01, 0x02])
        .unwrap();

    let store = StoreInner::open(&path).unwrap();
    assert_eq!(store.state.entries.len(), 1);
    assert!(store.state.entries.contains_key(&0));
    assert_eq!(store.journal.metadata().unwrap().len(), valid_length);
}

#[test]
fn journal_discards_a_torn_final_payload_frame() {
    let directory = TempDirectory::new().unwrap();
    let path = directory.0.join("raft");
    let valid_length;
    {
        let mut store = StoreInner::open(&path).unwrap();
        store.append_entries(vec![blank_entry(0)]).unwrap();
        valid_length = store.journal.metadata().unwrap().len();
    }

    let journal_path = journal_path(&path, 0);
    let mut journal = fs::OpenOptions::new()
        .append(true)
        .open(&journal_path)
        .unwrap();
    journal.write_all(&64_u32.to_le_bytes()).unwrap();
    journal.write_all(&[0; 32]).unwrap();
    journal.write_all(b"partial").unwrap();
    drop(journal);

    let store = StoreInner::open(&path).unwrap();
    assert_eq!(store.state.entries.len(), 1);
    assert!(store.state.entries.contains_key(&0));
    assert_eq!(store.journal.metadata().unwrap().len(), valid_length);
}

#[test]
fn journal_rejects_a_complete_frame_with_a_bad_checksum() {
    let directory = TempDirectory::new().unwrap();
    let path = directory.0.join("raft");
    {
        let mut store = StoreInner::open(&path).unwrap();
        store.append_entries(vec![blank_entry(0)]).unwrap();
    }

    let journal_path = journal_path(&path, 0);
    let mut bytes = fs::read(&journal_path).unwrap();
    bytes[JOURNAL_MAGIC.len() + 4] ^= 0xff;
    fs::write(&journal_path, &bytes).unwrap();

    let error = StoreInner::open(&path)
        .err()
        .expect("corrupt frame must fail");
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert_eq!(fs::read(&journal_path).unwrap(), bytes);
}

#[test]
fn only_one_process_can_open_a_node_log() {
    let directory = TempDirectory::new().unwrap();
    let path = directory.0.join("raft");
    let store = RaftLogStore::open(&path).unwrap();
    assert!(RaftLogStore::open(&path).is_err());
    drop(store);
    assert!(RaftLogStore::open(&path).is_ok());
}

struct TestStoreBuilder;

impl StoreBuilder<TypeConfig, RaftLogStore, RaftCatalogStateMachine, TempDirectory>
    for TestStoreBuilder
{
    async fn build(
        &self,
    ) -> Result<(TempDirectory, RaftLogStore, RaftCatalogStateMachine), StorageError<u64>> {
        let directory = TempDirectory::new().map_err(|error| {
            StorageError::from_io_error(ErrorSubject::Store, ErrorVerb::Write, error)
        })?;
        let catalog_path = directory.0.join("catalog");
        fs::create_dir_all(&catalog_path).map_err(|error| {
            StorageError::from_io_error(ErrorSubject::Store, ErrorVerb::Write, error)
        })?;
        let catalog = Catalog::from_path(catalog_path).map_err(|error| {
            StorageError::from_io_error(
                ErrorSubject::StateMachine,
                ErrorVerb::Write,
                io::Error::other(error.to_string()),
            )
        })?;
        let state_machine = RaftCatalogStateMachine::initialize(catalog).map_err(|error| {
            StorageError::from_io_error(
                ErrorSubject::StateMachine,
                ErrorVerb::Write,
                io::Error::other(error),
            )
        })?;
        let log_store = RaftLogStore::open(directory.0.join("raft")).map_err(|error| {
            StorageError::from_io_error(ErrorSubject::Store, ErrorVerb::Write, error)
        })?;
        Ok((directory, log_store, state_machine))
    }
}

#[test]
fn passes_openraft_storage_suite() {
    Suite::<
        TypeConfig,
        RaftLogStore,
        RaftCatalogStateMachine,
        TestStoreBuilder,
        TempDirectory,
    >::test_all(TestStoreBuilder)
    .unwrap();
}
