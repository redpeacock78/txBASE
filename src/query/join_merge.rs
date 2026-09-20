use super::join::{JoinError, JoinRequest, JoinType, emit};
use crate::dbf::DbfRecord;
use crate::json_order::compare_scalar_values;
use crate::query_path::field_value;
use serde_json::{Map, Value};
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::ops::Range;

struct OrderedRecord {
    position: usize,
    key: Vec<Value>,
}

pub(super) fn execute(
    left_records: &[&DbfRecord],
    right_records: &[&DbfRecord],
    left_order: &[usize],
    right_order: &[usize],
    request: &JoinRequest,
    local_fields: &[String],
    foreign_fields: &[String],
) -> Result<Vec<Value>, JoinError> {
    let left_sorted = ordered_records(left_records, left_order, local_fields)?;
    let right_sorted = ordered_records(right_records, right_order, foreign_fields)?;

    let mut output = Vec::new();
    match &request.join.kind {
        JoinType::Inner => {
            for &left_record in left_records {
                let Some(key) = merge_key(&left_record.values, local_fields) else {
                    continue;
                };
                let Some(right_range) = equal_range(&right_sorted, &key) else {
                    continue;
                };
                for right_row in &right_sorted[right_range] {
                    emit(
                        &mut output,
                        request,
                        Some(left_record),
                        Some(right_records[right_row.position]),
                    )?;
                }
            }
        }
        JoinType::Left => {
            for &left_record in left_records {
                let Some(key) = merge_key(&left_record.values, local_fields) else {
                    emit(&mut output, request, Some(left_record), None)?;
                    continue;
                };
                if let Some(right_range) = equal_range(&right_sorted, &key) {
                    for right_row in &right_sorted[right_range] {
                        emit(
                            &mut output,
                            request,
                            Some(left_record),
                            Some(right_records[right_row.position]),
                        )?;
                    }
                } else {
                    emit(&mut output, request, Some(left_record), None)?;
                }
            }
        }
        JoinType::Semi => {
            for &left_record in left_records {
                if merge_key(&left_record.values, local_fields)
                    .and_then(|key| equal_range(&right_sorted, &key))
                    .is_some()
                {
                    emit(&mut output, request, Some(left_record), None)?;
                }
            }
        }
        JoinType::Anti => {
            for &left_record in left_records {
                if merge_key(&left_record.values, local_fields)
                    .and_then(|key| equal_range(&right_sorted, &key))
                    .is_none()
                {
                    emit(&mut output, request, Some(left_record), None)?;
                }
            }
        }
        JoinType::Right => {
            for &right_record in right_records {
                let Some(key) = merge_key(&right_record.values, foreign_fields) else {
                    emit(&mut output, request, None, Some(right_record))?;
                    continue;
                };
                if let Some(left_range) = equal_range(&left_sorted, &key) {
                    for left_row in &left_sorted[left_range] {
                        emit(
                            &mut output,
                            request,
                            Some(left_records[left_row.position]),
                            Some(right_record),
                        )?;
                    }
                } else {
                    emit(&mut output, request, None, Some(right_record))?;
                }
            }
        }
        JoinType::Cross => return Err(JoinError::Invalid("merge join cannot be cross".into())),
    }
    Ok(output)
}

fn ordered_records(
    records: &[&DbfRecord],
    order: &[usize],
    fields: &[String],
) -> Result<Vec<OrderedRecord>, JoinError> {
    let by_number = records
        .iter()
        .enumerate()
        .map(|(position, record)| (record.number, (position, *record)))
        .collect::<BTreeMap<_, _>>();
    let mut ordered = Vec::with_capacity(order.len());
    for record_number in order {
        let Some((position, record)) = by_number.get(record_number).copied() else {
            return Err(JoinError::Invalid(format!(
                "join index references missing active record: {record_number}"
            )));
        };
        let Some(key) = merge_key(&record.values, fields) else {
            continue;
        };
        ordered.push(OrderedRecord { position, key });
    }
    Ok(ordered)
}

fn merge_key(values: &Map<String, Value>, fields: &[String]) -> Option<Vec<Value>> {
    fields
        .iter()
        .map(|field| {
            let value = field_value(values, field)?;
            matches!(&value, Value::Bool(_) | Value::Number(_) | Value::String(_)).then_some(value)
        })
        .collect()
}

fn equal_range(records: &[OrderedRecord], key: &[Value]) -> Option<Range<usize>> {
    let start = lower_bound(records, key, |ordering| ordering.is_lt());
    if start == records.len() || compare_keys(&records[start].key, key) != Ordering::Equal {
        return None;
    }
    let end = lower_bound(&records[start..], key, |ordering| !ordering.is_gt()) + start;
    Some(start..end)
}

fn lower_bound(
    records: &[OrderedRecord],
    key: &[Value],
    before: impl Fn(Ordering) -> bool,
) -> usize {
    let mut start = 0;
    let mut end = records.len();
    while start < end {
        let middle = start + (end - start) / 2;
        if before(compare_keys(&records[middle].key, key)) {
            start = middle + 1;
        } else {
            end = middle;
        }
    }
    start
}

fn compare_keys(left: &[Value], right: &[Value]) -> Ordering {
    left.iter()
        .zip(right)
        .map(|(left, right)| compare_values(left, right))
        .find(|ordering| !ordering.is_eq())
        .unwrap_or_else(|| left.len().cmp(&right.len()))
}

fn compare_values(left: &Value, right: &Value) -> Ordering {
    compare_scalar_values(left, right).unwrap_or_else(|| value_rank(left).cmp(&value_rank(right)))
}

fn value_rank(value: &Value) -> u8 {
    match value {
        Value::Bool(_) => 1,
        Value::Number(_) => 2,
        Value::String(_) => 3,
        Value::Null => 0,
        Value::Array(_) => 4,
        Value::Object(_) => 5,
    }
}
