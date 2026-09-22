use super::lock::TableLock;
use super::mvcc_codec;
use super::schema_metadata::{SchemaMetadata, schema_metadata_bytes};
use super::{
    DbfError, DbfTable, MemoFile, MemoFormat, MemoSnapshot, find_memo_path,
    memo_format_for_version, sync_parent_directory, transaction_error,
};
use crate::transaction::{FileWal, Wal};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use super::row_mvcc::{self, RowChange};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Snapshot {
    pub(crate) dbf: Vec<u8>,
    pub(crate) memo: Option<MemoSnapshot>,
    pub(crate) schema: Option<Vec<u8>>,
    pub(crate) row_epoch: u64,
    pub(crate) row_changes: Option<Vec<RowChange>>,
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) type CatalogSnapshotParts = (Vec<u8>, Option<(u8, Vec<u8>)>, Option<Vec<u8>>);

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Record {
    Prepare {
        transaction_id: u64,
        snapshot: Snapshot,
    },
    Commit {
        transaction_id: u64,
    },
    RowHistory {
        transaction_id: u64,
        changes: Vec<RowChange>,
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
    append_record(
        path,
        &mvcc_codec::encode_prepare(transaction_id, &snapshot)?,
    )
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
    append_record(path, &mvcc_codec::encode_commit(transaction_id))
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
    gc_with_row_retention(path, keep_last, None)
}

pub(super) fn gc_with_row_retention(
    path: &Path,
    keep_last: usize,
    keep_rows: Option<usize>,
) -> Result<Vec<u64>, DbfError> {
    if keep_last == 0 {
        return Err(DbfError::Invalid(
            "MVCC GC keep count must be positive".into(),
        ));
    }
    if keep_rows == Some(0) {
        return Err(DbfError::Invalid(
            "MVCC row retention count must be positive".into(),
        ));
    }
    let history_path = path_for(path);
    if !history_path.exists() {
        return Ok(Vec::new());
    }
    let records = read_records(path)?;
    let snapshots = committed_snapshots(&records)?;
    let changes_by_transaction = committed_row_changes(&records)?;
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
    let detached = match (keep_rows, retained.first()) {
        (Some(keep_rows), Some((first_retained, _))) => {
            row_mvcc::detached_history(&changes_by_transaction, *first_retained, keep_rows)
        }
        _ => BTreeMap::new(),
    };

    let temporary = history_path.with_extension("txbase.mvcc.gc.tmp");
    let _ = fs::remove_file(&temporary);
    let result = (|| {
        let mut wal = FileWal::open(&temporary).map_err(transaction_error)?;
        for (transaction_id, changes) in &detached {
            wal.append(&mvcc_codec::encode_row_history(*transaction_id, changes)?)
                .map_err(transaction_error)?;
        }
        for (transaction_id, snapshot) in &retained {
            let prepare = mvcc_codec::encode_prepare(*transaction_id, snapshot)?;
            wal.append(&prepare).map_err(transaction_error)?;
            wal.append(&mvcc_codec::encode_commit(*transaction_id))
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
    pub(crate) fn mark_read_only(&mut self) {
        self.historical_snapshot = true;
        self.source = None;
    }

    pub fn from_path_at(path: impl AsRef<Path>, transaction_id: u64) -> Result<Self, DbfError> {
        let path = path.as_ref();
        let _lock = TableLock::acquire(path)?;
        Self::recover_wal_with_encoding(path, None)?;
        let _ = super::schema_export::recover_schema_export_locked(path)?;
        let snapshot = snapshot_at(path, transaction_id)?;
        Self::from_mvcc_snapshot(transaction_id, snapshot)
    }

    #[cfg(not(target_arch = "wasm32"))]
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
        let changes = committed_row_changes(&read_records(path)?)?;
        row_mvcc::row_history(&changes, record_number)
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
        let changes = committed_row_changes(&read_records(path)?)?;
        row_mvcc::row_at(&changes, transaction_id, id)
    }

    pub fn gc_mvcc(path: impl AsRef<Path>, keep_last: usize) -> Result<Vec<u64>, DbfError> {
        let path = path.as_ref();
        let _lock = TableLock::acquire(path)?;
        Self::recover_wal_with_encoding(path, None)?;
        let _ = super::schema_export::recover_schema_export_locked(path)?;
        gc(path, keep_last)
    }

    pub fn gc_mvcc_with_row_retention(
        path: impl AsRef<Path>,
        keep_last: usize,
        keep_rows: usize,
    ) -> Result<Vec<u64>, DbfError> {
        let path = path.as_ref();
        let _lock = TableLock::acquire(path)?;
        Self::recover_wal_with_encoding(path, None)?;
        let _ = super::schema_export::recover_schema_export_locked(path)?;
        gc_with_row_retention(path, keep_last, Some(keep_rows))
    }

    #[cfg(not(target_arch = "wasm32"))]
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
            Record::RowHistory { .. } => {}
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

fn committed_row_changes(records: &[Record]) -> Result<BTreeMap<u64, Vec<RowChange>>, DbfError> {
    let snapshots = committed_snapshots(records)?;
    let mut changes = BTreeMap::new();
    for (transaction_id, snapshot) in snapshots {
        changes.insert(transaction_id, row_mvcc::changes_for_snapshot(&snapshot)?);
    }
    for record in records {
        let Record::RowHistory {
            transaction_id,
            changes: row_changes,
        } = record
        else {
            continue;
        };
        if changes
            .insert(*transaction_id, row_changes.clone())
            .is_some()
        {
            return Err(DbfError::Invalid(format!(
                "MVCC row history transaction {transaction_id} is duplicated"
            )));
        }
    }
    Ok(changes)
}

fn read_records(path: &Path) -> Result<Vec<Record>, DbfError> {
    let path = path_for(path);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let wal = FileWal::open(path).map_err(transaction_error)?;
    wal.records()
        .iter()
        .map(|(_, payload)| mvcc_codec::decode_record(payload))
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
