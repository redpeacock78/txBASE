use super::{Catalog, ReplicationError, ReplicationLog};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const REPLICATION_PROGRESS_VERSION: u16 = 1;
pub const REPLICATION_PROGRESS_SIDECAR_NAME: &str = ".txbase.replication-progress";
pub const MAX_REPLICATION_FOLLOWERS: usize = 1024;
pub const MAX_REPLICATION_PROGRESS_BYTES: usize = crate::MAX_JSON_INPUT_BYTES;
const MAX_FOLLOWER_ID_BYTES: usize = 128;
const REPLICATION_PROGRESS_SIDECAR_MAGIC: &[u8; 4] = b"TXRG";
const REPLICATION_PROGRESS_SIDECAR_VERSION: u8 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplicationProgress {
    pub version: u16,
    pub follower_id: String,
    pub term: u64,
    pub index: u64,
    pub transaction_id: u64,
    pub schema_tag: String,
}

impl ReplicationProgress {
    pub fn new(
        follower_id: String,
        term: u64,
        index: u64,
        transaction_id: u64,
        schema_tag: String,
    ) -> Result<Self, ReplicationError> {
        let progress = Self {
            version: REPLICATION_PROGRESS_VERSION,
            follower_id,
            term,
            index,
            transaction_id,
            schema_tag,
        };
        progress.validate()?;
        Ok(progress)
    }

    pub fn validate(&self) -> Result<(), ReplicationError> {
        if self.version != REPLICATION_PROGRESS_VERSION {
            return Err(ReplicationError::Invalid(format!(
                "unsupported replication progress version: {}",
                self.version
            )));
        }
        if self.follower_id.is_empty()
            || self.follower_id.len() > MAX_FOLLOWER_ID_BYTES
            || !self
                .follower_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        {
            return Err(ReplicationError::Invalid(
                "follower_id must be 1 to 128 ASCII letters, digits, '.', '_', or '-'".into(),
            ));
        }
        if self.term == 0 {
            return Err(ReplicationError::Invalid(
                "progress term must be positive".into(),
            ));
        }
        if (self.index == 0) != (self.transaction_id == 0) {
            return Err(ReplicationError::Invalid(
                "progress index and transaction_id must both be zero or both be positive".into(),
            ));
        }
        if self.schema_tag.trim().is_empty() {
            return Err(ReplicationError::Invalid(
                "progress schema_tag must not be empty".into(),
            ));
        }
        Ok(())
    }

    pub fn to_json(&self) -> Result<Vec<u8>, ReplicationError> {
        self.validate()?;
        let bytes = serde_json::to_vec(self)
            .map_err(|error| ReplicationError::Serialization(error.to_string()))?;
        if bytes.len() > MAX_REPLICATION_PROGRESS_BYTES {
            return Err(ReplicationError::Invalid(format!(
                "replication progress JSON exceeds {MAX_REPLICATION_PROGRESS_BYTES} bytes"
            )));
        }
        Ok(bytes)
    }

    pub fn from_json(bytes: &[u8]) -> Result<Self, ReplicationError> {
        if bytes.len() > MAX_REPLICATION_PROGRESS_BYTES {
            return Err(ReplicationError::Invalid(format!(
                "replication progress JSON exceeds {MAX_REPLICATION_PROGRESS_BYTES} bytes"
            )));
        }
        let progress: Self = serde_json::from_slice(bytes)
            .map_err(|error| ReplicationError::Serialization(error.to_string()))?;
        progress.validate()?;
        Ok(progress)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplicationProgressOutcome {
    Accepted,
    Duplicate,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct FollowerWatermarks {
    followers: BTreeMap<String, ReplicationProgress>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FollowerWatermarksPayload {
    followers: BTreeMap<String, ReplicationProgress>,
}

impl FollowerWatermarks {
    pub(crate) fn to_sidecar_bytes(&self) -> Result<Vec<u8>, ReplicationError> {
        if self.followers.len() > MAX_REPLICATION_FOLLOWERS {
            return Err(ReplicationError::Invalid(format!(
                "replication follower count exceeds {MAX_REPLICATION_FOLLOWERS}"
            )));
        }
        for (follower_id, progress) in &self.followers {
            if follower_id != &progress.follower_id {
                return Err(ReplicationError::Invalid(
                    "replication progress follower key does not match follower_id".into(),
                ));
            }
            progress.validate()?;
        }
        let payload = serde_json::to_vec(&FollowerWatermarksPayload {
            followers: self.followers.clone(),
        })
        .map_err(|error| ReplicationError::Serialization(error.to_string()))?;
        if payload.len() > MAX_REPLICATION_PROGRESS_BYTES {
            return Err(ReplicationError::Invalid(format!(
                "replication progress sidecar JSON exceeds {MAX_REPLICATION_PROGRESS_BYTES} bytes"
            )));
        }
        let mut bytes = REPLICATION_PROGRESS_SIDECAR_MAGIC.to_vec();
        bytes.push(REPLICATION_PROGRESS_SIDECAR_VERSION);
        bytes.extend_from_slice(&payload);
        Ok(bytes)
    }

    pub(crate) fn from_sidecar_bytes(bytes: &[u8]) -> Result<Self, ReplicationError> {
        let Some(body) = bytes.strip_prefix(REPLICATION_PROGRESS_SIDECAR_MAGIC) else {
            return Err(ReplicationError::Invalid(
                "replication progress sidecar header is invalid".into(),
            ));
        };
        let Some((&version, payload)) = body.split_first() else {
            return Err(ReplicationError::Invalid(
                "replication progress sidecar header is truncated".into(),
            ));
        };
        if version != REPLICATION_PROGRESS_SIDECAR_VERSION {
            return Err(ReplicationError::Invalid(format!(
                "unsupported replication progress sidecar version: {version}"
            )));
        }
        if payload.len() > MAX_REPLICATION_PROGRESS_BYTES {
            return Err(ReplicationError::Invalid(format!(
                "replication progress sidecar JSON exceeds {MAX_REPLICATION_PROGRESS_BYTES} bytes"
            )));
        }
        let payload: FollowerWatermarksPayload = serde_json::from_slice(payload)
            .map_err(|error| ReplicationError::Serialization(error.to_string()))?;
        if payload.followers.len() > MAX_REPLICATION_FOLLOWERS {
            return Err(ReplicationError::Invalid(format!(
                "replication follower count exceeds {MAX_REPLICATION_FOLLOWERS}"
            )));
        }
        for (follower_id, progress) in &payload.followers {
            progress.validate()?;
            if follower_id != &progress.follower_id {
                return Err(ReplicationError::Invalid(
                    "replication progress follower key does not match follower_id".into(),
                ));
            }
        }
        Ok(Self {
            followers: payload.followers,
        })
    }

    pub(crate) fn validate_against(
        &self,
        log: &ReplicationLog,
        catalog: &Catalog,
    ) -> Result<(), ReplicationError> {
        let mut validated = Self::default();
        for progress in self.followers.values() {
            validated.acknowledge(log, catalog, progress.clone())?;
        }
        if validated != *self {
            return Err(ReplicationError::Invalid(
                "replication progress sidecar contains inconsistent followers".into(),
            ));
        }
        Ok(())
    }

    pub(crate) fn acknowledge(
        &mut self,
        log: &ReplicationLog,
        catalog: &Catalog,
        progress: ReplicationProgress,
    ) -> Result<ReplicationProgressOutcome, ReplicationError> {
        progress.validate()?;
        log.validate()?;
        log.ensure_catalog_position(super::current_transaction_id(catalog)?)?;
        if progress.term != log.term() {
            return Err(ReplicationError::TermMismatch {
                expected: log.term(),
                actual: progress.term,
            });
        }
        let (_, schema_tag) = catalog
            .schema_representation()
            .map_err(ReplicationError::Catalog)?;
        if progress.schema_tag != schema_tag {
            return Err(ReplicationError::SchemaMismatch {
                expected: schema_tag,
                actual: progress.schema_tag,
            });
        }
        if progress.index < log.base_index() {
            return Err(ReplicationError::HistoryUnavailable {
                index: progress.index,
                base_index: log.base_index(),
            });
        }
        if progress.index > log.last_index() {
            return Err(ReplicationError::ProgressUnavailable {
                requested: progress.index,
                applied: log.last_index(),
            });
        }
        let offset = progress.index - log.base_index();
        let expected_transaction_id = log
            .base_transaction_id()
            .checked_add(offset)
            .ok_or_else(|| ReplicationError::Invalid("progress transaction ID exhausted".into()))?;
        if progress.transaction_id != expected_transaction_id {
            return Err(ReplicationError::TransactionGap {
                expected: expected_transaction_id,
                actual: progress.transaction_id,
            });
        }

        if let Some(previous) = self.followers.get(&progress.follower_id) {
            if previous == &progress {
                return Ok(ReplicationProgressOutcome::Duplicate);
            }
            if progress.index < previous.index {
                return Err(ReplicationError::ProgressRegression {
                    follower_id: progress.follower_id,
                    previous_index: previous.index,
                    requested_index: progress.index,
                });
            }
        } else if self.followers.len() >= MAX_REPLICATION_FOLLOWERS {
            return Err(ReplicationError::Invalid(format!(
                "replication follower count exceeds {MAX_REPLICATION_FOLLOWERS}"
            )));
        }

        self.followers
            .insert(progress.follower_id.clone(), progress);
        Ok(ReplicationProgressOutcome::Accepted)
    }

    pub(crate) fn safe_compaction_index(&self) -> Option<u64> {
        self.followers.values().map(|progress| progress.index).min()
    }

    pub(crate) fn follower_count(&self) -> usize {
        self.followers.len()
    }
}
