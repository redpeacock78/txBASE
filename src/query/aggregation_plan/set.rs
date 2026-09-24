use super::SetExpression;
use crate::query::{QueryError, expression};
use serde_json::Value;
use std::collections::BTreeMap;

pub(super) fn parse_set(
    value: &Value,
    index: usize,
    operator: &str,
) -> Result<BTreeMap<String, SetExpression>, QueryError> {
    let object = value.as_object().ok_or_else(|| {
        QueryError::Invalid(format!(
            "aggregate stage {index}.{operator} must be an object"
        ))
    })?;
    if object.is_empty() {
        return Err(QueryError::Invalid(format!(
            "aggregate stage {index}.{operator} cannot be empty"
        )));
    }
    let mut expressions = BTreeMap::new();
    for (field, expression) in object {
        if field.is_empty() || field.starts_with('$') || field.contains('.') {
            return Err(QueryError::Invalid(format!(
                "aggregate stage {index}.{operator} output fields must be top-level names"
            )));
        }
        expressions.insert(
            field.clone(),
            expression::parse_scalar_operand(
                expression,
                &format!("aggregate stage {index}.{operator}.{field}"),
            )?,
        );
    }
    Ok(expressions)
}
