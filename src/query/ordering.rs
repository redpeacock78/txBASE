use crate::dbf::DbfRecord;
use crate::query_path::field_value;
use indexmap::IndexMap;
use serde::Serialize;
use serde_json::Value;
use std::cmp::Ordering;

pub(super) fn compare_records(
    left: &DbfRecord,
    right: &DbfRecord,
    sort: &IndexMap<String, i8>,
) -> Ordering {
    compare_records_from(left, right, sort, 0)
}

pub(super) fn sort_ordered_prefix(
    records: &mut [&DbfRecord],
    sort: &IndexMap<String, i8>,
    prefix_len: usize,
) {
    let Some((primary_field, _)) = prefix_len
        .checked_sub(1)
        .and_then(|index| sort.iter().nth(index))
    else {
        return;
    };
    let mut start = 0;
    while start < records.len() {
        let mut end = start + 1;
        while end < records.len()
            && compare_sort_field(records[start], records[end], primary_field).is_eq()
        {
            end += 1;
        }
        if end - start > 1 {
            records[start..end]
                .sort_by(|left, right| compare_records_from(left, right, sort, prefix_len));
        }
        start = end;
    }
}

pub(super) fn compare_values(left: &Value, right: &Value) -> Option<Ordering> {
    match (left, right) {
        (Value::Array(left), Value::Array(right)) => Some(json_text(left).cmp(&json_text(right))),
        (Value::Object(left), Value::Object(right)) => Some(json_text(left).cmp(&json_text(right))),
        _ => crate::json_order::compare_scalar_values(left, right),
    }
}

fn compare_records_from(
    left: &DbfRecord,
    right: &DbfRecord,
    sort: &IndexMap<String, i8>,
    skip: usize,
) -> Ordering {
    for (field, direction) in sort.iter().skip(skip) {
        let left_value = field_value(&left.values, field);
        let right_value = field_value(&right.values, field);
        let ordering = compare_for_sort(left_value.as_ref(), right_value.as_ref());
        if ordering != Ordering::Equal {
            return if *direction == 1 {
                ordering
            } else {
                ordering.reverse()
            };
        }
    }
    left.number.cmp(&right.number)
}

fn compare_sort_field(left: &DbfRecord, right: &DbfRecord, field: &str) -> Ordering {
    compare_for_sort(
        field_value(&left.values, field).as_ref(),
        field_value(&right.values, field).as_ref(),
    )
}

pub(super) fn compare_for_sort(left: Option<&Value>, right: Option<&Value>) -> Ordering {
    match (left, right) {
        (Some(left), Some(right)) => compare_values(left, right).unwrap_or_else(|| {
            type_rank(left)
                .cmp(&type_rank(right))
                .then_with(|| left.to_string().cmp(&right.to_string()))
        }),
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Less,
        (Some(_), None) => Ordering::Greater,
    }
}

fn json_text<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_default()
}

fn type_rank(value: &Value) -> u8 {
    match value {
        Value::Null => 0,
        Value::Bool(_) => 1,
        Value::Number(_) => 2,
        Value::String(_) => 3,
        Value::Array(_) => 4,
        Value::Object(_) => 5,
    }
}
