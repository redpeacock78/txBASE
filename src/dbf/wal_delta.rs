use super::super::find_memo_path;
use super::{DELTA_MAGIC, DbfError, DbfTable, MemoFormat, MemoSnapshot, RecoverySnapshot};
use std::fs;
use std::path::Path;

const DELTA_VERSION: u8 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
struct BytePatch {
    offset: usize,
    bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ByteDelta {
    base_length: u64,
    target_length: u64,
    base_hash: u64,
    target_hash: u64,
    patches: Vec<BytePatch>,
}

pub(crate) fn delta_payload(
    table: &DbfTable,
    path: &Path,
    memo: Option<&MemoSnapshot>,
    full_payload_length: usize,
) -> Result<Option<Vec<u8>>, DbfError> {
    let Some(source) = &table.source else {
        return Ok(None);
    };
    if source.path != path || (memo.is_some() && source.memo.is_none()) {
        return Ok(None);
    }

    let dbf_delta = ByteDelta::new(&source.dbf, &table.bytes)?;
    let mut payload = DELTA_MAGIC.to_vec();
    payload.push(DELTA_VERSION);
    encode_byte_delta(&mut payload, &dbf_delta)?;
    match memo {
        Some(memo) => {
            payload.push(1);
            payload.push(memo.format.tag());
            let base = source
                .memo
                .as_deref()
                .expect("memo presence was checked above");
            encode_byte_delta(&mut payload, &ByteDelta::new(base, &memo.bytes)?)?;
        }
        None => payload.push(0),
    }
    Ok((payload.len() < full_payload_length).then_some(payload))
}

impl ByteDelta {
    pub(crate) fn new(base: &[u8], target: &[u8]) -> Result<Self, DbfError> {
        let common_length = base.len().min(target.len());
        let mut patches = Vec::new();
        let mut offset = 0;
        while offset < common_length {
            if base[offset] == target[offset] {
                offset += 1;
                continue;
            }
            let start = offset;
            offset += 1;
            while offset < common_length && base[offset] != target[offset] {
                offset += 1;
            }
            patches.push(BytePatch {
                offset: start,
                bytes: target[start..offset].to_vec(),
            });
        }
        if target.len() > common_length {
            patches.push(BytePatch {
                offset: common_length,
                bytes: target[common_length..].to_vec(),
            });
        }
        Ok(Self {
            base_length: u64::try_from(base.len())
                .map_err(|_| DbfError::Invalid("WAL base length overflows u64".into()))?,
            target_length: u64::try_from(target.len())
                .map_err(|_| DbfError::Invalid("WAL target length overflows u64".into()))?,
            base_hash: delta_hash(base),
            target_hash: delta_hash(target),
            patches,
        })
    }
}

fn encode_byte_delta(payload: &mut Vec<u8>, delta: &ByteDelta) -> Result<(), DbfError> {
    payload.extend_from_slice(&delta.base_length.to_le_bytes());
    payload.extend_from_slice(&delta.target_length.to_le_bytes());
    payload.extend_from_slice(&delta.base_hash.to_le_bytes());
    payload.extend_from_slice(&delta.target_hash.to_le_bytes());
    let patch_count = u32::try_from(delta.patches.len())
        .map_err(|_| DbfError::Invalid("WAL patch count overflows u32".into()))?;
    payload.extend_from_slice(&patch_count.to_le_bytes());
    for patch in &delta.patches {
        payload.extend_from_slice(
            &u64::try_from(patch.offset)
                .map_err(|_| DbfError::Invalid("WAL patch offset overflows u64".into()))?
                .to_le_bytes(),
        );
        payload.extend_from_slice(
            &u32::try_from(patch.bytes.len())
                .map_err(|_| DbfError::Invalid("WAL patch length overflows u32".into()))?
                .to_le_bytes(),
        );
        payload.extend_from_slice(&patch.bytes);
    }
    Ok(())
}

pub(super) fn decode_delta_payload(
    path: &Path,
    payload: &[u8],
) -> Result<Option<RecoverySnapshot>, DbfError> {
    let mut cursor = DELTA_MAGIC.len();
    let version = take_delta_u8(payload, &mut cursor)?;
    if version != DELTA_VERSION {
        return Err(DbfError::Invalid(format!(
            "unknown DBF delta version {version}"
        )));
    }
    let dbf_delta = decode_byte_delta(payload, &mut cursor)?;
    let memo_present = take_delta_u8(payload, &mut cursor)?;
    let memo_delta = match memo_present {
        0 => None,
        1 => {
            let format = MemoFormat::from_tag(take_delta_u8(payload, &mut cursor)?)?;
            Some((format, decode_byte_delta(payload, &mut cursor)?))
        }
        value => {
            return Err(DbfError::Invalid(format!(
                "invalid DBF delta memo flag {value}"
            )));
        }
    };
    if cursor != payload.len() {
        return Err(DbfError::Invalid("DBF delta has trailing bytes".into()));
    }

    let current_dbf = fs::read(path)?;
    let dbf = apply_byte_delta(&current_dbf, &dbf_delta, "DBF")?;
    let memo = memo_delta
        .map(|(format, delta)| {
            let memo_path =
                find_memo_path(path).unwrap_or_else(|| path.with_extension(format.extension()));
            let current_memo = fs::read(memo_path)?;
            Ok::<MemoSnapshot, DbfError>(MemoSnapshot {
                format,
                bytes: apply_byte_delta(&current_memo, &delta, "memo sidecar")?,
            })
        })
        .transpose()?;
    Ok(Some(RecoverySnapshot { dbf, memo }))
}

fn decode_byte_delta(payload: &[u8], cursor: &mut usize) -> Result<ByteDelta, DbfError> {
    let base_length = take_delta_u64(payload, cursor)?;
    let target_length = take_delta_u64(payload, cursor)?;
    let base_hash = take_delta_u64(payload, cursor)?;
    let target_hash = take_delta_u64(payload, cursor)?;
    let patch_count = usize::try_from(take_delta_u32(payload, cursor)?)
        .map_err(|_| DbfError::Invalid("WAL patch count overflows usize".into()))?;
    let remaining = payload.len().saturating_sub(*cursor);
    if patch_count > remaining / 12 {
        return Err(DbfError::Invalid("WAL patch count is unreasonable".into()));
    }
    let mut patches = Vec::with_capacity(patch_count);
    for _ in 0..patch_count {
        let offset = usize::try_from(take_delta_u64(payload, cursor)?)
            .map_err(|_| DbfError::Invalid("WAL patch offset overflows usize".into()))?;
        let length = usize::try_from(take_delta_u32(payload, cursor)?)
            .map_err(|_| DbfError::Invalid("WAL patch length overflows usize".into()))?;
        let bytes = take_delta_bytes(payload, cursor, length)?.to_vec();
        patches.push(BytePatch { offset, bytes });
    }
    Ok(ByteDelta {
        base_length,
        target_length,
        base_hash,
        target_hash,
        patches,
    })
}

pub(crate) fn apply_byte_delta(
    current: &[u8],
    delta: &ByteDelta,
    label: &str,
) -> Result<Vec<u8>, DbfError> {
    let current_length = u64::try_from(current.len())
        .map_err(|_| DbfError::Invalid(format!("{label} length overflows u64")))?;
    let current_hash = delta_hash(current);
    if current_length == delta.target_length && current_hash == delta.target_hash {
        return Ok(current.to_vec());
    }
    if current_length != delta.base_length || current_hash != delta.base_hash {
        return Err(DbfError::Invalid(format!(
            "{label} does not match the DBF delta base"
        )));
    }
    let target_length = usize::try_from(delta.target_length)
        .map_err(|_| DbfError::Invalid(format!("{label} target length overflows usize")))?;
    let mut output = current.to_vec();
    output.resize(target_length, 0);
    let mut previous_end = 0;
    for patch in &delta.patches {
        let end = patch
            .offset
            .checked_add(patch.bytes.len())
            .ok_or_else(|| DbfError::Invalid(format!("{label} patch range overflows")))?;
        if patch.bytes.is_empty() || patch.offset < previous_end || end > output.len() {
            return Err(DbfError::Invalid(format!(
                "{label} contains an invalid patch range"
            )));
        }
        output[patch.offset..end].copy_from_slice(&patch.bytes);
        previous_end = end;
    }
    if delta_hash(&output) != delta.target_hash {
        return Err(DbfError::Invalid(format!(
            "{label} delta checksum does not match"
        )));
    }
    Ok(output)
}

fn take_delta_bytes<'a>(
    payload: &'a [u8],
    cursor: &mut usize,
    length: usize,
) -> Result<&'a [u8], DbfError> {
    let end = cursor
        .checked_add(length)
        .ok_or_else(|| DbfError::Invalid("WAL delta length overflows".into()))?;
    let bytes = payload
        .get(*cursor..end)
        .ok_or_else(|| DbfError::Invalid("WAL delta is truncated".into()))?;
    *cursor = end;
    Ok(bytes)
}

fn take_delta_u8(payload: &[u8], cursor: &mut usize) -> Result<u8, DbfError> {
    Ok(*take_delta_bytes(payload, cursor, 1)?
        .first()
        .expect("one-byte delta read is fixed"))
}

fn take_delta_u32(payload: &[u8], cursor: &mut usize) -> Result<u32, DbfError> {
    Ok(u32::from_le_bytes(
        take_delta_bytes(payload, cursor, 4)?
            .try_into()
            .expect("four-byte delta read is fixed"),
    ))
}

fn take_delta_u64(payload: &[u8], cursor: &mut usize) -> Result<u64, DbfError> {
    Ok(u64::from_le_bytes(
        take_delta_bytes(payload, cursor, 8)?
            .try_into()
            .expect("eight-byte delta read is fixed"),
    ))
}

fn delta_hash(bytes: &[u8]) -> u64 {
    // ponytail: deterministic FNV-1a keeps the WAL self-contained; replace with a
    // cryptographic digest only if untrusted WAL input becomes a requirement.
    bytes.iter().fold(0xcbf29ce484222325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    })
}
