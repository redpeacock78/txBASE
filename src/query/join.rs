use super::{QueryError, matches_filter};
use crate::catalog::{Catalog, CatalogError};
use crate::dbf::DbfRecord;
use crate::query_path::{field_value, project_values};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt::{self, Display, Formatter};

pub const MAX_JOIN_ROWS: usize = 100_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JoinType {
    Inner,
    Left,
    Semi,
    Anti,
    Cross,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JoinRequest {
    pub from: String,
    pub join: JoinSpec,
    #[serde(default)]
    pub filter: Map<String, Value>,
    #[serde(default)]
    pub projection: BTreeMap<String, i8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JoinSpec {
    #[serde(rename = "type")]
    pub kind: JoinType,
    pub table: String,
    pub on: BTreeMap<String, JoinCondition>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JoinCondition {
    #[serde(rename = "$eq")]
    pub equality: JoinField,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JoinField {
    #[serde(rename = "$field")]
    pub field: String,
}

#[derive(Debug)]
pub enum JoinError {
    InvalidJson(serde_json::Error),
    Catalog(CatalogError),
    Query(QueryError),
    Invalid(String),
}

impl Display for JoinError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidJson(error) => write!(formatter, "invalid join JSON: {error}"),
            Self::Catalog(error) => write!(formatter, "join catalog error: {error}"),
            Self::Query(error) => write!(formatter, "join filter error: {error}"),
            Self::Invalid(message) => write!(formatter, "invalid join: {message}"),
        }
    }
}

impl Error for JoinError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidJson(error) => Some(error),
            Self::Catalog(error) => Some(error),
            Self::Query(error) => Some(error),
            Self::Invalid(_) => None,
        }
    }
}

impl From<CatalogError> for JoinError {
    fn from(error: CatalogError) -> Self {
        Self::Catalog(error)
    }
}

impl From<QueryError> for JoinError {
    fn from(error: QueryError) -> Self {
        Self::Query(error)
    }
}

pub fn parse(body: &[u8]) -> Result<JoinRequest, JoinError> {
    let request = serde_json::from_slice::<JoinRequest>(body).map_err(JoinError::InvalidJson)?;
    validate(&request)?;
    Ok(request)
}

pub fn execute(catalog: &Catalog, request: &JoinRequest) -> Result<Vec<Value>, JoinError> {
    validate(request)?;
    let (local_fields, foreign_fields) = join_fields(request)?;
    let left = catalog.open_table(&request.from)?;
    let right = catalog.open_table(&request.join.table)?;
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
                emit(&mut output, request, left_record, Some(right_record))?;
            }
        }
        return Ok(output);
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
                        emit(&mut output, request, left_record, Some(right_record))?;
                    }
                }
            }
            JoinType::Left => {
                if let Some(matches) = matches {
                    for right_record in matches {
                        emit(&mut output, request, left_record, Some(right_record))?;
                    }
                }
                if !had_matches {
                    emit(&mut output, request, left_record, None)?;
                }
            }
            JoinType::Semi if had_matches => emit(&mut output, request, left_record, None)?,
            JoinType::Anti if !had_matches => emit(&mut output, request, left_record, None)?,
            JoinType::Semi | JoinType::Anti => {}
            JoinType::Cross => unreachable!("cross joins return before key lookup"),
        }
    }
    Ok(output)
}

fn validate(request: &JoinRequest) -> Result<(), JoinError> {
    if request.from.is_empty() || request.join.table.is_empty() {
        return Err(JoinError::Invalid(
            "from and join.table must not be empty".into(),
        ));
    }
    if request.from == request.join.table {
        return Err(JoinError::Invalid(
            "self joins require an alias and are not supported".into(),
        ));
    }
    join_fields(request)?;
    super::validation::validate_filter(&request.filter, "filter")?;
    validate_projection(&request.projection)
}

fn join_fields(request: &JoinRequest) -> Result<(Vec<String>, Vec<String>), JoinError> {
    if let JoinType::Cross = &request.join.kind {
        if !request.join.on.is_empty() {
            return Err(JoinError::Invalid(
                "cross joins do not accept join.on conditions".into(),
            ));
        }
        return Ok((Vec::new(), Vec::new()));
    }
    if request.join.on.is_empty() {
        return Err(JoinError::Invalid(
            "join.on requires at least one equality condition".into(),
        ));
    }
    let mut local_fields = Vec::with_capacity(request.join.on.len());
    let mut foreign_fields = Vec::with_capacity(request.join.on.len());
    for (local, condition) in &request.join.on {
        local_fields.push(qualified_field(local, &request.from, "join.on field")?);
        foreign_fields.push(qualified_field(
            &condition.equality.field,
            &request.join.table,
            "join.on.$eq.$field",
        )?);
    }
    Ok((local_fields, foreign_fields))
}

fn qualified_field(value: &str, table: &str, path: &str) -> Result<String, JoinError> {
    let prefix = format!("{table}.");
    let Some(field) = value.strip_prefix(&prefix) else {
        return Err(JoinError::Invalid(format!(
            "{path} must reference {table}.<field>"
        )));
    };
    if field.is_empty() {
        return Err(JoinError::Invalid(format!("{path} has an empty field")));
    }
    Ok(field.to_owned())
}

fn validate_projection(projection: &BTreeMap<String, i8>) -> Result<(), JoinError> {
    let has_inclusion = projection.values().any(|value| *value == 1);
    let has_exclusion = projection.values().any(|value| *value == 0);
    if projection.values().any(|value| !matches!(value, 0 | 1)) {
        return Err(JoinError::Invalid(
            "projection values must be 0 or 1".into(),
        ));
    }
    if has_inclusion && has_exclusion {
        return Err(JoinError::Invalid(
            "projection cannot mix inclusion and exclusion".into(),
        ));
    }
    Ok(())
}

fn encoded_key(
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

fn emit(
    output: &mut Vec<Value>,
    request: &JoinRequest,
    left: &DbfRecord,
    right: Option<&DbfRecord>,
) -> Result<(), JoinError> {
    let mut values = Map::new();
    add_qualified_values(&mut values, &request.from, &left.values);
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
