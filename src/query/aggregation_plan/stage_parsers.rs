use super::super::QueryError;
use super::super::expression::{ScalarExpression, parse_scalar_operand};
use super::field_reference;
use super::types::UnwindSpec;
use indexmap::IndexMap;
use serde_json::Value;
use std::collections::BTreeMap;

pub(super) fn parse_unwind(value: &Value, index: usize) -> Result<UnwindSpec, QueryError> {
    let (path, include_array_index, preserve_null_and_empty) = match value {
        Value::String(path) => (path.clone(), None, false),
        Value::Object(object) => {
            for option in object.keys() {
                if !matches!(
                    option.as_str(),
                    "path" | "includeArrayIndex" | "preserveNullAndEmptyArrays"
                ) {
                    return Err(QueryError::Invalid(format!(
                        "aggregate stage {index}.$unwind has an unsupported option {option}"
                    )));
                }
            }
            let path = object
                .get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    QueryError::Invalid(format!(
                        "aggregate stage {index}.$unwind.path must be a field reference"
                    ))
                })?
                .to_owned();
            let include_array_index = match object.get("includeArrayIndex") {
                None => None,
                Some(value) => Some(
                    value
                        .as_str()
                        .ok_or_else(|| {
                            QueryError::Invalid(format!(
                                "aggregate stage {index}.$unwind.includeArrayIndex must be a field name"
                            ))
                        })?
                        .to_owned(),
                ),
            };
            let preserve_null_and_empty = match object.get("preserveNullAndEmptyArrays") {
                None => false,
                Some(value) => value.as_bool().ok_or_else(|| {
                    QueryError::Invalid(format!(
                        "aggregate stage {index}.$unwind.preserveNullAndEmptyArrays must be a boolean"
                    ))
                })?,
            };
            (path, include_array_index, preserve_null_and_empty)
        }
        _ => {
            return Err(QueryError::Invalid(format!(
                "aggregate stage {index}.$unwind must be a field reference or option object"
            )));
        }
    };
    let field = field_reference(&path, &format!("aggregate stage {index}.$unwind.path"))?;
    if field.contains('.') {
        return Err(QueryError::Invalid(format!(
            "aggregate stage {index}.$unwind must reference a top-level field"
        )));
    }
    if let Some(include_array_index) = &include_array_index {
        if include_array_index.is_empty()
            || include_array_index.starts_with('$')
            || include_array_index.contains('.')
        {
            return Err(QueryError::Invalid(format!(
                "aggregate stage {index}.$unwind.includeArrayIndex must be a top-level field name"
            )));
        }
        if include_array_index == &field {
            return Err(QueryError::Invalid(format!(
                "aggregate stage {index}.$unwind.includeArrayIndex must differ from path"
            )));
        }
    }
    Ok(UnwindSpec {
        field,
        include_array_index,
        preserve_null_and_empty,
    })
}

pub(super) fn parse_count(value: &Value, index: usize) -> Result<String, QueryError> {
    let Some(field) = value.as_str() else {
        return Err(QueryError::Invalid(format!(
            "aggregate stage {index}.$count must be a field name"
        )));
    };
    if field.is_empty() || field.starts_with('$') || field.contains('.') {
        return Err(QueryError::Invalid(format!(
            "aggregate stage {index}.$count has an invalid field name"
        )));
    }
    Ok(field.to_owned())
}

pub(super) fn parse_distinct(value: &Value, index: usize) -> Result<String, QueryError> {
    let Some(field) = value.as_str() else {
        return Err(QueryError::Invalid(format!(
            "aggregate stage {index}.$distinct must be a field reference"
        )));
    };
    field_reference(field, &format!("aggregate stage {index}.$distinct"))
}

pub(super) fn parse_sort_by_count(
    value: &Value,
    index: usize,
) -> Result<ScalarExpression, QueryError> {
    parse_scalar_operand(value, &format!("aggregate stage {index}.$sortByCount"))
}

pub(super) fn parse_projection(
    value: &Value,
    index: usize,
) -> Result<BTreeMap<String, i8>, QueryError> {
    let object = value.as_object().ok_or_else(|| {
        QueryError::Invalid(format!(
            "aggregate stage {index}.$project must be an object"
        ))
    })?;
    if object.is_empty() {
        return Err(QueryError::Invalid(format!(
            "aggregate stage {index}.$project cannot be empty"
        )));
    }
    let mut projection = BTreeMap::new();
    for (field, inclusion) in object {
        if field.is_empty() {
            return Err(QueryError::Invalid(format!(
                "aggregate stage {index}.$project contains an empty field"
            )));
        }
        let Some(inclusion) = inclusion.as_i64() else {
            return Err(QueryError::Invalid(format!(
                "aggregate projection value for {field} must be 0 or 1"
            )));
        };
        if !matches!(inclusion, 0 | 1) {
            return Err(QueryError::Invalid(format!(
                "aggregate projection value for {field} must be 0 or 1"
            )));
        }
        projection.insert(field.clone(), inclusion as i8);
    }
    let has_inclusion = projection.values().any(|value| *value == 1);
    let has_exclusion = projection.values().any(|value| *value == 0);
    if has_inclusion && has_exclusion {
        return Err(QueryError::Invalid(
            "aggregate projection cannot mix inclusion and exclusion".into(),
        ));
    }
    Ok(projection)
}

pub(super) fn parse_sort(value: &Value, index: usize) -> Result<IndexMap<String, i8>, QueryError> {
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

pub(super) fn parse_limit(value: &Value, index: usize) -> Result<u64, QueryError> {
    let Some(limit) = value.as_u64() else {
        return Err(QueryError::Invalid(format!(
            "aggregate stage {index}.$limit must be a non-negative integer"
        )));
    };
    Ok(limit)
}

pub(super) fn parse_skip(value: &Value, index: usize) -> Result<u64, QueryError> {
    let Some(skip) = value.as_u64() else {
        return Err(QueryError::Invalid(format!(
            "aggregate stage {index}.$skip must be a non-negative integer"
        )));
    };
    Ok(skip)
}
