use super::mvcc::{Record, Snapshot};
use super::row_mvcc::RowChange;
use super::{DbfError, MemoFormat, MemoSnapshot};
use serde_json::{Map, Value};

const MVCC_MAGIC: &[u8; 4] = b"TXMV";
const LEGACY_MVCC_VERSION: u8 = 1;
const MVCC_VERSION: u8 = 2;
const ROW_HISTORY_VERSION: u8 = 3;
const PREPARE_KIND: u8 = 0;
const COMMIT_KIND: u8 = 1;
const ROW_HISTORY_KIND: u8 = 2;
const NO_MEMO: u8 = 0xff;
const LEGACY_PREPARE_HEADER_SIZE: usize = 39;
const PREPARE_HEADER_SIZE: usize = 51;
const ROW_HISTORY_HEADER_SIZE: usize = 18;
const ROW_CHANGE_HEADER_SIZE: usize = 25;
const COMMIT_RECORD_SIZE: usize = 14;

pub(crate) fn encode_prepare(
    transaction_id: u64,
    snapshot: &Snapshot,
) -> Result<Vec<u8>, DbfError> {
    if transaction_id == 0 {
        return Err(DbfError::Invalid(
            "MVCC transaction ID must be positive".into(),
        ));
    }
    let dbf_length = u64::try_from(snapshot.dbf.len())
        .map_err(|_| DbfError::Invalid("MVCC DBF snapshot length overflows u64".into()))?;
    let (memo_tag, memo_bytes) = snapshot.memo.as_ref().map_or((NO_MEMO, &[][..]), |memo| {
        (memo.format.tag(), memo.bytes.as_slice())
    });
    let memo_length = u64::try_from(memo_bytes.len())
        .map_err(|_| DbfError::Invalid("MVCC memo snapshot length overflows u64".into()))?;
    let schema_bytes = snapshot.schema.as_deref().unwrap_or_default();
    let schema_length = u64::try_from(schema_bytes.len())
        .map_err(|_| DbfError::Invalid("MVCC schema snapshot length overflows u64".into()))?;
    let row_epoch = snapshot.row_epoch.max(1);
    let row_changes = snapshot.row_changes.as_deref().unwrap_or_default();
    let row_count = u32::try_from(row_changes.len())
        .map_err(|_| DbfError::Invalid("MVCC row-change count overflows u32".into()))?;
    let mut payload = Vec::with_capacity(
        PREPARE_HEADER_SIZE
            .saturating_add(row_changes.len().saturating_mul(ROW_CHANGE_HEADER_SIZE))
            .saturating_add(snapshot.dbf.len())
            .saturating_add(memo_bytes.len())
            .saturating_add(schema_bytes.len()),
    );
    payload.extend_from_slice(MVCC_MAGIC);
    payload.extend_from_slice(&[MVCC_VERSION, PREPARE_KIND]);
    payload.extend_from_slice(&transaction_id.to_le_bytes());
    payload.extend_from_slice(&dbf_length.to_le_bytes());
    payload.push(memo_tag);
    payload.extend_from_slice(&memo_length.to_le_bytes());
    payload.extend_from_slice(&schema_length.to_le_bytes());
    payload.extend_from_slice(&row_epoch.to_le_bytes());
    payload.extend_from_slice(&row_count.to_le_bytes());
    append_row_changes(&mut payload, row_changes)?;
    payload.extend_from_slice(&snapshot.dbf);
    payload.extend_from_slice(memo_bytes);
    payload.extend_from_slice(schema_bytes);
    Ok(payload)
}

pub(crate) fn encode_row_history(
    transaction_id: u64,
    changes: &[RowChange],
) -> Result<Vec<u8>, DbfError> {
    if transaction_id == 0 {
        return Err(DbfError::Invalid(
            "MVCC transaction ID must be positive".into(),
        ));
    }
    if changes.is_empty() {
        return Err(DbfError::Invalid(
            "MVCC row history must contain a change".into(),
        ));
    }
    let row_count = u32::try_from(changes.len())
        .map_err(|_| DbfError::Invalid("MVCC row-change count overflows u32".into()))?;
    let mut payload = Vec::with_capacity(
        ROW_HISTORY_HEADER_SIZE
            .saturating_add(changes.len().saturating_mul(ROW_CHANGE_HEADER_SIZE)),
    );
    payload.extend_from_slice(MVCC_MAGIC);
    payload.extend_from_slice(&[ROW_HISTORY_VERSION, ROW_HISTORY_KIND]);
    payload.extend_from_slice(&transaction_id.to_le_bytes());
    payload.extend_from_slice(&row_count.to_le_bytes());
    append_row_changes(&mut payload, changes)?;
    Ok(payload)
}

pub(crate) fn encode_commit(transaction_id: u64) -> Vec<u8> {
    let mut payload = Vec::with_capacity(COMMIT_RECORD_SIZE);
    payload.extend_from_slice(MVCC_MAGIC);
    payload.extend_from_slice(&[MVCC_VERSION, COMMIT_KIND]);
    payload.extend_from_slice(&transaction_id.to_le_bytes());
    payload
}

pub(crate) fn decode_record(payload: &[u8]) -> Result<Record, DbfError> {
    if !payload.starts_with(MVCC_MAGIC) {
        return Err(DbfError::Invalid("MVCC record has an invalid magic".into()));
    }
    let version = *payload
        .get(4)
        .ok_or_else(|| DbfError::Invalid("MVCC record header is truncated".into()))?;
    if !matches!(
        version,
        LEGACY_MVCC_VERSION | MVCC_VERSION | ROW_HISTORY_VERSION
    ) {
        return Err(DbfError::Invalid(format!(
            "unknown MVCC record version {version}"
        )));
    }
    let kind = *payload
        .get(5)
        .ok_or_else(|| DbfError::Invalid("MVCC record kind is truncated".into()))?;
    let transaction_id = read_u64(payload, 6)?;
    if transaction_id == 0 {
        return Err(DbfError::Invalid(
            "MVCC transaction ID must be positive".into(),
        ));
    }
    match kind {
        COMMIT_KIND => {
            if payload.len() != COMMIT_RECORD_SIZE {
                return Err(DbfError::Invalid(
                    "MVCC commit record length is invalid".into(),
                ));
            }
            Ok(Record::Commit { transaction_id })
        }
        PREPARE_KIND => decode_prepare(payload, transaction_id, version),
        ROW_HISTORY_KIND => decode_row_history(payload, transaction_id, version),
        _ => Err(DbfError::Invalid(format!(
            "unknown MVCC record kind {kind}"
        ))),
    }
}

fn append_row_changes(payload: &mut Vec<u8>, row_changes: &[RowChange]) -> Result<(), DbfError> {
    for change in row_changes {
        if change.epoch == 0 || change.record_number == 0 {
            return Err(DbfError::Invalid(
                "MVCC row changes require positive epoch and record number".into(),
            ));
        }
        let values = serde_json::to_vec(&change.values).map_err(|error| {
            DbfError::Invalid(format!("MVCC row-change encoding failed: {error}"))
        })?;
        let values_length = u64::try_from(values.len())
            .map_err(|_| DbfError::Invalid("MVCC row values length overflows u64".into()))?;
        payload.extend_from_slice(&change.epoch.to_le_bytes());
        payload.extend_from_slice(
            &u64::try_from(change.record_number)
                .map_err(|_| DbfError::Invalid("MVCC record number overflows u64".into()))?
                .to_le_bytes(),
        );
        payload.push(u8::from(change.deleted));
        payload.extend_from_slice(&values_length.to_le_bytes());
        payload.extend_from_slice(&values);
    }
    Ok(())
}

fn decode_prepare(payload: &[u8], transaction_id: u64, version: u8) -> Result<Record, DbfError> {
    let header_size = if version == LEGACY_MVCC_VERSION {
        LEGACY_PREPARE_HEADER_SIZE
    } else {
        PREPARE_HEADER_SIZE
    };
    if payload.len() < header_size {
        return Err(DbfError::Invalid(
            "MVCC prepare record header is truncated".into(),
        ));
    }
    let (row_epoch, row_changes, data_start) = if version == LEGACY_MVCC_VERSION {
        (0, None, LEGACY_PREPARE_HEADER_SIZE)
    } else {
        let row_epoch = read_u64(payload, LEGACY_PREPARE_HEADER_SIZE)?;
        if row_epoch == 0 {
            return Err(DbfError::Invalid("MVCC row epoch must be positive".into()));
        }
        let row_count = usize::try_from(u32::from_le_bytes(
            payload[LEGACY_PREPARE_HEADER_SIZE + 8..PREPARE_HEADER_SIZE]
                .try_into()
                .expect("MVCC row-change count is fixed"),
        ))
        .map_err(|_| DbfError::Invalid("MVCC row-change count overflows usize".into()))?;
        let (changes, cursor) = decode_row_changes(payload, PREPARE_HEADER_SIZE, row_count)?;
        (row_epoch, Some(changes), cursor)
    };
    let dbf_length = usize_from_u64(read_u64(payload, 14)?, "DBF")?;
    let memo_tag = payload[22];
    let memo_length = usize_from_u64(read_u64(payload, 23)?, "memo")?;
    let schema_length = usize_from_u64(read_u64(payload, 31)?, "schema")?;
    let data_end = data_start
        .checked_add(dbf_length)
        .and_then(|end| end.checked_add(memo_length))
        .and_then(|end| end.checked_add(schema_length))
        .ok_or_else(|| DbfError::Invalid("MVCC prepare record length overflows".into()))?;
    if data_end != payload.len() {
        return Err(DbfError::Invalid(
            "MVCC prepare lengths do not match the record".into(),
        ));
    }
    let dbf_end = data_start + dbf_length;
    let memo_end = dbf_end + memo_length;
    let memo = if memo_tag == NO_MEMO {
        if memo_length != 0 {
            return Err(DbfError::Invalid(
                "MVCC memo tag is absent but memo bytes are present".into(),
            ));
        }
        None
    } else {
        Some(MemoSnapshot {
            format: MemoFormat::from_tag(memo_tag)?,
            bytes: payload[dbf_end..memo_end].to_vec(),
        })
    };
    let schema = (schema_length != 0).then(|| payload[memo_end..].to_vec());
    Ok(Record::Prepare {
        transaction_id,
        snapshot: Snapshot {
            dbf: payload[data_start..dbf_end].to_vec(),
            memo,
            schema,
            row_epoch,
            row_changes,
        },
    })
}

fn decode_row_history(
    payload: &[u8],
    transaction_id: u64,
    version: u8,
) -> Result<Record, DbfError> {
    if version != ROW_HISTORY_VERSION || payload.len() < ROW_HISTORY_HEADER_SIZE {
        return Err(DbfError::Invalid(
            "MVCC row-history record header is invalid".into(),
        ));
    }
    let row_count = usize::try_from(u32::from_le_bytes(
        payload[14..ROW_HISTORY_HEADER_SIZE]
            .try_into()
            .expect("MVCC row-history count is fixed"),
    ))
    .map_err(|_| DbfError::Invalid("MVCC row-change count overflows usize".into()))?;
    if row_count == 0 {
        return Err(DbfError::Invalid(
            "MVCC row-history record must contain a change".into(),
        ));
    }
    let (changes, cursor) = decode_row_changes(payload, ROW_HISTORY_HEADER_SIZE, row_count)?;
    if cursor != payload.len() {
        return Err(DbfError::Invalid(
            "MVCC row-history record has trailing bytes".into(),
        ));
    }
    Ok(Record::RowHistory {
        transaction_id,
        changes,
    })
}

fn decode_row_changes(
    payload: &[u8],
    mut cursor: usize,
    row_count: usize,
) -> Result<(Vec<RowChange>, usize), DbfError> {
    let remaining = payload.len().saturating_sub(cursor);
    if row_count > remaining / ROW_CHANGE_HEADER_SIZE {
        return Err(DbfError::Invalid(
            "MVCC row-change count is unreasonable".into(),
        ));
    }
    let mut changes = Vec::with_capacity(row_count);
    for _ in 0..row_count {
        let epoch = read_u64(payload, cursor)?;
        let record_number = usize_from_u64(read_u64(payload, cursor + 8)?, "record number")?;
        if epoch == 0 || record_number == 0 {
            return Err(DbfError::Invalid(
                "MVCC row change has a non-positive identity".into(),
            ));
        }
        let deleted = match payload.get(cursor + 16).copied() {
            Some(0) => false,
            Some(1) => true,
            Some(value) => {
                return Err(DbfError::Invalid(format!(
                    "MVCC row change has an invalid deletion flag {value}"
                )));
            }
            None => {
                return Err(DbfError::Invalid(
                    "MVCC row-change header is truncated".into(),
                ));
            }
        };
        let values_length = usize_from_u64(read_u64(payload, cursor + 17)?, "row values")?;
        let values_start = cursor
            .checked_add(ROW_CHANGE_HEADER_SIZE)
            .ok_or_else(|| DbfError::Invalid("MVCC row-change offset overflows".into()))?;
        let values_end = values_start
            .checked_add(values_length)
            .ok_or_else(|| DbfError::Invalid("MVCC row values length overflows".into()))?;
        let values: Map<String, Value> = serde_json::from_slice(
            payload
                .get(values_start..values_end)
                .ok_or_else(|| DbfError::Invalid("MVCC row values are truncated".into()))?,
        )
        .map_err(|error| DbfError::Invalid(format!("invalid MVCC row values: {error}")))?;
        changes.push(RowChange {
            epoch,
            record_number,
            deleted,
            values,
        });
        cursor = values_end;
    }
    Ok((changes, cursor))
}

fn read_u64(bytes: &[u8], start: usize) -> Result<u64, DbfError> {
    let end = start
        .checked_add(8)
        .ok_or_else(|| DbfError::Invalid("MVCC integer offset overflows".into()))?;
    bytes
        .get(start..end)
        .ok_or_else(|| DbfError::Invalid("MVCC integer is truncated".into()))
        .map(|value| u64::from_le_bytes(value.try_into().expect("MVCC integer is fixed")))
}

fn usize_from_u64(value: u64, name: &str) -> Result<usize, DbfError> {
    usize::try_from(value)
        .map_err(|_| DbfError::Invalid(format!("MVCC {name} snapshot length overflows usize")))
}
