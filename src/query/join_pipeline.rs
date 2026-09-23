use super::join::JoinSource;
use super::join::{JoinError, JoinRequest, JoinSpec, JoinType, MAX_JOIN_ROWS};
use super::join_strategy::JoinCostInput;
use super::matches_filter;
use crate::query_path::{field_value, project_values};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

mod stages;

pub(super) const MAX_JOIN_STAGES: usize = 8;

struct LoadedRows {
    values: Vec<Map<String, Value>>,
    numbers: Vec<usize>,
    page_reads: usize,
}

pub(super) fn validate(request: &JoinRequest) -> Result<(), JoinError> {
    if request.joins.len() >= MAX_JOIN_STAGES {
        return Err(JoinError::Invalid(format!(
            "join stages exceed {MAX_JOIN_STAGES}"
        )));
    }

    let mut available = vec![request.from.as_str()];
    for spec in std::iter::once(&request.join).chain(request.joins.iter()) {
        validate_spec(spec, &available)?;
        available.push(spec.table.as_str());
    }
    Ok(())
}

pub(super) fn execute(
    source: &dyn JoinSource,
    request: &JoinRequest,
) -> Result<Vec<Value>, JoinError> {
    validate(request)?;
    let initial = load_rows(source, &request.from)?;
    let mut rows = initial.values;
    let mut row_page_reads = initial.page_reads;
    for spec in std::iter::once(&request.join).chain(request.joins.iter()) {
        let right = load_rows(source, &spec.table)?;
        let (local_fields, foreign_fields) = stage_fields(spec);
        let cost_input = if matches!(&spec.kind, JoinType::Cross) {
            JoinCostInput::default()
        } else {
            JoinCostInput {
                outer_page_reads: row_page_reads,
                inner_page_reads: right.page_reads,
                output_rows: estimate_join_rows(
                    &rows,
                    &right.values,
                    &local_fields,
                    &foreign_fields,
                    &spec.kind,
                )?,
                output_columns: output_columns(&rows, &right.values, &spec.kind),
            }
        };
        let next_rows = stages::apply(
            source,
            rows,
            &right.values,
            &right.numbers,
            spec,
            cost_input,
        )?;
        row_page_reads = materialized_page_reads(&next_rows);
        rows = next_rows;
    }

    let mut output = Vec::new();
    for row in rows {
        if matches_filter(&row, &request.filter)? {
            output.push(project_values(&row, &request.projection));
        }
    }
    Ok(output)
}

fn validate_spec(spec: &JoinSpec, available: &[&str]) -> Result<(), JoinError> {
    if spec.table.is_empty() {
        return Err(JoinError::Invalid("join.table must not be empty".into()));
    }
    if available.iter().any(|table| *table == spec.table) {
        return Err(JoinError::Invalid(format!(
            "join table is already present: {}",
            spec.table
        )));
    }
    if matches!(&spec.kind, JoinType::Cross) {
        if !spec.on.is_empty() {
            return Err(JoinError::Invalid(
                "cross joins do not accept join.on conditions".into(),
            ));
        }
        return Ok(());
    }
    if spec.on.is_empty() {
        return Err(JoinError::Invalid(
            "join.on requires at least one equality condition".into(),
        ));
    }
    let foreign_tables = [spec.table.as_str()];
    for (local, condition) in &spec.on {
        validate_qualified_field(local, available, "join.on field")?;
        validate_qualified_field(
            &condition.equality.field,
            &foreign_tables,
            "join.on.$eq.$field",
        )?;
    }
    Ok(())
}

fn load_rows(source: &dyn JoinSource, table_name: &str) -> Result<LoadedRows, JoinError> {
    let table = source.open_table(table_name)?;
    let page_reads = table
        .byte_len()
        .div_ceil(crate::index::COST_PAGE_SIZE)
        .max(1);
    let mut values = Vec::new();
    let mut numbers = Vec::new();
    for record in table.active_records() {
        numbers.push(record.number);
        values.push(qualified_values(table_name, &record.values));
    }
    Ok(LoadedRows {
        values,
        numbers,
        page_reads,
    })
}

fn output_columns(
    left: &[Map<String, Value>],
    right: &[Map<String, Value>],
    join_type: &JoinType,
) -> usize {
    let left_columns = left.iter().map(Map::len).max().unwrap_or_default();
    if matches!(join_type, JoinType::Semi | JoinType::Anti) {
        return left_columns.max(1);
    }
    left_columns
        .saturating_add(right.iter().map(Map::len).max().unwrap_or_default())
        .max(1)
}

fn materialized_page_reads(rows: &[Map<String, Value>]) -> usize {
    let bytes = rows.iter().fold(0usize, |total, row| {
        total.saturating_add(
            serde_json::to_vec(row)
                .expect("join rows must be JSON serializable")
                .len(),
        )
    });
    bytes.div_ceil(crate::index::COST_PAGE_SIZE).max(1)
}

fn estimate_join_rows(
    left_rows: &[Map<String, Value>],
    right_rows: &[Map<String, Value>],
    local_fields: &[String],
    foreign_fields: &[String],
    join_type: &JoinType,
) -> Result<usize, JoinError> {
    if matches!(join_type, JoinType::Cross) {
        return Err(JoinError::Invalid(
            "cross joins do not use equality cardinality estimation".into(),
        ));
    }

    let mut left_counts = BTreeMap::<String, usize>::new();
    for row in left_rows {
        if let Some(key) = encoded_key(row, local_fields)? {
            let count = left_counts.entry(key).or_default();
            *count = count.saturating_add(1);
        }
    }
    let mut right_counts = BTreeMap::<String, usize>::new();
    for row in right_rows {
        if let Some(key) = encoded_key(row, foreign_fields)? {
            let count = right_counts.entry(key).or_default();
            *count = count.saturating_add(1);
        }
    }

    let mut matched_pairs = 0usize;
    let mut left_matched = 0usize;
    for (key, left_count) in &left_counts {
        if let Some(right_count) = right_counts.get(key) {
            matched_pairs = matched_pairs.saturating_add(left_count.saturating_mul(*right_count));
            left_matched = left_matched.saturating_add(*left_count);
        }
    }
    let mut right_matched = 0usize;
    for (key, right_count) in &right_counts {
        if left_counts.contains_key(key) {
            right_matched = right_matched.saturating_add(*right_count);
        }
    }
    let left_unmatched = left_rows.len().saturating_sub(left_matched);
    let right_unmatched = right_rows.len().saturating_sub(right_matched);

    Ok(match join_type {
        JoinType::Inner => matched_pairs,
        JoinType::Left => matched_pairs.saturating_add(left_unmatched),
        JoinType::Right => matched_pairs.saturating_add(right_unmatched),
        JoinType::Full => matched_pairs
            .saturating_add(left_unmatched)
            .saturating_add(right_unmatched),
        JoinType::Semi => left_matched,
        JoinType::Anti => left_unmatched,
        JoinType::Cross => unreachable!("cross join handled before cardinality estimation"),
    })
}

fn stage_fields(spec: &JoinSpec) -> (Vec<String>, Vec<String>) {
    spec.on
        .iter()
        .map(|(local, condition)| (local.clone(), condition.equality.field.clone()))
        .unzip()
}

fn unqualified_field<'a>(path: &'a str, table: &str) -> Option<&'a str> {
    let (qualified_table, field) = path.split_once('.')?;
    (qualified_table == table && !field.is_empty()).then_some(field)
}

pub(super) fn encoded_key(
    values: &Map<String, Value>,
    fields: &[String],
) -> Result<Option<String>, JoinError> {
    let mut key = Vec::with_capacity(fields.len());
    for field in fields {
        let Some(value) = qualified_value(values, field) else {
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

fn qualified_value(values: &Map<String, Value>, path: &str) -> Option<Value> {
    if let Some(value) = values.get(path) {
        return Some(value.clone());
    }
    let (table, field_path) = path.split_once('.')?;
    let mut segments = field_path.split('.');
    let first = segments.next()?;
    let root = values.get(&format!("{table}.{first}"))?;
    if segments.next().is_none() {
        return Some(root.clone());
    }
    let mut nested = Map::new();
    nested.insert(first.to_owned(), root.clone());
    field_value(&nested, field_path)
}

fn qualified_values(table: &str, values: &Map<String, Value>) -> Map<String, Value> {
    values
        .iter()
        .map(|(field, value)| (format!("{table}.{field}"), value.clone()))
        .collect()
}

pub(super) fn push_combined(
    output: &mut Vec<Map<String, Value>>,
    left: Option<&Map<String, Value>>,
    right: Option<&Map<String, Value>>,
) -> Result<(), JoinError> {
    if output.len() >= MAX_JOIN_ROWS {
        return Err(JoinError::Invalid(format!(
            "join result exceeds {MAX_JOIN_ROWS} rows"
        )));
    }
    let mut row = Map::new();
    if let Some(left) = left {
        row.extend(left.clone());
    }
    if let Some(right) = right {
        row.extend(right.clone());
    }
    output.push(row);
    Ok(())
}

fn validate_qualified_field(value: &str, tables: &[&str], path: &str) -> Result<(), JoinError> {
    for table in tables {
        let prefix = format!("{table}.");
        if let Some(field) = value.strip_prefix(&prefix) {
            if field.is_empty() {
                return Err(JoinError::Invalid(format!("{path} has an empty field")));
            }
            return Ok(());
        }
    }
    Err(JoinError::Invalid(format!(
        "{path} must reference one of the joined tables"
    )))
}

#[cfg(test)]
mod tests {
    use super::{JoinType, estimate_join_rows, output_columns};
    use serde_json::{Map, json};

    fn row(table: &str, key: Option<i64>) -> Map<String, serde_json::Value> {
        let mut values = Map::new();
        if let Some(key) = key {
            values.insert(format!("{table}.ID"), json!(key));
        }
        values
    }

    #[test]
    fn estimates_chained_stage_cardinality_from_qualified_keys() {
        let left = [
            row("left", Some(1)),
            row("left", Some(1)),
            row("left", Some(2)),
            row("left", None),
        ];
        let right = [row("right", Some(1)), row("right", Some(3))];
        let local_fields = vec!["left.ID".to_owned()];
        let foreign_fields = vec!["right.ID".to_owned()];

        assert_eq!(
            estimate_join_rows(
                &left,
                &right,
                &local_fields,
                &foreign_fields,
                &JoinType::Left,
            )
            .unwrap(),
            4
        );
        assert_eq!(output_columns(&left, &right, &JoinType::Inner), 2);
        assert_eq!(output_columns(&left, &right, &JoinType::Semi), 1);
    }

    #[test]
    fn rejects_cross_stage_cardinality_estimation() {
        let rows = [row("left", Some(1))];
        let fields = vec!["left.ID".to_owned()];

        assert!(estimate_join_rows(&rows, &rows, &fields, &fields, &JoinType::Cross).is_err());
    }
}
