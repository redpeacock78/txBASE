use super::mvcc::Snapshot;
use super::{DbfError, DbfRecord, DbfTable};
use crate::dbf::{RowId, RowVersion};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RowChange {
    pub(crate) epoch: u64,
    pub(crate) record_number: usize,
    pub(crate) deleted: bool,
    pub(crate) values: Map<String, Value>,
}

pub(crate) fn changes_for_commit(
    previous: Option<&Snapshot>,
    current: &Snapshot,
    force_new_epoch: bool,
) -> Result<Snapshot, DbfError> {
    let current_table = DbfTable::from_snapshot(current.clone())?;
    let Some(previous) = previous else {
        return Ok(with_changes(current, 1, current_rows(&current_table, 1)));
    };

    let previous_table = DbfTable::from_snapshot(previous.clone())?;
    let previous_epoch = effective_epoch(previous);
    let layout_changed = force_new_epoch
        || previous.schema != current.schema
        || previous_table.fields != current_table.fields
        || previous_table.header.record_length != current_table.header.record_length
        || current_table.records().len() < previous_table.records().len();
    let epoch = if layout_changed {
        previous_epoch
            .checked_add(1)
            .ok_or_else(|| DbfError::Invalid("row MVCC epoch exhausted".into()))?
    } else {
        previous_epoch
    };

    let mut changes = Vec::new();
    if layout_changed {
        changes.extend(previous_rows(&previous_table, previous_epoch));
        changes.extend(current_rows(&current_table, epoch));
    } else {
        for (index, record) in current_table.records().iter().enumerate() {
            let changed = previous_table
                .records()
                .get(index)
                .is_none_or(|previous| !same_row(previous, record));
            if changed {
                changes.push(row_change(record, epoch));
            }
        }
    }
    Ok(with_changes(current, epoch, changes))
}

pub(crate) fn compact_snapshots(snapshots: &mut [(u64, Snapshot)]) -> Result<(), DbfError> {
    let mut previous = None;
    for (_, snapshot) in snapshots.iter_mut() {
        let current = snapshot.clone();
        let force_new_epoch = previous.as_ref().is_some_and(|previous: &Snapshot| {
            current.row_epoch != 0 && current.row_epoch != effective_epoch(previous)
        });
        let rebuilt = changes_for_commit(previous.as_ref(), &current, force_new_epoch)?;
        *snapshot = rebuilt.clone();
        previous = Some(rebuilt);
    }
    Ok(())
}

pub(crate) fn row_history(
    snapshots: &BTreeMap<u64, Snapshot>,
    record_number: usize,
) -> Result<Vec<RowVersion>, DbfError> {
    validate_record_number(record_number)?;
    let mut versions = Vec::new();
    for (transaction_id, snapshot) in snapshots {
        for change in changes_for_snapshot(snapshot)? {
            if change.record_number != record_number {
                continue;
            }
            versions.push(to_version(*transaction_id, change));
        }
    }
    Ok(versions)
}

pub(crate) fn row_at(
    snapshots: &BTreeMap<u64, Snapshot>,
    transaction_id: u64,
    id: RowId,
) -> Result<Option<RowVersion>, DbfError> {
    if transaction_id == 0 {
        return Err(DbfError::Invalid(
            "MVCC transaction ID must be positive".into(),
        ));
    }
    validate_id(id)?;
    if !snapshots.contains_key(&transaction_id) {
        return Err(DbfError::Invalid(format!(
            "MVCC snapshot {transaction_id} is not committed"
        )));
    }
    let mut current = None;
    for (version, snapshot) in snapshots.range(..=transaction_id) {
        for change in changes_for_snapshot(snapshot)? {
            if change.epoch == id.epoch && change.record_number == id.record_number {
                current = Some(to_version(*version, change));
            }
        }
    }
    Ok(current)
}

fn changes_for_snapshot(snapshot: &Snapshot) -> Result<Vec<RowChange>, DbfError> {
    if let Some(changes) = &snapshot.row_changes {
        return Ok(changes.clone());
    }
    let table = DbfTable::from_snapshot(snapshot.clone())?;
    Ok(current_rows(&table, effective_epoch(snapshot)))
}

fn current_rows(table: &DbfTable, epoch: u64) -> Vec<RowChange> {
    table
        .records()
        .iter()
        .map(|record| row_change(record, epoch))
        .collect()
}

fn previous_rows(table: &DbfTable, epoch: u64) -> Vec<RowChange> {
    table
        .records()
        .iter()
        .map(|record| RowChange {
            epoch,
            record_number: record.number,
            deleted: true,
            values: record.values.clone(),
        })
        .collect()
}

fn row_change(record: &DbfRecord, epoch: u64) -> RowChange {
    RowChange {
        epoch,
        record_number: record.number,
        deleted: record.deleted,
        values: record.values.clone(),
    }
}

fn same_row(left: &DbfRecord, right: &DbfRecord) -> bool {
    left.deleted == right.deleted && left.values == right.values
}

fn with_changes(snapshot: &Snapshot, row_epoch: u64, changes: Vec<RowChange>) -> Snapshot {
    Snapshot {
        dbf: snapshot.dbf.clone(),
        memo: snapshot.memo.clone(),
        schema: snapshot.schema.clone(),
        row_epoch,
        row_changes: Some(changes),
    }
}

fn effective_epoch(snapshot: &Snapshot) -> u64 {
    snapshot.row_epoch.max(1)
}

fn to_version(transaction_id: u64, change: RowChange) -> RowVersion {
    RowVersion {
        id: RowId {
            epoch: change.epoch,
            record_number: change.record_number,
        },
        transaction_id,
        deleted: change.deleted,
        values: change.values,
    }
}

fn validate_record_number(record_number: usize) -> Result<(), DbfError> {
    if record_number == 0 {
        return Err(DbfError::Invalid(
            "MVCC record number must be positive".into(),
        ));
    }
    Ok(())
}

fn validate_id(id: RowId) -> Result<(), DbfError> {
    if id.epoch == 0 {
        return Err(DbfError::Invalid("MVCC row epoch must be positive".into()));
    }
    validate_record_number(id.record_number)
}
