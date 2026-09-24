use super::{REPLICATION_TRANSPORT_VERSION, ReplicationEntry, ReplicationError, ReplicationLog};
use serde::{Deserialize, Serialize};

pub const MAX_REPLICATION_ENTRY_BATCH: usize = 128;
pub const MAX_REPLICATION_ENTRY_BATCH_BYTES: usize = crate::MAX_JSON_INPUT_BYTES;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplicationEntryBatch {
    pub transport_version: u16,
    pub term: u64,
    pub base_index: u64,
    pub base_transaction_id: u64,
    pub after_index: u64,
    pub last_index: u64,
    pub last_transaction_id: u64,
    pub entries: Vec<ReplicationEntry>,
    pub next_after: Option<u64>,
}

impl ReplicationEntryBatch {
    pub(crate) fn from_log(
        log: &ReplicationLog,
        after_index: u64,
        limit: usize,
    ) -> Result<Self, ReplicationError> {
        if !(1..=MAX_REPLICATION_ENTRY_BATCH).contains(&limit) {
            return Err(ReplicationError::Invalid(format!(
                "replication entry batch limit must be between 1 and {MAX_REPLICATION_ENTRY_BATCH}"
            )));
        }
        log.validate()?;
        if after_index < log.base_index() {
            return Err(ReplicationError::HistoryUnavailable {
                index: after_index,
                base_index: log.base_index(),
            });
        }
        if after_index > log.last_index() {
            return Err(ReplicationError::EntriesUnavailable {
                requested: after_index,
                applied: log.last_index(),
            });
        }

        let offset = usize::try_from(after_index - log.base_index()).map_err(|_| {
            ReplicationError::Invalid("replication entry offset is too large".into())
        })?;
        let available = &log.entries()[offset..];
        let mut entries = Vec::new();
        for entry in available.iter().take(limit) {
            let mut candidate = Self::new(log, after_index, entries.clone());
            candidate.entries.push(entry.clone());
            candidate.next_after = candidate
                .entries
                .last()
                .and_then(|last| (last.index < log.last_index()).then_some(last.index));
            candidate.validate()?;
            if serde_json::to_vec(&candidate)
                .map_err(|error| ReplicationError::Serialization(error.to_string()))?
                .len()
                > MAX_REPLICATION_ENTRY_BATCH_BYTES
            {
                if entries.is_empty() {
                    return Err(ReplicationError::Invalid(format!(
                        "one replication entry exceeds the {MAX_REPLICATION_ENTRY_BATCH_BYTES}-byte batch response limit"
                    )));
                }
                break;
            }
            entries.push(entry.clone());
        }

        let mut batch = Self::new(log, after_index, entries);
        batch.next_after = batch
            .entries
            .last()
            .and_then(|last| (last.index < log.last_index()).then_some(last.index));
        batch.validate()?;
        Ok(batch)
    }

    fn new(log: &ReplicationLog, after_index: u64, entries: Vec<ReplicationEntry>) -> Self {
        Self {
            transport_version: REPLICATION_TRANSPORT_VERSION,
            term: log.term(),
            base_index: log.base_index(),
            base_transaction_id: log.base_transaction_id(),
            after_index,
            last_index: log.last_index(),
            last_transaction_id: log.last_transaction_id(),
            entries,
            next_after: None,
        }
    }

    pub fn validate(&self) -> Result<(), ReplicationError> {
        if self.transport_version != REPLICATION_TRANSPORT_VERSION {
            return Err(ReplicationError::Invalid(format!(
                "unsupported replication transport version: {}",
                self.transport_version
            )));
        }
        if self.term == 0
            || self.base_index > self.after_index
            || self.last_index < self.after_index
            || self.base_transaction_id > self.last_transaction_id
        {
            return Err(ReplicationError::Invalid(
                "replication entry batch position is invalid".into(),
            ));
        }
        let expected_last_transaction_id = self
            .base_transaction_id
            .checked_add(self.last_index - self.base_index)
            .ok_or_else(|| {
                ReplicationError::Invalid("replication transaction ID exhausted".into())
            })?;
        if self.last_transaction_id != expected_last_transaction_id {
            return Err(ReplicationError::TransactionGap {
                expected: expected_last_transaction_id,
                actual: self.last_transaction_id,
            });
        }
        if self.entries.len() > MAX_REPLICATION_ENTRY_BATCH {
            return Err(ReplicationError::Invalid(format!(
                "replication entry batch exceeds {MAX_REPLICATION_ENTRY_BATCH} entries"
            )));
        }
        let mut expected_index = self.after_index.checked_add(1);
        let mut expected_transaction_id = self
            .base_transaction_id
            .checked_add(self.after_index - self.base_index)
            .and_then(|value| value.checked_add(1));
        for entry in &self.entries {
            entry.validate()?;
            if entry.term != self.term {
                return Err(ReplicationError::TermMismatch {
                    expected: self.term,
                    actual: entry.term,
                });
            }
            if Some(entry.index) != expected_index {
                return Err(ReplicationError::IndexGap {
                    expected: expected_index.unwrap_or(u64::MAX),
                    actual: entry.index,
                });
            }
            let Some(expected_transaction_id_value) = expected_transaction_id else {
                return Err(ReplicationError::Invalid(
                    "replication entry transaction ID is exhausted".into(),
                ));
            };
            if entry.transaction_id != expected_transaction_id_value {
                return Err(ReplicationError::TransactionGap {
                    expected: expected_transaction_id_value,
                    actual: entry.transaction_id,
                });
            }
            expected_transaction_id = entry.transaction_id.checked_add(1);
            expected_index = entry.index.checked_add(1);
        }
        let expected_next_after = self
            .entries
            .last()
            .and_then(|entry| (entry.index < self.last_index).then_some(entry.index));
        if self.next_after != expected_next_after {
            return Err(ReplicationError::Invalid(
                "replication entry batch continuation is invalid".into(),
            ));
        }
        Ok(())
    }

    pub fn to_json(&self) -> Result<Vec<u8>, ReplicationError> {
        self.validate()?;
        let bytes = serde_json::to_vec(self)
            .map_err(|error| ReplicationError::Serialization(error.to_string()))?;
        if bytes.len() > MAX_REPLICATION_ENTRY_BATCH_BYTES {
            return Err(ReplicationError::Invalid(format!(
                "replication entry batch JSON exceeds {MAX_REPLICATION_ENTRY_BATCH_BYTES} bytes"
            )));
        }
        Ok(bytes)
    }

    pub fn from_json(bytes: &[u8]) -> Result<Self, ReplicationError> {
        if bytes.len() > MAX_REPLICATION_ENTRY_BATCH_BYTES {
            return Err(ReplicationError::Invalid(format!(
                "replication entry batch JSON exceeds {MAX_REPLICATION_ENTRY_BATCH_BYTES} bytes"
            )));
        }
        let batch: Self = serde_json::from_slice(bytes)
            .map_err(|error| ReplicationError::Serialization(error.to_string()))?;
        batch.validate()?;
        Ok(batch)
    }
}

impl ReplicationLog {
    /// Returns a bounded contiguous entry page after an already applied index.
    pub fn entry_batch(
        &self,
        after_index: u64,
        limit: usize,
    ) -> Result<ReplicationEntryBatch, ReplicationError> {
        ReplicationEntryBatch::from_log(self, after_index, limit)
    }
}
