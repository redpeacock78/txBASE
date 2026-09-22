use super::CatalogError;
use crate::dbf::{ChangeRecord, DbfTable};
use crate::transaction::{FileWal, MAX_WAL_RECORD_SIZE, encode_records};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const PATH: &str = ".txbase.catalog.cdc";
const MAGIC: &[u8; 4] = b"TXCC";
const VERSION: u8 = 1;
const HEADER_SIZE: usize = MAGIC.len() + 1 + 4;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogChangeEvent {
    pub transaction_id: u64,
    pub tables: BTreeMap<String, CatalogTableChange>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogTableChange {
    pub reset: bool,
    pub changes: Vec<ChangeRecord>,
}

pub(crate) fn path_for(root: &Path) -> PathBuf {
    root.join(PATH)
}

pub(crate) fn event_for_tables(
    transaction_id: u64,
    before: &BTreeMap<String, DbfTable>,
    after: &BTreeMap<String, DbfTable>,
) -> Result<CatalogChangeEvent, CatalogError> {
    if transaction_id == 0 {
        return Err(CatalogError::Invalid(
            "catalog CDC transaction ID must be positive".into(),
        ));
    }
    let mut tables = BTreeMap::new();
    for (name, after_table) in after {
        let before_table = before.get(name).ok_or_else(|| {
            CatalogError::Invalid(format!("catalog CDC table is missing before state: {name}"))
        })?;
        let event =
            crate::dbf::cdc_event_for_tables(transaction_id, before_table, after_table, false)
                .map_err(|source| CatalogError::Table {
                    name: name.clone(),
                    source,
                })?;
        tables.insert(
            name.clone(),
            CatalogTableChange {
                reset: event.reset,
                changes: event.changes,
            },
        );
    }
    if tables.is_empty() {
        return Err(CatalogError::Invalid(
            "catalog CDC event must contain at least one table".into(),
        ));
    }
    Ok(CatalogChangeEvent {
        transaction_id,
        tables,
    })
}

pub(crate) fn staged_bytes(
    path: &Path,
    event: &CatalogChangeEvent,
) -> Result<Vec<u8>, CatalogError> {
    let encoded = payload(event)?;
    let mut records = Vec::new();
    if path.exists() {
        let wal = FileWal::open(path).map_err(transaction_error)?;
        let mut last_transaction_id = None;
        for (_, record) in wal.records() {
            let Some(existing) = decode_payload(record)? else {
                return Err(CatalogError::Invalid(
                    "catalog CDC log contains an unknown record".into(),
                ));
            };
            if last_transaction_id.is_some_and(|last| existing.transaction_id <= last) {
                return Err(CatalogError::Invalid(
                    "catalog CDC transaction IDs are not strictly increasing".into(),
                ));
            }
            if existing.transaction_id == event.transaction_id {
                if existing == *event {
                    return encode_records(
                        &wal.records()
                            .iter()
                            .map(|(_, r)| r.clone())
                            .collect::<Vec<_>>(),
                    )
                    .map_err(transaction_error);
                }
                return Err(CatalogError::Invalid(format!(
                    "catalog CDC transaction {} has conflicting event data",
                    event.transaction_id
                )));
            }
            last_transaction_id = Some(existing.transaction_id);
            records.push(record.clone());
        }
        if last_transaction_id.is_some_and(|last| last > event.transaction_id) {
            return Err(CatalogError::Invalid(format!(
                "catalog CDC transaction {} is older than the log tail",
                event.transaction_id
            )));
        }
    }
    records.push(encoded);
    encode_records(&records).map_err(transaction_error)
}

pub(crate) fn read(
    root: &Path,
    after: Option<u64>,
) -> Result<Vec<CatalogChangeEvent>, CatalogError> {
    let path = path_for(root);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let wal = FileWal::open(path).map_err(transaction_error)?;
    let mut events = Vec::new();
    let mut last_transaction_id = None;
    for (_, record) in wal.records() {
        let Some(event) = decode_payload(record)? else {
            return Err(CatalogError::Invalid(
                "catalog CDC log contains an unknown record".into(),
            ));
        };
        if last_transaction_id.is_some_and(|last| event.transaction_id <= last) {
            return Err(CatalogError::Invalid(
                "catalog CDC transaction IDs are not strictly increasing".into(),
            ));
        }
        last_transaction_id = Some(event.transaction_id);
        if after.is_none_or(|transaction_id| event.transaction_id > transaction_id) {
            events.push(event);
        }
    }
    Ok(events)
}

fn payload(event: &CatalogChangeEvent) -> Result<Vec<u8>, CatalogError> {
    validate_event(event)?;
    let body = serde_json::to_vec(event).map_err(|error| {
        CatalogError::Invalid(format!("catalog CDC event encoding failed: {error}"))
    })?;
    let length = u32::try_from(body.len())
        .map_err(|_| CatalogError::Invalid("catalog CDC event payload is too large".into()))?;
    let total = HEADER_SIZE.checked_add(body.len()).ok_or_else(|| {
        CatalogError::Invalid("catalog CDC event payload length overflows".into())
    })?;
    if total > MAX_WAL_RECORD_SIZE {
        return Err(CatalogError::Invalid(
            "catalog CDC event payload exceeds the WAL record limit".into(),
        ));
    }
    let mut bytes = Vec::with_capacity(total);
    bytes.extend_from_slice(MAGIC);
    bytes.push(VERSION);
    bytes.extend_from_slice(&length.to_le_bytes());
    bytes.extend_from_slice(&body);
    Ok(bytes)
}

fn decode_payload(payload: &[u8]) -> Result<Option<CatalogChangeEvent>, CatalogError> {
    let Some(body) = payload.strip_prefix(MAGIC) else {
        return Ok(None);
    };
    if body.len() < 5 {
        return Err(CatalogError::Invalid(
            "catalog CDC event header is truncated".into(),
        ));
    }
    if body[0] != VERSION {
        return Err(CatalogError::Invalid(format!(
            "unknown catalog CDC event version {}",
            body[0]
        )));
    }
    let length = usize::try_from(u32::from_le_bytes(
        body[1..5]
            .try_into()
            .expect("catalog CDC event length is fixed"),
    ))
    .map_err(|_| CatalogError::Invalid("catalog CDC event length overflows".into()))?;
    let encoded = body
        .get(5..)
        .filter(|encoded| encoded.len() == length)
        .ok_or_else(|| {
            CatalogError::Invalid("catalog CDC event length does not match payload".into())
        })?;
    let event = serde_json::from_slice(encoded).map_err(|error| {
        CatalogError::Invalid(format!("catalog CDC event decoding failed: {error}"))
    })?;
    validate_event(&event)?;
    Ok(Some(event))
}

fn validate_event(event: &CatalogChangeEvent) -> Result<(), CatalogError> {
    if event.transaction_id == 0 {
        return Err(CatalogError::Invalid(
            "catalog CDC transaction ID must be positive".into(),
        ));
    }
    if event.tables.is_empty() {
        return Err(CatalogError::Invalid(
            "catalog CDC event must contain at least one table".into(),
        ));
    }
    for (name, table) in &event.tables {
        if name.is_empty() {
            return Err(CatalogError::Invalid(
                "catalog CDC table name must not be empty".into(),
            ));
        }
        let mut previous = 0;
        for change in &table.changes {
            if change.record_number == 0 || change.record_number <= previous {
                return Err(CatalogError::Invalid(format!(
                    "catalog CDC record numbers for {name} must be positive and strictly increasing"
                )));
            }
            previous = change.record_number;
            if change.before.is_none() && change.after.is_none() {
                return Err(CatalogError::Invalid(
                    "catalog CDC change must contain before or after state".into(),
                ));
            }
        }
    }
    Ok(())
}

fn transaction_error(error: crate::transaction::TransactionError) -> CatalogError {
    CatalogError::Invalid(format!("catalog CDC WAL error: {error}"))
}
