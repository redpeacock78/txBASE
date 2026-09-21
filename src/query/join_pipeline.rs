use super::join::{JoinError, JoinRequest, JoinSpec, JoinType, MAX_JOIN_ROWS};
use super::matches_filter;
use crate::catalog::Catalog;
use crate::query_path::{field_value, project_values};
use serde_json::{Map, Value};

mod stages;

pub(super) const MAX_JOIN_STAGES: usize = 8;

struct LoadedRows {
    values: Vec<Map<String, Value>>,
    numbers: Vec<usize>,
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

pub(super) fn execute(catalog: &Catalog, request: &JoinRequest) -> Result<Vec<Value>, JoinError> {
    validate(request)?;
    let _lock = catalog.acquire_read_lock()?;
    let mut rows = load_rows(catalog, &request.from)?.values;
    for spec in std::iter::once(&request.join).chain(request.joins.iter()) {
        let right = load_rows(catalog, &spec.table)?;
        rows = stages::apply(catalog, rows, &right.values, &right.numbers, spec)?;
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

fn load_rows(catalog: &Catalog, table_name: &str) -> Result<LoadedRows, JoinError> {
    let table = catalog.open_table_unlocked(table_name)?;
    let mut values = Vec::new();
    let mut numbers = Vec::new();
    for record in table.active_records() {
        numbers.push(record.number);
        values.push(qualified_values(table_name, &record.values));
    }
    Ok(LoadedRows { values, numbers })
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
