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
    let left_matches = matching_ranges(&left_sorted, &right_sorted, left_records.len());
    let right_matches = matching_ranges(&right_sorted, &left_sorted, right_records.len());

    let mut output = Vec::new();
    match &request.join.kind {
        JoinType::Inner => {
            for (left_position, &left_record) in left_records.iter().enumerate() {
                let Some(right_range) = left_matches[left_position].as_ref() else {
                    continue;
                };
                for right_row in &right_sorted[right_range.start..right_range.end] {
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
            for (left_position, &left_record) in left_records.iter().enumerate() {
                if let Some(right_range) = left_matches[left_position].as_ref() {
                    for right_row in &right_sorted[right_range.start..right_range.end] {
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
            for (left_position, &left_record) in left_records.iter().enumerate() {
                if left_matches[left_position].is_some() {
                    emit(&mut output, request, Some(left_record), None)?;
                }
            }
        }
        JoinType::Anti => {
            for (left_position, &left_record) in left_records.iter().enumerate() {
                if left_matches[left_position].is_none() {
                    emit(&mut output, request, Some(left_record), None)?;
                }
            }
        }
        JoinType::Right => {
            for (right_position, &right_record) in right_records.iter().enumerate() {
                if let Some(left_range) = right_matches[right_position].as_ref() {
                    for left_row in &left_sorted[left_range.start..left_range.end] {
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
        JoinType::Full => {
            return Err(JoinError::Invalid(
                "merge join does not support full joins".into(),
            ));
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

fn matching_ranges(
    outer_sorted: &[OrderedRecord],
    inner_sorted: &[OrderedRecord],
    outer_count: usize,
) -> Vec<Option<Range<usize>>> {
    let mut matches = (0..outer_count).map(|_| None).collect::<Vec<_>>();
    let mut outer_start = 0;
    let mut inner_start = 0;
    while outer_start < outer_sorted.len() && inner_start < inner_sorted.len() {
        match compare_keys(
            &outer_sorted[outer_start].key,
            &inner_sorted[inner_start].key,
        ) {
            Ordering::Less => outer_start += 1,
            Ordering::Greater => inner_start += 1,
            Ordering::Equal => {
                let outer_end = key_end(outer_sorted, outer_start);
                let inner_end = key_end(inner_sorted, inner_start);
                for row in &outer_sorted[outer_start..outer_end] {
                    matches[row.position] = Some(inner_start..inner_end);
                }
                outer_start = outer_end;
                inner_start = inner_end;
            }
        }
    }
    matches
}

fn key_end(records: &[OrderedRecord], start: usize) -> usize {
    let key = &records[start].key;
    let mut end = start + 1;
    while end < records.len() && compare_keys(&records[end].key, key) == Ordering::Equal {
        end += 1;
    }
    end
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

#[cfg(test)]
mod tests {
    use super::{OrderedRecord, matching_ranges};
    use serde_json::json;

    fn ordered(position: usize, key: i64) -> OrderedRecord {
        OrderedRecord {
            position,
            key: vec![json!(key)],
        }
    }

    #[test]
    fn scans_equal_key_runs_and_maps_them_to_outer_positions() {
        let outer = vec![ordered(1, 1), ordered(3, 1), ordered(5, 3)];
        let inner = vec![ordered(0, 1), ordered(2, 1), ordered(4, 2)];

        let matches = matching_ranges(&outer, &inner, 6);

        assert_eq!(matches[1], Some(0..2));
        assert_eq!(matches[3], Some(0..2));
        assert_eq!(matches[5], None);
    }
}
