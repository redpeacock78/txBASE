use super::group::parse_accumulators;
use super::{
    AccumulatorKind, AccumulatorSpec, BucketAutoSpec, GroupSpec, MAX_BUCKETS, QueryError,
    field_reference,
};
use serde_json::Value;

pub(super) fn parse_bucket_auto(value: &Value, index: usize) -> Result<BucketAutoSpec, QueryError> {
    let object = value.as_object().ok_or_else(|| {
        QueryError::Invalid(format!(
            "aggregate stage {index}.$bucketAuto must be an object"
        ))
    })?;
    for option in object.keys() {
        if !matches!(option.as_str(), "groupBy" | "buckets" | "output") {
            return Err(QueryError::Invalid(format!(
                "aggregate stage {index}.$bucketAuto has an unsupported option {option}"
            )));
        }
    }

    let group_by = object
        .get("groupBy")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            QueryError::Invalid(format!(
                "aggregate stage {index}.$bucketAuto.groupBy must be a field reference"
            ))
        })?;
    let group_by = field_reference(
        group_by,
        &format!("aggregate stage {index}.$bucketAuto.groupBy"),
    )?;

    let buckets = object
        .get("buckets")
        .and_then(Value::as_u64)
        .and_then(|buckets| usize::try_from(buckets).ok())
        .filter(|buckets| (1..=MAX_BUCKETS).contains(buckets))
        .ok_or_else(|| {
            QueryError::Invalid(format!(
                "aggregate stage {index}.$bucketAuto.buckets must be a positive integer no greater than {MAX_BUCKETS}"
            ))
        })?;

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
                    "aggregate stage {index}.$bucketAuto.output must be an object"
                ))
            })?;
            if output.is_empty() {
                return Err(QueryError::Invalid(format!(
                    "aggregate stage {index}.$bucketAuto.output cannot be empty"
                )));
            }
            GroupSpec {
                key_field: None,
                key_expression: None,
                accumulators: parse_accumulators(
                    output,
                    &format!("aggregate stage {index}.$bucketAuto.output"),
                )?,
            }
        }
    };

    Ok(BucketAutoSpec {
        group_by,
        buckets,
        output,
    })
}
