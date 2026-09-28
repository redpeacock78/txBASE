use super::super::TypeConfig;
use super::super::state_machine::run_blocking;
use super::RaftLogStore;
use openraft::storage::{LogFlushed, RaftLogReader, RaftLogStorage};
use openraft::{Entry, ErrorSubject, ErrorVerb, LogId, LogState, StorageError, Vote};
use std::io;
use std::ops::RangeBounds;

impl RaftLogReader<TypeConfig> for RaftLogStore {
    async fn try_get_log_entries<RB>(
        &mut self,
        range: RB,
    ) -> Result<Vec<Entry<TypeConfig>>, StorageError<u64>>
    where
        RB: RangeBounds<u64> + Clone + std::fmt::Debug + Send,
    {
        let inner = self.inner.lock().map_err(|error| {
            storage_error(
                ErrorSubject::Logs,
                ErrorVerb::Read,
                io::Error::other(format!("Raft log mutex poisoned: {error}")),
            )
        })?;
        Ok(inner
            .state
            .entries
            .range(range)
            .map(|(_, entry)| entry.clone())
            .collect())
    }
}

impl RaftLogStorage<TypeConfig> for RaftLogStore {
    type LogReader = Self;

    async fn get_log_state(&mut self) -> Result<LogState<TypeConfig>, StorageError<u64>> {
        let inner = self.inner.clone();
        run_blocking(ErrorSubject::Logs, ErrorVerb::Read, move || {
            let inner = inner
                .lock()
                .map_err(|error| format!("Raft log mutex poisoned: {error}"))?;
            Ok(inner.log_state())
        })
        .await
        .map_err(|error| *error)
    }

    async fn get_log_reader(&mut self) -> Self::LogReader {
        self.clone()
    }

    async fn save_vote(&mut self, vote: &Vote<u64>) -> Result<(), StorageError<u64>> {
        let inner = self.inner.clone();
        let vote = *vote;
        run_blocking(ErrorSubject::Vote, ErrorVerb::Write, move || {
            let mut inner = inner
                .lock()
                .map_err(|error| format!("Raft log mutex poisoned: {error}"))?;
            inner.save_vote(vote).map_err(|error| error.to_string())
        })
        .await
        .map_err(|error| *error)
    }

    async fn read_vote(&mut self) -> Result<Option<Vote<u64>>, StorageError<u64>> {
        let inner = self.inner.clone();
        run_blocking(ErrorSubject::Vote, ErrorVerb::Read, move || {
            let inner = inner
                .lock()
                .map_err(|error| format!("Raft log mutex poisoned: {error}"))?;
            Ok(inner.state.vote)
        })
        .await
        .map_err(|error| *error)
    }

    async fn save_committed(
        &mut self,
        committed: Option<LogId<u64>>,
    ) -> Result<(), StorageError<u64>> {
        let inner = self.inner.clone();
        run_blocking(ErrorSubject::Logs, ErrorVerb::Write, move || {
            let mut inner = inner
                .lock()
                .map_err(|error| format!("Raft log mutex poisoned: {error}"))?;
            inner
                .save_committed(committed)
                .map_err(|error| error.to_string())
        })
        .await
        .map_err(|error| *error)
    }

    async fn read_committed(&mut self) -> Result<Option<LogId<u64>>, StorageError<u64>> {
        let inner = self.inner.clone();
        run_blocking(ErrorSubject::Logs, ErrorVerb::Read, move || {
            let inner = inner
                .lock()
                .map_err(|error| format!("Raft log mutex poisoned: {error}"))?;
            Ok(inner.state.committed)
        })
        .await
        .map_err(|error| *error)
    }

    async fn append<I>(
        &mut self,
        entries: I,
        callback: LogFlushed<TypeConfig>,
    ) -> Result<(), StorageError<u64>>
    where
        I: IntoIterator<Item = Entry<TypeConfig>> + Send,
        I::IntoIter: Send,
    {
        let entries = entries.into_iter().collect::<Vec<_>>();
        let inner = self.inner.clone();
        let result = tokio::task::spawn_blocking(move || {
            let mut inner = inner
                .lock()
                .map_err(|error| io::Error::other(format!("Raft log mutex poisoned: {error}")))?;
            inner.append_entries(entries)
        })
        .await
        .map_err(|error| io::Error::other(error.to_string()))
        .and_then(|result| result);
        callback.log_io_completed(result);
        Ok(())
    }

    async fn truncate(&mut self, log_id: LogId<u64>) -> Result<(), StorageError<u64>> {
        let inner = self.inner.clone();
        run_blocking(
            ErrorSubject::LogIndex(log_id.index),
            ErrorVerb::Delete,
            move || {
                let mut inner = inner
                    .lock()
                    .map_err(|error| format!("Raft log mutex poisoned: {error}"))?;
                inner.truncate(log_id).map_err(|error| error.to_string())
            },
        )
        .await
        .map_err(|error| *error)
    }

    async fn purge(&mut self, log_id: LogId<u64>) -> Result<(), StorageError<u64>> {
        let inner = self.inner.clone();
        run_blocking(
            ErrorSubject::LogIndex(log_id.index),
            ErrorVerb::Delete,
            move || {
                let mut inner = inner
                    .lock()
                    .map_err(|error| format!("Raft log mutex poisoned: {error}"))?;
                inner.purge(log_id).map_err(|error| error.to_string())
            },
        )
        .await
        .map_err(|error| *error)
    }
}

fn storage_error(
    subject: ErrorSubject<u64>,
    verb: ErrorVerb,
    error: io::Error,
) -> StorageError<u64> {
    StorageError::from_io_error(subject, verb, error)
}
