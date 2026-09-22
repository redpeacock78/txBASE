use super::{DbfError, DbfRecord, DbfTable, MemoSnapshot, memo_format_for_version};

pub(super) fn merge_disjoint_rows(
    table: &mut DbfTable,
    current: &DbfTable,
) -> Result<(), DbfError> {
    let Some(source) = &table.source else {
        return Err(DbfError::Invalid(
            "row-level merge requires a path-backed snapshot".into(),
        ));
    };
    let source_header = DbfTable::from_bytes(&source.dbf)?.header;
    let source_memo = source
        .memo
        .clone()
        .map(|bytes| {
            let format = memo_format_for_version(source_header.version).ok_or_else(|| {
                DbfError::Invalid("row-level merge cannot determine memo format".into())
            })?;
            Ok::<MemoSnapshot, DbfError>(MemoSnapshot { format, bytes })
        })
        .transpose()?;
    let base = DbfTable::from_snapshot(super::mvcc::Snapshot {
        dbf: source.dbf.clone(),
        memo: source_memo,
        schema: source.schema.clone(),
        row_epoch: 1,
        row_changes: None,
    })?;
    if table.layout_changed
        || base.fields != current.fields
        || base.fields != table.fields
        || base.header.record_length != current.header.record_length
        || base.header.record_length != table.header.record_length
        || base.schema != current.schema
        || base.schema != table.schema
        || base.records.len() != current.records.len()
        || base.records.len() != table.records.len()
    {
        return Err(DbfError::Invalid(
            "row-level merge requires unchanged schema, layout, and record count".into(),
        ));
    }

    let mut merged = current.clone();
    for ((base_record, current_record), desired_record) in base
        .records
        .iter()
        .zip(&current.records)
        .zip(&table.records)
    {
        let current_changed = !same_record(base_record, current_record);
        let desired_changed = !same_record(base_record, desired_record);
        if !desired_changed {
            continue;
        }
        if current_changed && !same_record(current_record, desired_record) {
            return Err(DbfError::Invalid(format!(
                "row-level merge conflict at record {}",
                desired_record.number
            )));
        }
        if same_record(current_record, desired_record) {
            continue;
        }
        if desired_record.deleted {
            merged.delete_record(desired_record.number)?;
        } else {
            merged.replace_record(desired_record.number, desired_record.values.clone())?;
        }
    }
    *table = merged;
    Ok(())
}

fn same_record(left: &DbfRecord, right: &DbfRecord) -> bool {
    left.deleted == right.deleted && left.values == right.values
}
