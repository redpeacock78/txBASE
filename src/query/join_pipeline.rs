use super::join::JoinSource;
use super::join::{JoinError, JoinRequest, JoinSpec, JoinType, MAX_JOIN_ROWS};
use super::join_strategy::JoinCostInput;
use super::matches_filter;
use crate::query_path::{field_value, project_values};
use serde_json::{Map, Value};

mod cost;
mod stages;
#[cfg(not(target_arch = "wasm32"))]
mod stream;

#[cfg(not(target_arch = "wasm32"))]
pub(super) use stream::execute as stream;

pub(super) const MAX_JOIN_STAGES: usize = 8;
use cost::{estimate_join_rows, load_rows, materialized_page_reads, output_columns};

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
        (rows, row_page_reads) = apply_stage(source, rows, row_page_reads, spec)?;
    }

    let mut output = Vec::new();
    for row in rows {
        if matches_filter(&row, &request.filter)? {
            output.push(project_values(&row, &request.projection));
        }
    }
    Ok(output)
}

fn apply_stage(
    source: &dyn JoinSource,
    rows: Vec<Map<String, Value>>,
    row_page_reads: usize,
    spec: &JoinSpec,
) -> Result<(Vec<Map<String, Value>>, usize), JoinError> {
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
            ..JoinCostInput::default()
        }
    };
    let rows = stages::apply(
        source,
        rows,
        &right.values,
        &right.numbers,
        spec,
        cost_input,
    )?;
    let page_reads = materialized_page_reads(&rows);
    Ok((rows, page_reads))
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
