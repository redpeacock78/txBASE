use super::TypeConfig;
use fs2::FileExt;
use openraft::{Entry, LogId, LogState, Vote};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

mod journal;
mod storage_adapter;
use journal::{
    FRAME_HEADER_BYTES, JOURNAL_MAGIC, LOCK_NAME, MAX_RECORD_BYTES, create_generation,
    invalid_data, journal_path, latest_generation, parse_generation, sync_directory, write_record,
};

#[derive(Clone)]
pub struct RaftLogStore {
    inner: Arc<Mutex<StoreInner>>,
}

impl RaftLogStore {
    /// Opens the durable Raft log in a node-specific directory.
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        Ok(Self {
            inner: Arc::new(Mutex::new(StoreInner::open(path.as_ref())?)),
        })
    }
}

struct StoreInner {
    directory: PathBuf,
    generation: u64,
    journal: File,
    _lock_file: File,
    state: LogStateData,
    poisoned: bool,
}

#[derive(Default)]
struct LogStateData {
    entries: BTreeMap<u64, Entry<TypeConfig>>,
    last_purged: Option<LogId<u64>>,
    committed: Option<LogId<u64>>,
    vote: Option<Vote<u64>>,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
enum JournalRecord {
    Append(Entry<TypeConfig>),
    Vote(Vote<u64>),
    Committed(Option<LogId<u64>>),
    Truncate(LogId<u64>),
    Purge(LogId<u64>),
}

impl StoreInner {
    fn open(directory: &Path) -> io::Result<Self> {
        fs::create_dir_all(directory)?;
        let lock_file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(directory.join(LOCK_NAME))?;
        FileExt::try_lock_exclusive(&lock_file)?;

        let generation = latest_generation(directory)?.unwrap_or_default();
        let path = journal_path(directory, generation);
        let journal = if path.exists() {
            OpenOptions::new().read(true).append(true).open(&path)?
        } else if generation == 0 {
            create_generation(directory, generation, |_| Ok(()))?
        } else {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "latest Raft log generation is missing",
            ));
        };

        let mut store = Self {
            directory: directory.to_path_buf(),
            generation,
            journal,
            _lock_file: lock_file,
            state: LogStateData::default(),
            poisoned: false,
        };
        store.recover()?;
        store.remove_old_generations();
        Ok(store)
    }

    fn recover(&mut self) -> io::Result<()> {
        let length = self.journal.metadata()?.len();
        if length < JOURNAL_MAGIC.len() as u64 {
            return Err(invalid_data("Raft log journal header is truncated"));
        }

        self.journal.seek(SeekFrom::Start(0))?;
        let mut magic = [0; JOURNAL_MAGIC.len()];
        self.journal.read_exact(&mut magic)?;
        if &magic != JOURNAL_MAGIC {
            return Err(invalid_data("Raft log journal header is invalid"));
        }

        let mut offset = JOURNAL_MAGIC.len() as u64;
        while offset < length {
            let remaining = length - offset;
            if remaining < 4 {
                self.truncate_torn_tail(offset)?;
                break;
            }

            self.journal.seek(SeekFrom::Start(offset))?;
            let mut length_bytes = [0; 4];
            self.journal.read_exact(&mut length_bytes)?;
            let record_length = u32::from_le_bytes(length_bytes) as usize;
            if record_length == 0 || record_length > MAX_RECORD_BYTES {
                return Err(invalid_data("Raft log journal record length is invalid"));
            }
            let frame_length = FRAME_HEADER_BYTES + record_length as u64;
            if remaining < frame_length {
                self.truncate_torn_tail(offset)?;
                break;
            }

            let mut checksum = [0; 32];
            self.journal.read_exact(&mut checksum)?;
            let mut payload = vec![0; record_length];
            self.journal.read_exact(&mut payload)?;
            let actual: [u8; 32] = Sha256::digest(&payload).into();
            if checksum != actual {
                return Err(invalid_data("Raft log journal checksum does not match"));
            }
            let record = serde_json::from_slice(&payload)
                .map_err(|error| invalid_data(error.to_string()))?;
            self.apply_record(record)?;
            offset += frame_length;
        }

        self.journal.seek(SeekFrom::End(0))?;
        Ok(())
    }

    fn truncate_torn_tail(&mut self, valid_length: u64) -> io::Result<()> {
        self.journal.set_len(valid_length)?;
        self.journal.sync_all()
    }

    fn append_entries(&mut self, entries: Vec<Entry<TypeConfig>>) -> io::Result<()> {
        let Some(first) = entries.first() else {
            return Ok(());
        };
        let first_index = first.log_id.index;
        if self
            .state
            .last_purged
            .is_some_and(|purged| first_index <= purged.index)
            || self
                .state
                .committed
                .is_some_and(|committed| first_index <= committed.index)
        {
            return Err(invalid_data(
                "Raft log append cannot replace a purged or committed entry",
            ));
        }
        if let Some(last) = self.last_log_index() {
            if first_index > last {
                let next = last
                    .checked_add(1)
                    .ok_or_else(|| invalid_data("Raft log index overflow"))?;
                if first_index != next {
                    return Err(invalid_data(format!(
                        "Raft log append expected index {next}, got {first_index}"
                    )));
                }
            }
        }

        let mut previous: Option<u64> = None;
        for entry in &entries {
            if let Some(previous) = previous {
                let expected = previous
                    .checked_add(1)
                    .ok_or_else(|| invalid_data("Raft log index overflow"))?;
                if entry.log_id.index != expected {
                    return Err(invalid_data(format!(
                        "Raft log append expected index {expected}, got {}",
                        entry.log_id.index
                    )));
                }
            }
            previous = Some(entry.log_id.index);
        }

        self.persist_records(entries.iter().cloned().map(JournalRecord::Append))?;
        self.state.entries.split_off(&first_index);
        for entry in entries {
            self.state.entries.insert(entry.log_id.index, entry);
        }
        Ok(())
    }

    fn save_vote(&mut self, vote: Vote<u64>) -> io::Result<()> {
        if self.state.vote == Some(vote) {
            return Ok(());
        }
        if self.state.vote.is_some_and(|current| vote < current) {
            return Err(invalid_data("Raft vote must not move backwards"));
        }
        self.persist_records(std::iter::once(JournalRecord::Vote(vote)))?;
        self.state.vote = Some(vote);
        Ok(())
    }

    fn save_committed(&mut self, committed: Option<LogId<u64>>) -> io::Result<()> {
        if self.state.committed == committed {
            return Ok(());
        }
        if let Some(current) = self.state.committed {
            match committed {
                Some(next) if next.index > current.index => {}
                Some(next) if next == current => return Ok(()),
                _ => {
                    return Err(invalid_data(
                        "committed Raft log id must not move backwards",
                    ));
                }
            }
        }
        self.persist_records(std::iter::once(JournalRecord::Committed(committed)))?;
        self.state.committed = committed;
        Ok(())
    }

    fn truncate(&mut self, log_id: LogId<u64>) -> io::Result<()> {
        if self
            .state
            .last_purged
            .is_some_and(|purged| log_id.index <= purged.index)
            || self.state.entries.range(log_id.index..).next().is_none()
        {
            return Ok(());
        }
        if self
            .state
            .committed
            .is_some_and(|committed| log_id.index <= committed.index)
        {
            return Err(invalid_data("cannot truncate a committed Raft log entry"));
        }

        self.persist_records(std::iter::once(JournalRecord::Truncate(log_id)))?;
        self.state.entries.split_off(&log_id.index);
        Ok(())
    }

    fn purge(&mut self, log_id: LogId<u64>) -> io::Result<()> {
        if let Some(purged) = self.state.last_purged {
            if log_id.index < purged.index {
                return Ok(());
            }
            if log_id.index == purged.index {
                return if log_id == purged {
                    Ok(())
                } else {
                    Err(invalid_data(
                        "Raft purge conflicts with the existing log id",
                    ))
                };
            }
        }

        self.persist_records(std::iter::once(JournalRecord::Purge(log_id)))?;
        self.state.entries.retain(|index, _| *index > log_id.index);
        self.state.last_purged = Some(log_id);
        self.compact()
    }

    fn persist_records<I>(&mut self, records: I) -> io::Result<()>
    where
        I: IntoIterator<Item = JournalRecord>,
    {
        if self.poisoned {
            return Err(io::Error::other(
                "Raft log store is unusable after a storage error",
            ));
        }
        let original_length = self.journal.metadata()?.len();
        let result = (|| {
            for record in records {
                write_record(&mut self.journal, &record)?;
            }
            self.journal.sync_all()
        })();
        if let Err(error) = result {
            if let Err(rollback_error) = self
                .journal
                .set_len(original_length)
                .and_then(|()| self.journal.sync_all())
            {
                self.poisoned = true;
                return Err(rollback_error);
            }
            return Err(error);
        }
        Ok(())
    }

    fn apply_record(&mut self, record: JournalRecord) -> io::Result<()> {
        match record {
            JournalRecord::Append(entry) => {
                let index = entry.log_id.index;
                if self
                    .state
                    .last_purged
                    .is_some_and(|purged| index <= purged.index)
                    || self
                        .state
                        .committed
                        .is_some_and(|committed| index <= committed.index)
                {
                    return Err(invalid_data(
                        "Raft journal replaces a purged or committed entry",
                    ));
                }
                if let Some(last) = self.last_log_index() {
                    if index > last {
                        let next = last
                            .checked_add(1)
                            .ok_or_else(|| invalid_data("Raft log index overflow"))?;
                        if index != next {
                            return Err(invalid_data(format!(
                                "Raft log journal expected index {next}, got {index}"
                            )));
                        }
                    }
                }
                self.state.entries.split_off(&index);
                self.state.entries.insert(index, entry);
            }
            JournalRecord::Vote(vote) => {
                if self.state.vote.is_some_and(|current| vote < current) {
                    return Err(invalid_data("Raft journal contains a backwards vote"));
                }
                self.state.vote = Some(vote);
            }
            JournalRecord::Committed(committed) => {
                if let Some(current) = self.state.committed {
                    if !matches!(committed, Some(next) if next.index > current.index || next == current)
                    {
                        return Err(invalid_data(
                            "Raft journal contains a backwards committed log id",
                        ));
                    }
                }
                self.state.committed = committed;
            }
            JournalRecord::Truncate(log_id) => {
                if self
                    .state
                    .last_purged
                    .is_some_and(|purged| log_id.index <= purged.index)
                    || self
                        .state
                        .committed
                        .is_some_and(|committed| log_id.index <= committed.index)
                {
                    return Err(invalid_data("Raft journal truncates a protected log entry"));
                }
                self.state.entries.split_off(&log_id.index);
            }
            JournalRecord::Purge(log_id) => {
                if self
                    .state
                    .last_purged
                    .is_some_and(|purged| log_id.index <= purged.index && log_id != purged)
                {
                    return Err(invalid_data("Raft journal contains a backwards purge"));
                }
                self.state.entries.retain(|index, _| *index > log_id.index);
                self.state.last_purged = Some(log_id);
            }
        }
        Ok(())
    }

    fn log_state(&self) -> LogState<TypeConfig> {
        let last_log_id = self
            .state
            .entries
            .last_key_value()
            .map(|(_, entry)| entry.log_id)
            .or(self.state.last_purged);
        LogState {
            last_purged_log_id: self.state.last_purged,
            last_log_id,
        }
    }

    fn last_log_index(&self) -> Option<u64> {
        self.state
            .entries
            .last_key_value()
            .map(|(index, _)| *index)
            .or(self.state.last_purged.map(|log_id| log_id.index))
    }

    fn compact(&mut self) -> io::Result<()> {
        if self.poisoned {
            return Err(io::Error::other(
                "Raft log store is unusable after a storage error",
            ));
        }
        let next_generation = self
            .generation
            .checked_add(1)
            .ok_or_else(|| invalid_data("Raft log generation overflow"))?;
        let new_journal = create_generation(&self.directory, next_generation, |file| {
            if let Some(purged) = self.state.last_purged {
                write_record(file, &JournalRecord::Purge(purged))?;
            }
            if let Some(vote) = self.state.vote {
                write_record(file, &JournalRecord::Vote(vote))?;
            }
            for entry in self.state.entries.values() {
                write_record(file, &JournalRecord::Append(entry.clone()))?;
            }
            if let Some(committed) = self.state.committed {
                write_record(file, &JournalRecord::Committed(Some(committed)))?;
            }
            Ok(())
        })
        .inspect_err(|_| self.poisoned = true)?;

        let old_generation = self.generation;
        self.journal = new_journal;
        self.generation = next_generation;
        let _ = fs::remove_file(journal_path(&self.directory, old_generation));
        let _ = sync_directory(&self.directory);
        Ok(())
    }

    fn remove_old_generations(&self) {
        let Ok(entries) = fs::read_dir(&self.directory) else {
            return;
        };
        for entry in entries.flatten() {
            let Some(generation) = parse_generation(&entry.file_name()) else {
                continue;
            };
            if generation < self.generation {
                let _ = fs::remove_file(entry.path());
            }
        }
    }
}

#[cfg(test)]
#[path = "log_store_tests.rs"]
mod tests;
