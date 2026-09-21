use super::lock::TableLock;
use super::schema_metadata::{SchemaMetadata, schema_metadata_bytes};
use super::{
    DbfError, DbfTable, MemoFile, MemoFormat, MemoSnapshot, find_memo_path,
    memo_format_for_version, sync_parent_directory, transaction_error,
};
use crate::transaction::{FileWal, Wal};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use super::row_mvcc::{self, RowChange};

const MVCC_MAGIC: &[u8; 4] = b"TXMV";
const LEGACY_MVCC_VERSION: u8 = 1;
const MVCC_VERSION: u8 = 2;
const PREPARE_KIND: u8 = 0;
const COMMIT_KIND: u8 = 1;
const NO_MEMO: u8 = 0xff;
const LEGACY_PREPARE_HEADER_SIZE: usize = 39;
const PREPARE_HEADER_SIZE: usize = 51;
const ROW_CHANGE_HEADER_SIZE: usize = 25;
const COMMIT_RECORD_SIZE: usize = 14;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Snapshot {
    pub(crate) dbf: Vec<u8>,
    pub(crate) memo: Option<MemoSnapshot>,
    pub(crate) schema: Option<Vec<u8>>,
    pub(crate) row_epoch: u64,
    pub(crate) row_changes: Option<Vec<RowChange>>,
}

pub(crate) type CatalogSnapshotParts = (Vec<u8>, Option<(u8, Vec<u8>)>, Option<Vec<u8>>);

#[derive(Debug, Clone, PartialEq)]
enum Record {
    Prepare {
        transaction_id: u64,
        snapshot: Snapshot,
    },
    Commit {
        transaction_id: u64,
    },
}

pub(super) fn path_for(path: &Path) -> PathBuf {
    path.with_extension("txbase.mvcc")
}

pub(super) fn prepare_snapshot(
    path: &Path,
    transaction_id: u64,
    dbf: &[u8],
    memo: Option<&MemoSnapshot>,
    schema: Option<&[u8]>,
    force_new_epoch: bool,
) -> Result<(), DbfError> {
    let records = read_records(path)?;
    let previous = committed_snapshots(&records)?.into_iter().next_back();
    let base = Snapshot {
        dbf: dbf.to_vec(),
        memo: memo.cloned(),
        schema: schema.map(ToOwned::to_owned),
        row_epoch: previous
            .as_ref()
            .map(|(_, snapshot)| snapshot.row_epoch)
            .unwrap_or(1),
        row_changes: Some(Vec::new()),
    };
    let snapshot = row_mvcc::changes_for_commit(
        previous.as_ref().map(|(_, snapshot)| snapshot),
        &base,
        force_new_epoch,
    )?;
    if let Some(existing) = prepared_snapshot(&records, transaction_id) {
        if existing != snapshot {
            return Err(DbfError::Invalid(format!(
                "MVCC prepare for transaction {transaction_id} does not match"
            )));
        }
        return Ok(());
    }
    append_record(path, &encode_prepare(transaction_id, &snapshot)?)
}

pub(super) fn commit_snapshot(
    path: &Path,
    transaction_id: u64,
    dbf: &[u8],
    memo: Option<&MemoSnapshot>,
    schema: Option<&[u8]>,
    force_new_epoch: bool,
) -> Result<(), DbfError> {
    prepare_snapshot(path, transaction_id, dbf, memo, schema, force_new_epoch)?;
    let records = read_records(path)?;
    if committed_snapshots(&records)?.contains_key(&transaction_id) {
        return Ok(());
    }
    append_record(path, &encode_commit(transaction_id))
}

pub(super) fn commit_recovered_snapshot(
    path: &Path,
    transaction_id: u64,
    dbf: &[u8],
    memo: Option<&MemoSnapshot>,
    force_new_epoch: bool,
) -> Result<(), DbfError> {
    let memo = match memo {
        Some(memo) => Some(memo.clone()),
        None => current_memo_snapshot(path, dbf)?,
    };
    let schema = schema_metadata_bytes(path)?;
    commit_snapshot(
        path,
        transaction_id,
        dbf,
        memo.as_ref(),
        schema.as_deref(),
        force_new_epoch,
    )
}

pub(super) fn versions(path: &Path) -> Result<Vec<u64>, DbfError> {
    let records = read_records(path)?;
    Ok(committed_snapshots(&records)?.into_keys().collect())
}

pub(super) fn gc(path: &Path, keep_last: usize) -> Result<Vec<u64>, DbfError> {
    if keep_last == 0 {
        return Err(DbfError::Invalid(
            "MVCC GC keep count must be positive".into(),
        ));
    }
    let history_path = path_for(path);
    if !history_path.exists() {
        return Ok(Vec::new());
    }
    let records = read_records(path)?;
    let snapshots = committed_snapshots(&records)?;
    let mut retained = snapshots
        .into_iter()
        .rev()
        .take(keep_last)
        .collect::<Vec<_>>();
    retained.reverse();
    row_mvcc::compact_snapshots(&mut retained)?;
    let retained_ids = retained
        .iter()
        .map(|(transaction_id, _)| *transaction_id)
        .collect::<Vec<_>>();

    let temporary = history_path.with_extension("txbase.mvcc.gc.tmp");
    let _ = fs::remove_file(&temporary);
    let result = (|| {
        let mut wal = FileWal::open(&temporary).map_err(transaction_error)?;
        for (transaction_id, snapshot) in &retained {
            let prepare = encode_prepare(*transaction_id, snapshot)?;
            wal.append(&prepare).map_err(transaction_error)?;
            wal.append(&encode_commit(*transaction_id))
                .map_err(transaction_error)?;
        }
        wal.sync().map_err(transaction_error)?;
        drop(wal);
        replace_history(&temporary, &history_path)?;
        sync_parent_directory(&history_path)?;
        Ok(retained_ids)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

pub(super) fn snapshot_at(path: &Path, transaction_id: u64) -> Result<Snapshot, DbfError> {
    if transaction_id == 0 {
        return Err(DbfError::Invalid(
            "MVCC transaction ID must be positive".into(),
        ));
    }
    let records = read_records(path)?;
    let snapshots = committed_snapshots(&records)?;
    snapshots.get(&transaction_id).cloned().ok_or_else(|| {
        let available = snapshots
            .keys()
            .map(u64::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        DbfError::Invalid(format!(
            "MVCC snapshot {transaction_id} is not committed (available: [{available}])"
        ))
    })
}

impl DbfTable {
    pub fn from_path_at(path: impl AsRef<Path>, transaction_id: u64) -> Result<Self, DbfError> {
        let path = path.as_ref();
        let _lock = TableLock::acquire(path)?;
        Self::recover_wal_with_encoding(path, None)?;
        let _ = super::schema_export::recover_schema_export_locked(path)?;
        let snapshot = snapshot_at(path, transaction_id)?;
        Self::from_mvcc_snapshot(transaction_id, snapshot)
    }

    pub(crate) fn from_catalog_snapshot(
        dbf: &[u8],
        memo: Option<(u8, Vec<u8>)>,
        schema: Option<&[u8]>,
    ) -> Result<Self, DbfError> {
        let memo = match memo {
            Some((tag, bytes)) => Some(MemoSnapshot {
                format: MemoFormat::from_tag(tag)?,
                bytes,
            }),
            None => None,
        };
        let mut table = Self::from_snapshot(Snapshot {
            dbf: dbf.to_vec(),
            memo,
            schema: schema.map(ToOwned::to_owned),
            row_epoch: 1,
            row_changes: None,
        })?;
        table.historical_snapshot = true;
        Ok(table)
    }

    pub fn mvcc_versions(path: impl AsRef<Path>) -> Result<Vec<u64>, DbfError> {
        let path = path.as_ref();
        let _lock = TableLock::acquire(path)?;
        Self::recover_wal_with_encoding(path, None)?;
        let _ = super::schema_export::recover_schema_export_locked(path)?;
        versions(path)
    }

    pub fn mvcc_row_versions(
        path: impl AsRef<Path>,
        record_number: usize,
    ) -> Result<Vec<super::RowVersion>, DbfError> {
        let path = path.as_ref();
        let _lock = TableLock::acquire(path)?;
        Self::recover_wal_with_encoding(path, None)?;
        let _ = super::schema_export::recover_schema_export_locked(path)?;
        let snapshots = committed_snapshots(&read_records(path)?)?;
        row_mvcc::row_history(&snapshots, record_number)
    }

    pub fn mvcc_read_row(
        path: impl AsRef<Path>,
        transaction_id: u64,
        id: super::RowId,
    ) -> Result<Option<super::RowVersion>, DbfError> {
        let path = path.as_ref();
        let _lock = TableLock::acquire(path)?;
        Self::recover_wal_with_encoding(path, None)?;
        let _ = super::schema_export::recover_schema_export_locked(path)?;
        let snapshots = committed_snapshots(&read_records(path)?)?;
        row_mvcc::row_at(&snapshots, transaction_id, id)
    }

    pub fn gc_mvcc(path: impl AsRef<Path>, keep_last: usize) -> Result<Vec<u64>, DbfError> {
        let path = path.as_ref();
        let _lock = TableLock::acquire(path)?;
        Self::recover_wal_with_encoding(path, None)?;
        let _ = super::schema_export::recover_schema_export_locked(path)?;
        gc(path, keep_last)
    }

    pub(crate) fn catalog_snapshot_parts(
        &self,
        path: &Path,
    ) -> Result<CatalogSnapshotParts, DbfError> {
        Ok((
            self.bytes.clone(),
            self.memo
                .as_ref()
                .map(|memo| (memo.format.tag(), memo.bytes.clone())),
            schema_metadata_bytes(path)?,
        ))
    }

    fn from_mvcc_snapshot(transaction_id: u64, snapshot: Snapshot) -> Result<Self, DbfError> {
        let mut table = Self::from_snapshot(snapshot)?;
        table.transaction_id = Some(transaction_id);
        table.historical_snapshot = true;
        Ok(table)
    }

    pub(crate) fn from_snapshot(snapshot: Snapshot) -> Result<Self, DbfError> {
        let schema = snapshot
            .schema
            .as_deref()
            .map(SchemaMetadata::from_bytes)
            .transpose()?;
        let encoding = schema.as_ref().and_then(SchemaMetadata::encoding);
        let mut table = Self::from_bytes_with_encoding(&snapshot.dbf, encoding)?;
        if let Some(metadata) = &schema {
            metadata.validate_fields(&table.fields)?;
            table.schema = Some(metadata.clone());
        }
        if table.has_sidecar_fields() {
            let memo = snapshot.memo.ok_or_else(|| {
                DbfError::Invalid("MVCC snapshot is missing its memo sidecar".into())
            })?;
            let memo = MemoFile::from_bytes(memo.bytes, memo.format)?;
            table.resolve_memos(&memo)?;
            table.memo = Some(memo);
        }
        if let Some(metadata) = &table.schema {
            metadata.validate_records(&table.records)?;
        }
        table.source = None;
        Ok(table)
    }
}

fn prepared_snapshot(records: &[Record], transaction_id: u64) -> Option<Snapshot> {
    records.iter().rev().find_map(|record| match record {
        Record::Prepare {
            transaction_id: id,
            snapshot,
        } if *id == transaction_id => Some(snapshot.clone()),
        _ => None,
    })
}

fn committed_snapshots(records: &[Record]) -> Result<BTreeMap<u64, Snapshot>, DbfError> {
    let mut prepared = BTreeMap::new();
    let mut committed = BTreeSet::new();
    for record in records {
        match record {
            Record::Prepare {
                transaction_id,
                snapshot,
            } => {
                prepared.insert(*transaction_id, snapshot.clone());
            }
            Record::Commit { transaction_id } => {
                if !prepared.contains_key(transaction_id) {
                    return Err(DbfError::Invalid(format!(
                        "MVCC commit {transaction_id} has no prepare record"
                    )));
                }
                committed.insert(*transaction_id);
            }
        }
    }
    Ok(committed
        .into_iter()
        .filter_map(|transaction_id| {
            prepared
                .remove(&transaction_id)
                .map(|snapshot| (transaction_id, snapshot))
        })
        .collect())
}

fn read_records(path: &Path) -> Result<Vec<Record>, DbfError> {
    let path = path_for(path);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let wal = FileWal::open(path).map_err(transaction_error)?;
    wal.records()
        .iter()
        .map(|(_, payload)| decode_record(payload))
        .collect()
}

fn append_record(path: &Path, payload: &[u8]) -> Result<(), DbfError> {
    let history_path = path_for(path);
    let mut wal = FileWal::open(history_path).map_err(transaction_error)?;
    wal.append(payload).map_err(transaction_error)?;
    wal.sync().map_err(transaction_error)
}

fn replace_history(source: &Path, destination: &Path) -> Result<(), DbfError> {
    #[cfg(windows)]
    if destination.exists() {
        fs::remove_file(destination)?;
    }
    fs::rename(source, destination)?;
    Ok(())
}

fn encode_prepare(transaction_id: u64, snapshot: &Snapshot) -> Result<Vec<u8>, DbfError> {
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
    payload.extend_from_slice(&snapshot.dbf);
    payload.extend_from_slice(memo_bytes);
    payload.extend_from_slice(schema_bytes);
    Ok(payload)
}

fn encode_commit(transaction_id: u64) -> Vec<u8> {
    let mut payload = Vec::with_capacity(COMMIT_RECORD_SIZE);
    payload.extend_from_slice(MVCC_MAGIC);
    payload.extend_from_slice(&[MVCC_VERSION, COMMIT_KIND]);
    payload.extend_from_slice(&transaction_id.to_le_bytes());
    payload
}

fn decode_record(payload: &[u8]) -> Result<Record, DbfError> {
    if !payload.starts_with(MVCC_MAGIC) {
        return Err(DbfError::Invalid("MVCC record has an invalid magic".into()));
    }
    let version = *payload
        .get(4)
        .ok_or_else(|| DbfError::Invalid("MVCC record header is truncated".into()))?;
    if !matches!(version, LEGACY_MVCC_VERSION | MVCC_VERSION) {
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
        _ => Err(DbfError::Invalid(format!(
            "unknown MVCC record kind {kind}"
        ))),
    }
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
        let remaining = payload.len().saturating_sub(PREPARE_HEADER_SIZE);
        if row_count > remaining / ROW_CHANGE_HEADER_SIZE {
            return Err(DbfError::Invalid(
                "MVCC row-change count is unreasonable".into(),
            ));
        }
        let mut cursor = PREPARE_HEADER_SIZE;
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

fn current_memo_snapshot(path: &Path, dbf: &[u8]) -> Result<Option<MemoSnapshot>, DbfError> {
    let Some(memo_path) = find_memo_path(path) else {
        return Ok(None);
    };
    let Some(version) = dbf.first().copied() else {
        return Err(DbfError::Invalid("MVCC DBF snapshot is empty".into()));
    };
    let format = if memo_path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("fpt"))
    {
        MemoFormat::FoxPro
    } else {
        memo_format_for_version(version).ok_or_else(|| {
            DbfError::Invalid("MVCC memo sidecar has an unsupported DBF version".into())
        })?
    };
    Ok(Some(MemoSnapshot {
        format,
        bytes: fs::read(memo_path)?,
    }))
}
