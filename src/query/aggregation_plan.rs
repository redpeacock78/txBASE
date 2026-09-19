use super::{QueryError, QueryRequest, validation};
use indexmap::IndexMap;
use serde_json::{Map, Value};

#[derive(Debug, Clone)]
pub(super) struct GroupSpec {
    pub(super) key_field: Option<String>,
    pub(super) accumulators: Vec<AccumulatorSpec>,
}

#[derive(Debug, Clone)]
pub(super) struct AggregationPlan {
    pub(super) matches: Vec<Map<String, Value>>,
    pub(super) group: GroupSpec,
    pub(super) sort: Option<IndexMap<String, i8>>,
    pub(super) limit: Option<u64>,
}

#[derive(Debug, Clone)]
pub(super) struct AccumulatorSpec {
    pub(super) name: String,
    pub(super) kind: AccumulatorKind,
}

#[derive(Debug, Clone)]
pub(super) enum AccumulatorKind {
    Count,
    Average(String),
    Sum(String),
    Min(String),
    Max(String),
}

pub(super) fn validate(request: &QueryRequest) -> Result<(), QueryError> {
    let Some(stages) = request.aggregate.as_ref() else {
        return Ok(());
    };
    if !request.sort.is_empty()
        || !request.projection.is_empty()
        || request.limit.is_some()
        || request.skip.is_some()
        || request.page_size.is_some()
        || request.cursor.is_some()
    {
        return Err(QueryError::Invalid(
            "aggregate cannot be combined with sort, projection, skip, limit, page_size, or cursor"
                .into(),
        ));
    }
    parse(stages).map(|_| ())
}

pub(super) fn parse(stages: &[Map<String, Value>]) -> Result<AggregationPlan, QueryError> {
    if stages.is_empty() {
        return Err(QueryError::Invalid(
            "aggregate requires at least one stage".into(),
        ));
    }

    let mut matches = Vec::new();
    let mut group = None;
    let mut sort = None;
    let mut limit = None;
    for (index, stage) in stages.iter().enumerate() {
        if stage.len() != 1 {
            return Err(QueryError::Invalid(format!(
                "aggregate stage {index} must contain one operator"
            )));
        }
        let (operator, value) = stage.iter().next().expect("one aggregate operator");
        match operator.as_str() {
            "$match" if group.is_none() => {
                let filter = value.as_object().ok_or_else(|| {
                    QueryError::Invalid(format!("aggregate stage {index}.$match must be an object"))
                })?;
                validation::validate_filter(filter, &format!("aggregate[{index}].$match"))?;
                matches.push(filter.clone());
            }
            "$group" if group.is_none() => group = Some(parse_group(value)?),
            "$sort" if group.is_some() && sort.is_none() && limit.is_none() => {
                sort = Some(parse_sort(value, index)?);
            }
            "$limit" if group.is_some() && limit.is_none() => {
                limit = Some(parse_limit(value, index)?);
            }
            "$match" => {
                return Err(QueryError::Invalid(format!(
                    "aggregate stage {index}.$match must precede $group"
                )));
            }
            "$group" => {
                return Err(QueryError::Invalid(
                    "aggregate supports only one $group stage".into(),
                ));
            }
            "$sort" => {
                return Err(QueryError::Invalid(format!(
                    "aggregate stage {index}.$sort must follow $group, precede $limit, and appear once"
                )));
            }
            "$limit" => {
                return Err(QueryError::Invalid(format!(
                    "aggregate stage {index}.$limit must follow $group and appear once"
                )));
            }
            _ => {
                return Err(QueryError::Invalid(format!(
                    "unsupported aggregate stage {operator}"
                )));
            }
        }
    }

    let group =
        group.ok_or_else(|| QueryError::Invalid("aggregate requires a $group stage".into()))?;
    Ok(AggregationPlan {
        matches,
        group,
        sort,
        limit,
    })
}

fn parse_sort(value: &Value, index: usize) -> Result<IndexMap<String, i8>, QueryError> {
    let object = value.as_object().ok_or_else(|| {
        QueryError::Invalid(format!("aggregate stage {index}.$sort must be an object"))
    })?;
    if object.is_empty() {
        return Err(QueryError::Invalid(format!(
            "aggregate stage {index}.$sort cannot be empty"
        )));
    }
    let mut sort = IndexMap::new();
    for (field, direction) in object {
        if field.is_empty() {
            return Err(QueryError::Invalid(format!(
                "aggregate stage {index}.$sort contains an empty field"
            )));
        }
        let Some(direction) = direction.as_i64() else {
            return Err(QueryError::Invalid(format!(
                "aggregate sort direction for {field} must be 1 or -1"
            )));
        };
        if !matches!(direction, -1 | 1) {
            return Err(QueryError::Invalid(format!(
                "aggregate sort direction for {field} must be 1 or -1"
            )));
        }
        sort.insert(field.clone(), direction as i8);
    }
    Ok(sort)
}

fn parse_limit(value: &Value, index: usize) -> Result<u64, QueryError> {
    let Some(limit) = value.as_u64() else {
        return Err(QueryError::Invalid(format!(
            "aggregate stage {index}.$limit must be a non-negative integer"
        )));
    };
    Ok(limit)
}

fn parse_group(definition: &Value) -> Result<GroupSpec, QueryError> {
    let definition = definition
        .as_object()
        .ok_or_else(|| QueryError::Invalid("$group must be an object".into()))?;
    let key = definition
        .get("_id")
        .ok_or_else(|| QueryError::Invalid("$group requires _id".into()))?;
    let key_field = match key {
        Value::Null => None,
        Value::String(value) => Some(field_reference(value, "$group._id")?),
        _ => {
            return Err(QueryError::Invalid(
                "$group._id must be null or a field reference".into(),
            ));
        }
    };

    let mut accumulators = Vec::new();
    for (name, value) in definition {
        if name == "_id" {
            continue;
        }
        if name.is_empty() || name.starts_with('$') || name.contains('.') {
            return Err(QueryError::Invalid(format!(
                "$group output field {name} is invalid"
            )));
        }
        let operators = value
            .as_object()
            .ok_or_else(|| QueryError::Invalid(format!("$group.{name} must be an object")))?;
        if operators.len() != 1 {
            return Err(QueryError::Invalid(format!(
                "$group.{name} must contain one accumulator"
            )));
        }
        let (operator, operand) = operators.iter().next().expect("one accumulator");
        let kind = match operator.as_str() {
            "$count" if operand.as_object().is_some_and(|object| object.is_empty()) => {
                AccumulatorKind::Count
            }
            "$avg" => AccumulatorKind::Average(field_reference(
                operand.as_str().ok_or_else(|| {
                    QueryError::Invalid(format!("$group.{name}.$avg must be a field reference"))
                })?,
                &format!("$group.{name}.$avg"),
            )?),
            "$sum" => AccumulatorKind::Sum(field_reference(
                operand.as_str().ok_or_else(|| {
                    QueryError::Invalid(format!("$group.{name}.$sum must be a field reference"))
                })?,
                &format!("$group.{name}.$sum"),
            )?),
            "$min" | "$max" => {
                let field = field_reference(
                    operand.as_str().ok_or_else(|| {
                        QueryError::Invalid(format!(
                            "$group.{name}.{operator} must be a field reference"
                        ))
                    })?,
                    &format!("$group.{name}.{operator}"),
                )?;
                if operator == "$min" {
                    AccumulatorKind::Min(field)
                } else {
                    AccumulatorKind::Max(field)
                }
            }
            _ => {
                return Err(QueryError::Invalid(format!(
                    "unsupported aggregate accumulator {operator}"
                )));
            }
        };
        accumulators.push(AccumulatorSpec {
            name: name.clone(),
            kind,
        });
    }
    Ok(GroupSpec {
        key_field,
        accumulators,
    })
}

fn field_reference(value: &str, path: &str) -> Result<String, QueryError> {
    let Some(field) = value.strip_prefix('$') else {
        return Err(QueryError::Invalid(format!(
            "{path} must be a field reference"
        )));
    };
    if field.is_empty() {
        return Err(QueryError::Invalid(format!(
            "{path} has an empty field reference"
        )));
    }
    Ok(field.to_owned())
}
