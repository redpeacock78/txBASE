use crate::dbf::DbfRecord;
use serde_json::{Map, Value};
use std::collections::BTreeMap;

pub(crate) fn project(record: &DbfRecord, projection: &BTreeMap<String, i8>) -> Value {
    if projection.is_empty() {
        return Value::Object(record.values.clone());
    }
    if projection.values().any(|value| *value == 1) {
        let mut values = Map::new();
        for (field, inclusion) in projection {
            if *inclusion != 1 {
                continue;
            }
            let Some(value) = field_value(&record.values, field) else {
                continue;
            };
            insert_projected_value(&mut values, &record.values, field, value);
        }
        return Value::Object(values);
    }
    let mut values = record.values.clone();
    for field in projection
        .iter()
        .filter_map(|(field, exclusion)| (*exclusion == 0).then_some(field))
    {
        remove_projected_value(&mut values, field);
    }
    Value::Object(values)
}

pub(crate) fn field_value(values: &Map<String, Value>, path: &str) -> Option<Value> {
    if let Some(value) = values.get(path) {
        return Some(value.clone());
    }
    let segments = path.split('.').collect::<Vec<_>>();
    let first = segments.first().copied()?;
    let value = values.get(first)?;
    let mut matches = Vec::new();
    collect_path_values(value, &segments[1..], &mut matches);
    match matches.len() {
        0 => None,
        1 => matches.pop(),
        _ => Some(Value::Array(matches)),
    }
}

fn collect_path_values(value: &Value, segments: &[&str], matches: &mut Vec<Value>) {
    if segments.is_empty() {
        matches.push(value.clone());
        return;
    }
    match value {
        Value::Object(values) => {
            if let Some(value) = values.get(segments[0]) {
                collect_path_values(value, &segments[1..], matches);
            }
        }
        Value::Array(values) => {
            if let Some(index) = array_index(segments[0]) {
                if let Some(value) = values.get(index) {
                    collect_path_values(value, &segments[1..], matches);
                }
            } else {
                for value in values {
                    collect_path_values(value, segments, matches);
                }
            }
        }
        _ => {}
    }
}

fn insert_projected_value(
    output: &mut Map<String, Value>,
    source: &Map<String, Value>,
    path: &str,
    value: Value,
) {
    if source.contains_key(path) || !path.contains('.') {
        output.insert(path.to_owned(), value);
        return;
    }
    let segments = path.split('.').collect::<Vec<_>>();
    insert_nested_value(output, Some(source), &segments, value);
}

fn insert_nested_value(
    output: &mut Map<String, Value>,
    source: Option<&Map<String, Value>>,
    segments: &[&str],
    value: Value,
) {
    let Some((first, rest)) = segments.split_first() else {
        return;
    };
    if rest.is_empty() {
        output.insert((*first).to_owned(), value);
        return;
    }
    let source_value = source.and_then(|source| source.get(*first));
    let use_array = source_value.is_some_and(Value::is_array) && array_index(rest[0]).is_some();
    let entry = output.entry((*first).to_owned()).or_insert_with(|| {
        if use_array {
            Value::Array(Vec::new())
        } else {
            Value::Object(Map::new())
        }
    });
    if use_array {
        if !entry.is_array() {
            *entry = Value::Array(Vec::new());
        }
        insert_nested_array(entry, source_value, rest, value);
    } else {
        if !entry.is_object() {
            *entry = Value::Object(Map::new());
        }
        insert_nested_value(
            entry.as_object_mut().expect("object was initialized"),
            source_value.and_then(Value::as_object),
            rest,
            value,
        );
    }
}

fn insert_nested_array(
    output: &mut Value,
    source: Option<&Value>,
    segments: &[&str],
    value: Value,
) {
    let Some(index) = segments.first().and_then(|segment| array_index(segment)) else {
        return;
    };
    let Some(source_values) = source.and_then(Value::as_array) else {
        return;
    };
    if index >= source_values.len() {
        return;
    }
    let Some(values) = output.as_array_mut() else {
        return;
    };
    values.resize_with(index.saturating_add(1), || Value::Null);
    let rest = &segments[1..];
    if rest.is_empty() {
        values[index] = value;
        return;
    }

    let source_value = source
        .and_then(Value::as_array)
        .and_then(|source| source.get(index));
    let use_array = source_value.is_some_and(Value::is_array) && array_index(rest[0]).is_some();
    if use_array {
        if !values[index].is_array() {
            values[index] = Value::Array(Vec::new());
        }
        insert_nested_array(&mut values[index], source_value, rest, value);
    } else {
        if !values[index].is_object() {
            values[index] = Value::Object(Map::new());
        }
        insert_nested_value(
            values[index]
                .as_object_mut()
                .expect("object was initialized"),
            source_value.and_then(Value::as_object),
            rest,
            value,
        );
    }
}

fn remove_projected_value(values: &mut Map<String, Value>, path: &str) {
    if values.remove(path).is_some() || !path.contains('.') {
        return;
    }
    let segments = path.split('.').collect::<Vec<_>>();
    remove_nested_value(values, &segments);
}

fn remove_nested_value(values: &mut Map<String, Value>, segments: &[&str]) {
    let Some((first, rest)) = segments.split_first() else {
        return;
    };
    if rest.is_empty() {
        values.remove(*first);
        return;
    }
    let Some(value) = values.get_mut(*first) else {
        return;
    };
    match value {
        Value::Object(values) => remove_nested_value(values, rest),
        Value::Array(values) => remove_array_value(values, rest),
        _ => {}
    }
}

fn remove_array_value(values: &mut [Value], segments: &[&str]) {
    let Some((segment, rest)) = segments.split_first() else {
        return;
    };
    if let Some(index) = array_index(segment) {
        let Some(value) = values.get_mut(index) else {
            return;
        };
        if rest.is_empty() {
            *value = Value::Null;
        } else {
            remove_nested_value_from_value(value, rest);
        }
        return;
    }
    for value in values {
        remove_nested_value_from_value(value, segments);
    }
}

fn remove_nested_value_from_value(value: &mut Value, segments: &[&str]) {
    if segments.is_empty() {
        return;
    }
    match value {
        Value::Object(values) => remove_nested_value(values, segments),
        Value::Array(values) => remove_array_value(values, segments),
        _ => {}
    }
}

fn array_index(segment: &str) -> Option<usize> {
    segment.parse().ok()
}
