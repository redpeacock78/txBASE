use super::{QueryError, QueryRequest, validation};
use indexmap::IndexMap;
use serde_json::{Map, Value};
use std::collections::BTreeMap;

mod group;
mod set;
mod types;

use group::parse_group;
use set::parse_set;
pub(super) use types::{
    AccumulatorKind, AccumulatorSpec, AggregationPlan, GroupSpec, InputStage, SetExpression,
    UnwindSpec,
};

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
        || request.collation.is_some()
    {
        return Err(QueryError::Invalid(
            "aggregate cannot be combined with sort, projection, skip, limit, page_size, cursor, or collation"
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

    let mut input = Vec::new();
    let mut input_sort_seen = false;
    let mut input_skip_seen = false;
    let mut input_limit_seen = false;
    let mut input_projection_seen = false;
    let mut input_set_seen = false;
    let mut group_matches = Vec::new();
    let mut group = None;
    let mut count = None;
    let mut distinct = None;
    let mut projection = None;
    let mut sort = None;
    let mut skip = None;
    let mut limit = None;
    for (index, stage) in stages.iter().enumerate() {
        if stage.len() != 1 {
            return Err(QueryError::Invalid(format!(
                "aggregate stage {index} must contain one operator"
            )));
        }
        let (operator, value) = stage.iter().next().expect("one aggregate operator");
        match operator.as_str() {
            "$match" if group.is_none() && count.is_none() && distinct.is_none() => {
                let filter = value.as_object().ok_or_else(|| {
                    QueryError::Invalid(format!("aggregate stage {index}.$match must be an object"))
                })?;
                validation::validate_filter(filter, &format!("aggregate[{index}].$match"))?;
                input.push(InputStage::Match(filter.clone()));
            }
            "$unwind" if group.is_none() && count.is_none() && distinct.is_none() => {
                input.push(InputStage::Unwind(parse_unwind(value, index)?));
            }
            "$set" | "$addFields"
                if group.is_none() && count.is_none() && distinct.is_none() && !input_set_seen =>
            {
                input.push(InputStage::Set(parse_set(value, index, operator)?));
                input_set_seen = true;
            }
            "$project"
                if group.is_none()
                    && count.is_none()
                    && distinct.is_none()
                    && !input_projection_seen =>
            {
                input.push(InputStage::Project(parse_projection(value, index)?));
                input_projection_seen = true;
            }
            "$sort"
                if group.is_none() && count.is_none() && distinct.is_none() && !input_sort_seen =>
            {
                input.push(InputStage::Sort(parse_sort(value, index)?));
                input_sort_seen = true;
            }
            "$skip"
                if group.is_none() && count.is_none() && distinct.is_none() && !input_skip_seen =>
            {
                input.push(InputStage::Skip(parse_skip(value, index)?));
                input_skip_seen = true;
            }
            "$limit"
                if group.is_none()
                    && count.is_none()
                    && distinct.is_none()
                    && !input_limit_seen =>
            {
                input.push(InputStage::Limit(parse_limit(value, index)?));
                input_limit_seen = true;
            }
            "$match"
                if group.is_some()
                    && projection.is_none()
                    && sort.is_none()
                    && skip.is_none()
                    && limit.is_none() =>
            {
                let filter = value.as_object().ok_or_else(|| {
                    QueryError::Invalid(format!("aggregate stage {index}.$match must be an object"))
                })?;
                validation::validate_filter(filter, &format!("aggregate[{index}].$match"))?;
                group_matches.push(filter.clone());
            }
            "$group" if group.is_none() && count.is_none() && distinct.is_none() => {
                group = Some(parse_group(value)?);
            }
            "$count" if group.is_none() && count.is_none() && distinct.is_none() => {
                count = Some(parse_count(value, index)?);
            }
            "$distinct" if group.is_none() && count.is_none() && distinct.is_none() => {
                distinct = Some(parse_distinct(value, index)?);
            }
            "$project"
                if group.is_some()
                    && projection.is_none()
                    && sort.is_none()
                    && skip.is_none()
                    && limit.is_none() =>
            {
                projection = Some(parse_projection(value, index)?);
            }
            "$sort" if group.is_some() && sort.is_none() && skip.is_none() && limit.is_none() => {
                sort = Some(parse_sort(value, index)?);
            }
            "$skip" if group.is_some() && skip.is_none() && limit.is_none() => {
                skip = Some(parse_skip(value, index)?);
            }
            "$limit" if group.is_some() && limit.is_none() => {
                limit = Some(parse_limit(value, index)?);
            }
            "$match" => {
                return Err(QueryError::Invalid(format!(
                    "aggregate stage {index}.$match must precede $group or follow $group before $project, $sort, $skip, or $limit"
                )));
            }
            "$group" => {
                return Err(QueryError::Invalid(
                    "aggregate supports only one $group stage".into(),
                ));
            }
            "$project" => {
                return Err(QueryError::Invalid(format!(
                    "aggregate stage {index}.$project must be an input or group-output stage and appear once in its phase"
                )));
            }
            "$sort" => {
                return Err(QueryError::Invalid(format!(
                    "aggregate stage {index}.$sort must be an input stage or follow $group, and appear once in its phase"
                )));
            }
            "$skip" => {
                return Err(QueryError::Invalid(format!(
                    "aggregate stage {index}.$skip must be an input stage or follow $group, and appear once in its phase"
                )));
            }
            "$limit" => {
                return Err(QueryError::Invalid(format!(
                    "aggregate stage {index}.$limit must be an input stage or follow $group, and appear once in its phase"
                )));
            }
            "$distinct" => {
                return Err(QueryError::Invalid(
                    "aggregate supports only one terminal $distinct stage".into(),
                ));
            }
            "$unwind" => {
                return Err(QueryError::Invalid(format!(
                    "aggregate stage {index}.$unwind must precede $group, $count, or $distinct"
                )));
            }
            "$set" | "$addFields" => {
                return Err(QueryError::Invalid(format!(
                    "aggregate stage {index}.{operator} must be an input stage and appear once"
                )));
            }
            _ => {
                return Err(QueryError::Invalid(format!(
                    "unsupported aggregate stage {operator}"
                )));
            }
        }
    }

    if group.is_none() && count.is_none() && distinct.is_none() {
        return Err(QueryError::Invalid(
            "aggregate requires a $group, $count, or $distinct stage".into(),
        ));
    }
    Ok(AggregationPlan {
        input,
        group_matches,
        group,
        count,
        distinct,
        projection,
        sort,
        skip,
        limit,
    })
}

fn parse_unwind(value: &Value, index: usize) -> Result<UnwindSpec, QueryError> {
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

fn parse_count(value: &Value, index: usize) -> Result<String, QueryError> {
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

fn parse_distinct(value: &Value, index: usize) -> Result<String, QueryError> {
    let Some(field) = value.as_str() else {
        return Err(QueryError::Invalid(format!(
            "aggregate stage {index}.$distinct must be a field reference"
        )));
    };
    field_reference(field, &format!("aggregate stage {index}.$distinct"))
}

fn parse_projection(value: &Value, index: usize) -> Result<BTreeMap<String, i8>, QueryError> {
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

fn parse_skip(value: &Value, index: usize) -> Result<u64, QueryError> {
    let Some(skip) = value.as_u64() else {
        return Err(QueryError::Invalid(format!(
            "aggregate stage {index}.$skip must be a non-negative integer"
        )));
    };
    Ok(skip)
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
