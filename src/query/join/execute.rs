use super::super::matches_filter;
use super::super::{join_index, join_merge, join_nested, join_pipeline, join_strategy};
use super::{JoinError, JoinRequest, JoinType, MAX_JOIN_ROWS};
use crate::catalog::{Catalog, CatalogReadTransaction};
use crate::dbf::DbfRecord;
use crate::query_path::{field_value, project_values};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

pub(crate) trait JoinSource {
    fn is_historical(&self) -> bool;
    fn open_table(&self, name: &str) -> Result<crate::dbf::DbfTable, JoinError>;
    fn open_tables(
        &self,
        left: &str,
        right: &str,
    ) -> Result<(crate::dbf::DbfTable, crate::dbf::DbfTable), JoinError> {
        Ok((self.open_table(left)?, self.open_table(right)?))
    }
    fn catalog(&self) -> Option<&Catalog>;
}

impl JoinSource for Catalog {
    fn is_historical(&self) -> bool {
        self.is_historical()
    }

    fn open_table(&self, name: &str) -> Result<crate::dbf::DbfTable, JoinError> {
        self.open_table_unlocked(name).map_err(Into::into)
    }

    fn catalog(&self) -> Option<&Catalog> {
        Some(self)
    }
}

impl JoinSource for CatalogReadTransaction {
    fn is_historical(&self) -> bool {
        true
    }

    fn open_table(&self, name: &str) -> Result<crate::dbf::DbfTable, JoinError> {
        self.open_table(name).map_err(Into::into)
    }

    fn catalog(&self) -> Option<&Catalog> {
        None
    }
}

pub fn execute(catalog: &Catalog, request: &JoinRequest) -> Result<Vec<Value>, JoinError> {
    super::validate(request)?;
    let _lock = catalog.acquire_read_lock()?;
    execute_with_source(catalog, request)
}

pub(crate) fn execute_read_transaction(
    snapshot: &CatalogReadTransaction,
    request: &JoinRequest,
) -> Result<Vec<Value>, JoinError> {
    super::validate(request)?;
    execute_with_source(snapshot, request)
}

fn execute_with_source<S: JoinSource>(
    source: &S,
    request: &JoinRequest,
) -> Result<Vec<Value>, JoinError> {
    if !request.joins.is_empty() {
        return join_pipeline::execute(source, request);
    }
    let (local_fields, foreign_fields) = super::join_fields(request)?;
    let (left, right) = source.open_tables(&request.from, &request.join.table)?;
    let left_records = left.active_records().collect::<Vec<_>>();
    let right_records = right.active_records().collect::<Vec<_>>();

    if let JoinType::Cross = &request.join.kind {
        let pair_count = left_records
            .len()
            .checked_mul(right_records.len())
            .ok_or_else(|| {
                JoinError::Invalid("cross join candidate pair count overflows".into())
            })?;
        if pair_count > MAX_JOIN_ROWS {
            return Err(JoinError::Invalid(format!(
                "cross join candidate pairs exceed {MAX_JOIN_ROWS}"
            )));
        }
        let mut output = Vec::new();
        for &left_record in &left_records {
            for &right_record in &right_records {
                emit(&mut output, request, Some(left_record), Some(right_record))?;
            }
        }
        return Ok(output);
    }

    let current_catalog = source.catalog();
    let large_join = current_catalog.is_some()
        && !source.is_historical()
        && matches!(
            join_strategy::choose(left_records.len(), right_records.len(), false),
            join_strategy::JoinStrategy::Hash
        );
    let left_ordered = if large_join {
        current_catalog.and_then(|catalog| {
            join_index::load_ordered_fields(catalog, &request.from, &local_fields)
        })
    } else {
        None
    };
    let right_ordered = if large_join {
        current_catalog.and_then(|catalog| {
            join_index::load_ordered_fields(catalog, &request.join.table, &foreign_fields)
        })
    } else {
        None
    };
    let merge_available = left_ordered.is_some() && right_ordered.is_some();

    if let JoinType::Right = &request.join.kind {
        if matches!(
            join_strategy::choose_with_merge(
                left_records.len(),
                right_records.len(),
                false,
                merge_available,
            ),
            join_strategy::JoinStrategy::Merge
        ) {
            if let (Some(left_order), Some(right_order)) =
                (left_ordered.as_ref(), right_ordered.as_ref())
            {
                return join_merge::execute(
                    &left_records,
                    &right_records,
                    left_order,
                    right_order,
                    request,
                    &local_fields,
                    &foreign_fields,
                );
            }
        }
        let left_index = if large_join {
            current_catalog
                .and_then(|catalog| join_index::load_fields(catalog, &request.from, &local_fields))
        } else {
            None
        };
        if matches!(
            join_strategy::choose_with_probe_cost(
                right_records.len(),
                left_records.len(),
                left_index.as_ref().and_then(|index| {
                    join_index::equality_probe_cost(index, left_records.len(), &local_fields)
                }),
            ),
            join_strategy::JoinStrategy::IndexNestedLoop
        ) {
            if let Some(index) = left_index.as_ref() {
                if let Some(output) = join_index::execute_right_join(
                    &left_records,
                    &right_records,
                    request,
                    index,
                    &local_fields,
                    &foreign_fields,
                )? {
                    return Ok(output);
                }
            }
        }
        if matches!(
            join_strategy::choose(left_records.len(), right_records.len(), false),
            join_strategy::JoinStrategy::NestedLoop
        ) {
            return join_nested::execute_right_join(
                &left_records,
                &right_records,
                request,
                &local_fields,
                &foreign_fields,
            );
        }
        let mut left_by_key = BTreeMap::<String, Vec<&DbfRecord>>::new();
        for &record in &left_records {
            let Some(key) = encoded_key(&record.values, &local_fields)? else {
                continue;
            };
            left_by_key.entry(key).or_default().push(record);
        }

        let mut output = Vec::new();
        for &right_record in &right_records {
            let matches = encoded_key(&right_record.values, &foreign_fields)?
                .and_then(|key| left_by_key.get(&key));
            if let Some(matches) = matches {
                for left_record in matches {
                    emit(&mut output, request, Some(left_record), Some(right_record))?;
                }
            } else {
                emit(&mut output, request, None, Some(right_record))?;
            }
        }
        return Ok(output);
    }

    if matches!(
        join_strategy::choose_with_merge(
            left_records.len(),
            right_records.len(),
            false,
            merge_available,
        ),
        join_strategy::JoinStrategy::Merge
    ) {
        if let (Some(left_order), Some(right_order)) =
            (left_ordered.as_ref(), right_ordered.as_ref())
        {
            return join_merge::execute(
                &left_records,
                &right_records,
                left_order,
                right_order,
                request,
                &local_fields,
                &foreign_fields,
            );
        }
    }

    if let JoinType::Full = &request.join.kind {
        return join_nested::execute_full_join(
            &left_records,
            &right_records,
            request,
            &local_fields,
            &foreign_fields,
        );
    }

    let right_index = if large_join {
        current_catalog.and_then(|catalog| {
            join_index::load_fields(catalog, &request.join.table, &foreign_fields)
        })
    } else {
        None
    };
    if matches!(
        join_strategy::choose_with_probe_cost(
            left_records.len(),
            right_records.len(),
            right_index.as_ref().and_then(|index| {
                join_index::equality_probe_cost(index, right_records.len(), &foreign_fields)
            }),
        ),
        join_strategy::JoinStrategy::IndexNestedLoop
    ) {
        if let Some(index) = right_index.as_ref() {
            if let Some(output) = join_index::execute_join(
                &left_records,
                &right_records,
                request,
                index,
                &local_fields,
                &foreign_fields,
            )? {
                return Ok(output);
            }
        }
    }

    if matches!(
        join_strategy::choose(left_records.len(), right_records.len(), false),
        join_strategy::JoinStrategy::NestedLoop
    ) {
        return join_nested::execute_join(
            &left_records,
            &right_records,
            request,
            &local_fields,
            &foreign_fields,
        );
    }

    let mut right_by_key = BTreeMap::<String, Vec<&DbfRecord>>::new();
    for record in right_records {
        let Some(key) = encoded_key(&record.values, &foreign_fields)? else {
            continue;
        };
        right_by_key.entry(key).or_default().push(record);
    }

    let mut output = Vec::new();
    for left_record in left_records {
        let matches =
            encoded_key(&left_record.values, &local_fields)?.and_then(|key| right_by_key.get(&key));
        let had_matches = matches.is_some_and(|records| !records.is_empty());
        match &request.join.kind {
            JoinType::Inner => {
                if let Some(matches) = matches {
                    for right_record in matches {
                        emit(&mut output, request, Some(left_record), Some(right_record))?;
                    }
                }
            }
            JoinType::Left => {
                if let Some(matches) = matches {
                    for right_record in matches {
                        emit(&mut output, request, Some(left_record), Some(right_record))?;
                    }
                }
                if !had_matches {
                    emit(&mut output, request, Some(left_record), None)?;
                }
            }
            JoinType::Semi if had_matches => emit(&mut output, request, Some(left_record), None)?,
            JoinType::Anti if !had_matches => emit(&mut output, request, Some(left_record), None)?,
            JoinType::Semi | JoinType::Anti => {}
            JoinType::Right | JoinType::Full | JoinType::Cross => {
                unreachable!("join type handled above")
            }
        }
    }
    Ok(output)
}

pub(crate) fn encoded_key(
    values: &Map<String, Value>,
    fields: &[String],
) -> Result<Option<String>, JoinError> {
    let mut key = Vec::with_capacity(fields.len());
    for field in fields {
        let Some(value) = field_value(values, field) else {
            return Ok(None);
        };
        if value.is_null() {
            return Ok(None);
        }
        key.push(value);
    }
    serde_json::to_string(&key)
        .map(Some)
        .map_err(|error| JoinError::Invalid(format!("join key encoding failed: {error}")))
}

pub(crate) fn emit(
    output: &mut Vec<Value>,
    request: &JoinRequest,
    left: Option<&DbfRecord>,
    right: Option<&DbfRecord>,
) -> Result<(), JoinError> {
    let mut values = Map::new();
    if let Some(left) = left {
        add_qualified_values(&mut values, &request.from, &left.values);
    }
    if let Some(right) = right {
        add_qualified_values(&mut values, &request.join.table, &right.values);
    }
    if !matches_filter(&values, &request.filter)? {
        return Ok(());
    }
    if output.len() >= MAX_JOIN_ROWS {
        return Err(JoinError::Invalid(format!(
            "join result exceeds {MAX_JOIN_ROWS} rows"
        )));
    }
    output.push(project_values(&values, &request.projection));
    Ok(())
}

fn add_qualified_values(output: &mut Map<String, Value>, table: &str, values: &Map<String, Value>) {
    for (field, value) in values {
        output.insert(format!("{table}.{field}"), value.clone());
    }
}
