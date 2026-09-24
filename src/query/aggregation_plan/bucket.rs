use super::group::parse_accumulators;
use super::{AccumulatorKind, AccumulatorSpec, BucketSpec, GroupSpec, QueryError, field_reference};
use crate::query::aggregation_plan::MAX_BUCKETS;
use serde_json::Value;

pub(super) fn parse_bucket(value: &Value, index: usize) -> Result<BucketSpec, QueryError> {
    let object = value.as_object().ok_or_else(|| {
        QueryError::Invalid(format!("aggregate stage {index}.$bucket must be an object"))
    })?;
    for option in object.keys() {
        if !matches!(
            option.as_str(),
            "groupBy" | "boundaries" | "default" | "output"
        ) {
            return Err(QueryError::Invalid(format!(
                "aggregate stage {index}.$bucket has an unsupported option {option}"
            )));
        }
    }

    let group_by = object
        .get("groupBy")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            QueryError::Invalid(format!(
                "aggregate stage {index}.$bucket.groupBy must be a field reference"
            ))
        })?;
    let group_by = field_reference(
        group_by,
        &format!("aggregate stage {index}.$bucket.groupBy"),
    )?;

    let boundary_values = object
        .get("boundaries")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            QueryError::Invalid(format!(
                "aggregate stage {index}.$bucket.boundaries must be an array"
            ))
        })?;
    if boundary_values.len() < 2 {
        return Err(QueryError::Invalid(format!(
            "aggregate stage {index}.$bucket.boundaries requires at least two values"
        )));
    }
    if boundary_values.len() > MAX_BUCKETS + 1 {
        return Err(QueryError::Invalid(format!(
            "aggregate stage {index}.$bucket supports at most {MAX_BUCKETS} ranges"
        )));
    }

    let mut previous = None;
    let mut boundaries = Vec::with_capacity(boundary_values.len());
    for (boundary_index, boundary) in boundary_values.iter().enumerate() {
        let number = boundary
            .as_number()
            .and_then(|number| number.as_f64())
            .filter(|number| number.is_finite())
            .ok_or_else(|| {
                QueryError::Invalid(format!(
                    "aggregate stage {index}.$bucket.boundaries[{boundary_index}] must be a finite JSON number"
                ))
            })?;
        if previous.is_some_and(|previous| number <= previous) {
            return Err(QueryError::Invalid(format!(
                "aggregate stage {index}.$bucket.boundaries must be strictly ascending"
            )));
        }
        previous = Some(number);
        boundaries.push(boundary.clone());
    }

    let output = match object.get("output") {
        None => GroupSpec {
            key_field: None,
            key_expression: None,
            accumulators: vec![AccumulatorSpec {
                name: String::from("count"),
                kind: AccumulatorKind::Count,
            }],
        },
        Some(value) => {
            let output = value.as_object().ok_or_else(|| {
                QueryError::Invalid(format!(
                    "aggregate stage {index}.$bucket.output must be an object"
                ))
            })?;
            if output.is_empty() {
                return Err(QueryError::Invalid(format!(
                    "aggregate stage {index}.$bucket.output cannot be empty"
                )));
            }
            GroupSpec {
                key_field: None,
                key_expression: None,
                accumulators: parse_accumulators(
                    output,
                    &format!("aggregate stage {index}.$bucket.output"),
                )?,
            }
        }
    };

    Ok(BucketSpec {
        group_by,
        boundaries,
        default: object.get("default").cloned(),
        output,
    })
}
