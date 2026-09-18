#[path = "wal_delta.rs"]
mod delta;

use super::{DbfError, DbfTable, MemoFormat, MemoSnapshot};
use crate::xbase::OperationIr;
use std::path::Path;

pub(super) use delta::delta_payload;
#[cfg(test)]
pub(super) use delta::{ByteDelta, apply_byte_delta};

pub(super) const SNAPSHOT_MAGIC: &[u8; 4] = b"TXDB";
pub(super) const MEMO_SNAPSHOT_MAGIC: &[u8; 4] = b"TXDM";
pub(super) const DELTA_MAGIC: &[u8; 4] = b"TXDP";
pub(super) const OPERATION_MAGIC: &[u8; 4] = b"TXOP";
const OPERATION_VERSION: u8 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RecoverySnapshot {
    pub(super) dbf: Vec<u8>,
    pub(super) memo: Option<MemoSnapshot>,
}

pub(super) fn snapshot_payload(dbf: &[u8]) -> Vec<u8> {
    let mut payload = SNAPSHOT_MAGIC.to_vec();
    payload.extend_from_slice(dbf);
    payload
}

pub(super) fn operation_payload(operation: &OperationIr) -> Result<Vec<u8>, DbfError> {
    let body = serde_json::to_vec(operation)
        .map_err(|error| DbfError::Invalid(format!("operation WAL encoding failed: {error}")))?;
    let length = u32::try_from(body.len())
        .map_err(|_| DbfError::Invalid("operation WAL payload is too large".into()))?;
    let mut payload = OPERATION_MAGIC.to_vec();
    payload.push(OPERATION_VERSION);
    payload.extend_from_slice(&length.to_le_bytes());
    payload.extend_from_slice(&body);
    Ok(payload)
}

pub(super) fn memo_snapshot_payload(dbf: &[u8], memo: &MemoSnapshot) -> Result<Vec<u8>, DbfError> {
    let dbf_length = u64::try_from(dbf.len())
        .map_err(|_| DbfError::Invalid("DBF snapshot length overflows u64".into()))?;
    let memo_length = u64::try_from(memo.bytes.len())
        .map_err(|_| DbfError::Invalid("memo snapshot length overflows u64".into()))?;
    let mut payload = MEMO_SNAPSHOT_MAGIC.to_vec();
    payload.push(memo.format.tag());
    payload.extend_from_slice(&dbf_length.to_le_bytes());
    payload.extend_from_slice(&memo_length.to_le_bytes());
    payload.extend_from_slice(dbf);
    payload.extend_from_slice(&memo.bytes);
    Ok(payload)
}

pub(super) fn decode_wal_payload(
    path: &Path,
    payload: &[u8],
) -> Result<Option<RecoverySnapshot>, DbfError> {
    if payload.starts_with(DELTA_MAGIC) {
        delta::decode_delta_payload(path, payload)
    } else if payload.starts_with(OPERATION_MAGIC) {
        decode_operation_payload(payload)?;
        Ok(None)
    } else {
        decode_snapshot(payload)
    }
}

pub(super) fn decode_operation_payload(payload: &[u8]) -> Result<Option<OperationIr>, DbfError> {
    let Some(body) = payload.strip_prefix(OPERATION_MAGIC) else {
        return Ok(None);
    };
    let version = *body
        .first()
        .ok_or_else(|| DbfError::Invalid("operation WAL header is truncated".into()))?;
    if version != OPERATION_VERSION {
        return Err(DbfError::Invalid(format!(
            "unknown operation WAL version {version}"
        )));
    }
    let length = body
        .get(1..5)
        .ok_or_else(|| DbfError::Invalid("operation WAL length is truncated".into()))?;
    let length = usize::try_from(u32::from_le_bytes(
        length.try_into().expect("operation WAL length is fixed"),
    ))
    .map_err(|_| DbfError::Invalid("operation WAL length overflows usize".into()))?;
    let encoded = body
        .get(5..)
        .filter(|encoded| encoded.len() == length)
        .ok_or_else(|| DbfError::Invalid("operation WAL length does not match payload".into()))?;
    serde_json::from_slice(encoded)
        .map(Some)
        .map_err(|error| DbfError::Invalid(format!("operation WAL decoding failed: {error}")))
}

pub(super) fn decode_snapshot(payload: &[u8]) -> Result<Option<RecoverySnapshot>, DbfError> {
    if let Some(dbf) = payload.strip_prefix(SNAPSHOT_MAGIC) {
        return Ok(Some(RecoverySnapshot {
            dbf: dbf.to_vec(),
            memo: None,
        }));
    }
    let Some(body) = payload.strip_prefix(MEMO_SNAPSHOT_MAGIC) else {
        return Ok(None);
    };
    let header = body
        .get(..17)
        .ok_or_else(|| DbfError::Invalid("memo snapshot header is truncated".into()))?;
    let format = MemoFormat::from_tag(header[0])?;
    let dbf_length = usize::try_from(u64::from_le_bytes(
        header[1..9]
            .try_into()
            .expect("memo snapshot DBF length is fixed"),
    ))
    .map_err(|_| DbfError::Invalid("DBF snapshot length overflows usize".into()))?;
    let memo_length = usize::try_from(u64::from_le_bytes(
        header[9..17]
            .try_into()
            .expect("memo snapshot memo length is fixed"),
    ))
    .map_err(|_| DbfError::Invalid("memo snapshot length overflows usize".into()))?;
    let memo_start = 17usize
        .checked_add(dbf_length)
        .ok_or_else(|| DbfError::Invalid("memo snapshot length overflows usize".into()))?;
    let end = memo_start
        .checked_add(memo_length)
        .ok_or_else(|| DbfError::Invalid("memo snapshot length overflows usize".into()))?;
    if end != body.len() {
        return Err(DbfError::Invalid(
            "memo snapshot length does not match payload".into(),
        ));
    }
    Ok(Some(RecoverySnapshot {
        dbf: body[17..memo_start].to_vec(),
        memo: Some(MemoSnapshot {
            format,
            bytes: body[memo_start..end].to_vec(),
        }),
    }))
}
