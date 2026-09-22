use super::lock::TableLock;
use super::{DbfError, DbfTable};
use crate::transaction::{FileWal, MAX_WAL_RECORD_SIZE, Wal};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

const MAGIC: &[u8; 4] = b"TXCD";
const VERSION: u8 = 1;
const HEADER_SIZE: usize = MAGIC.len() + 1 + 4;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChangeEvent {
    pub transaction_id: u64,
    pub reset: bool,
    pub changes: Vec<ChangeRecord>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChangeRecord {
    pub record_number: usize,
    pub before: Option<ChangeState>,
    pub after: Option<ChangeState>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChangeState {
    pub deleted: bool,
    pub values: Map<String, Value>,
}

pub(super) fn path_for(path: &Path) -> PathBuf {
    path.with_extension("txbase.cdc")
}

pub(super) fn event_for_tables(
    transaction_id: u64,
    before: &DbfTable,
    after: &DbfTable,
    reset: bool,
) -> Result<ChangeEvent, DbfError> {
    if transaction_id == 0 {
        return Err(DbfError::Invalid(
            "CDC transaction ID must be positive".into(),
        ));
    }
    let record_count = before.records.len().max(after.records.len());
    let mut changes = Vec::new();
    for index in 0..record_count {
        let before = before.records.get(index).map(|record| ChangeState {
            deleted: record.deleted,
            values: record.values.clone(),
        });
        let after = after.records.get(index).map(|record| ChangeState {
            deleted: record.deleted,
            values: record.values.clone(),
        });
        if reset || before != after {
            changes.push(ChangeRecord {
                record_number: index + 1,
                before,
                after,
            });
        }
    }
    Ok(ChangeEvent {
        transaction_id,
        reset,
        changes,
    })
}

pub(super) fn payload(event: &ChangeEvent) -> Result<Vec<u8>, DbfError> {
    validate_event(event)?;
    let body = serde_json::to_vec(event)
        .map_err(|error| DbfError::Invalid(format!("CDC event encoding failed: {error}")))?;
    let length = u32::try_from(body.len())
        .map_err(|_| DbfError::Invalid("CDC event payload is too large".into()))?;
    let total = HEADER_SIZE
        .checked_add(body.len())
        .ok_or_else(|| DbfError::Invalid("CDC event payload length overflows".into()))?;
    if total > MAX_WAL_RECORD_SIZE {
        return Err(DbfError::Invalid(
            "CDC event payload exceeds the WAL record limit".into(),
        ));
    }
    let mut bytes = Vec::with_capacity(total);
    bytes.extend_from_slice(MAGIC);
    bytes.push(VERSION);
    bytes.extend_from_slice(&length.to_le_bytes());
    bytes.extend_from_slice(&body);
    Ok(bytes)
}

pub(super) fn decode_payload(payload: &[u8]) -> Result<Option<ChangeEvent>, DbfError> {
    let Some(body) = payload.strip_prefix(MAGIC) else {
        return Ok(None);
    };
    if body.len() < 5 {
        return Err(DbfError::Invalid("CDC event header is truncated".into()));
    }
    if body[0] != VERSION {
        return Err(DbfError::Invalid(format!(
            "unknown CDC event version {}",
            body[0]
        )));
    }
    let length = usize::try_from(u32::from_le_bytes(
        body[1..5].try_into().expect("CDC event length is fixed"),
    ))
    .map_err(|_| DbfError::Invalid("CDC event length overflows usize".into()))?;
    let encoded = body
        .get(5..)
        .filter(|encoded| encoded.len() == length)
        .ok_or_else(|| DbfError::Invalid("CDC event length does not match payload".into()))?;
    let event = serde_json::from_slice(encoded)
        .map_err(|error| DbfError::Invalid(format!("CDC event decoding failed: {error}")))?;
    validate_event(&event)?;
    Ok(Some(event))
}

pub(super) fn event_from_wal(wal: &FileWal) -> Result<Option<ChangeEvent>, DbfError> {
    let mut event = None;
    for (_, payload) in wal.records() {
        let Some(candidate) = decode_payload(payload)? else {
            continue;
        };
        if let Some(existing) = &event {
            if existing != &candidate {
                return Err(DbfError::Invalid(
                    "WAL contains conflicting CDC events".into(),
                ));
            }
        } else {
            event = Some(candidate);
        }
    }
    Ok(event)
}

pub(super) fn append(path: &Path, event: &ChangeEvent) -> Result<(), DbfError> {
    let cdc_path = path_for(path);
    let encoded = payload(event)?;
    let mut wal = FileWal::open(&cdc_path).map_err(super::transaction_error)?;
    let mut last_transaction_id = None;
    for (_, record) in wal.records() {
        let Some(existing) = decode_payload(record)? else {
            return Err(DbfError::Invalid(
                "CDC log contains an unknown record".into(),
            ));
        };
        if let Some(last) = last_transaction_id {
            if existing.transaction_id <= last {
                return Err(DbfError::Invalid(
                    "CDC transaction IDs are not strictly increasing".into(),
                ));
            }
        }
        if existing.transaction_id == event.transaction_id {
            if existing == *event {
                return Ok(());
            }
            return Err(DbfError::Invalid(format!(
                "CDC transaction {} has conflicting event data",
                event.transaction_id
            )));
        }
        last_transaction_id = Some(existing.transaction_id);
    }
    if last_transaction_id.is_some_and(|last| last > event.transaction_id) {
        return Err(DbfError::Invalid(format!(
            "CDC transaction {} is older than the log tail",
            event.transaction_id
        )));
    }
    wal.append(&encoded).map_err(super::transaction_error)?;
    wal.sync().map_err(super::transaction_error)
}

pub(super) fn read(path: &Path, after: Option<u64>) -> Result<Vec<ChangeEvent>, DbfError> {
    let cdc_path = path_for(path);
    if !cdc_path.exists() {
        return Ok(Vec::new());
    }
    let wal = FileWal::open(&cdc_path).map_err(super::transaction_error)?;
    let mut events = Vec::new();
    let mut last_transaction_id = None;
    for (_, record) in wal.records() {
        let Some(event) = decode_payload(record)? else {
            return Err(DbfError::Invalid(
                "CDC log contains an unknown record".into(),
            ));
        };
        if last_transaction_id.is_some_and(|last| event.transaction_id <= last) {
            return Err(DbfError::Invalid(
                "CDC transaction IDs are not strictly increasing".into(),
            ));
        }
        last_transaction_id = Some(event.transaction_id);
        if after.is_none_or(|transaction_id| event.transaction_id > transaction_id) {
            events.push(event);
        }
    }
    Ok(events)
}

impl DbfTable {
    pub fn cdc_events(
        path: impl AsRef<Path>,
        after: Option<u64>,
    ) -> Result<Vec<ChangeEvent>, DbfError> {
        let path = path.as_ref();
        let _lock = TableLock::acquire(path)?;
        Self::recover_wal_with_encoding(path, None)?;
        let _ = super::schema_export::recover_schema_export_locked(path)?;
        read(path, after)
    }
}

fn validate_event(event: &ChangeEvent) -> Result<(), DbfError> {
    if event.transaction_id == 0 {
        return Err(DbfError::Invalid(
            "CDC transaction ID must be positive".into(),
        ));
    }
    let mut previous = 0;
    for change in &event.changes {
        if change.record_number == 0 || change.record_number <= previous {
            return Err(DbfError::Invalid(
                "CDC record numbers must be positive and strictly increasing".into(),
            ));
        }
        previous = change.record_number;
        if change.before.is_none() && change.after.is_none() {
            return Err(DbfError::Invalid(
                "CDC change must contain before or after state".into(),
            ));
        }
    }
    Ok(())
}
