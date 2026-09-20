use super::{QueryError, QueryRequest, aggregation, pagination};
use serde_json::{Map, Value};

pub(super) fn validate(request: &QueryRequest) -> Result<(), QueryError> {
    aggregation::validate(request)?;
    pagination::validate(request)?;
    if request.collation.is_some() && request.sort.is_empty() {
        return Err(QueryError::Invalid(
            "collation requires a non-empty sort".into(),
        ));
    }
    for (field, direction) in &request.sort {
        if !matches!(direction, -1 | 1) {
            return Err(QueryError::Invalid(format!(
                "sort direction for {field} must be 1 or -1"
            )));
        }
    }
    for (field, inclusion) in &request.projection {
        if !matches!(inclusion, 0 | 1) {
            return Err(QueryError::Invalid(format!(
                "projection value for {field} must be 0 or 1"
            )));
        }
    }
    let has_inclusion = request.projection.values().any(|value| *value == 1);
    let has_exclusion = request.projection.values().any(|value| *value == 0);
    if has_inclusion && has_exclusion {
        return Err(QueryError::Invalid(
            "projection cannot mix inclusion and exclusion".into(),
        ));
    }
    validate_filter(&request.filter, "filter")
}

pub(crate) fn validate_filter(filter: &Map<String, Value>, path: &str) -> Result<(), QueryError> {
    for (field, condition) in filter {
        match field.as_str() {
            "$and" | "$or" => {
                let clauses = condition.as_array().ok_or_else(|| {
                    QueryError::Invalid(format!("{path}.{field} must be an array"))
                })?;
                for (index, clause) in clauses.iter().enumerate() {
                    let clause = clause.as_object().ok_or_else(|| {
                        QueryError::Invalid(format!("{path}.{field}[{index}] must be an object"))
                    })?;
                    validate_filter(clause, &format!("{path}.{field}[{index}]"))?;
                }
            }
            "$not" => {
                let clause = condition
                    .as_object()
                    .ok_or_else(|| QueryError::Invalid(format!("{path}.$not must be an object")))?;
                validate_filter(clause, &format!("{path}.$not"))?;
            }
            "$expr" => super::expression::validate(condition, &format!("{path}.$expr"))?,
            field if field.starts_with('$') => {
                return Err(QueryError::Invalid(format!(
                    "unsupported logical operator {field}"
                )));
            }
            _ => validate_condition(condition, &format!("{path}.{field}"))?,
        }
    }
    Ok(())
}

fn validate_condition(condition: &Value, path: &str) -> Result<(), QueryError> {
    let Some(operators) = condition.as_object() else {
        return Ok(());
    };
    let has_operator = operators.keys().any(|key| key.starts_with('$'));
    if !has_operator {
        return Ok(());
    }
    if operators.keys().any(|key| !key.starts_with('$')) {
        return Err(QueryError::Invalid(format!(
            "{path} cannot mix operators and fields"
        )));
    }
    for (operator, operand) in operators {
        match operator.as_str() {
            "$eq" | "$ne" | "$gt" | "$gte" | "$lt" | "$lte" => {}
            "$in" | "$nin" => {
                if !operand.is_array() {
                    return Err(QueryError::Invalid(format!(
                        "{path}.{operator} must be an array"
                    )));
                }
            }
            "$not" => {
                if !operand.is_object() {
                    return Err(QueryError::Invalid(format!(
                        "{path}.$not must be an object"
                    )));
                }
                validate_condition(operand, &format!("{path}.$not"))?;
            }
            _ => {
                return Err(QueryError::Invalid(format!(
                    "unsupported operator {operator} at {path}"
                )));
            }
        }
    }
    Ok(())
}
