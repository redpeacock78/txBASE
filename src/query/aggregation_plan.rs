use super::{QueryError, QueryRequest};

mod group;
mod parse;
mod set;
mod stage_parsers;
mod types;

pub(super) use parse::parse;
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
