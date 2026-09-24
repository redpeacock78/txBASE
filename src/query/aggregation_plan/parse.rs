use super::super::QueryError;
use super::super::expression::ScalarExpression;
use super::types::{AggregationPlan, InputStage};
use serde_json::{Map, Value};

mod input;
mod output;

pub(crate) fn parse(stages: &[Map<String, Value>]) -> Result<AggregationPlan, QueryError> {
    if stages.is_empty() {
        return Err(QueryError::Invalid(
            "aggregate requires at least one stage".into(),
        ));
    }

    let mut parser = Parser::default();
    for (index, stage) in stages.iter().enumerate() {
        if stage.len() != 1 {
            return Err(QueryError::Invalid(format!(
                "aggregate stage {index} must contain one operator"
            )));
        }
        let (operator, value) = stage.iter().next().expect("one aggregate operator");
        if parser.parse_input_stage(index, operator, value)?
            || parser.parse_output_stage(index, operator, value)?
        {
            continue;
        }
        return Err(parser.invalid_stage(index, operator));
    }
    parser.finish()
}

#[derive(Default)]
struct Parser {
    input: Vec<InputStage>,
    input_sort_seen: bool,
    input_skip_seen: bool,
    input_limit_seen: bool,
    input_projection_seen: bool,
    input_set_seen: bool,
    group_matches: Vec<Map<String, Value>>,
    group: Option<super::types::GroupSpec>,
    bucket: Option<super::types::BucketSpec>,
    bucket_auto: Option<super::types::BucketAutoSpec>,
    sort_by_count: Option<ScalarExpression>,
    count: Option<String>,
    distinct: Option<String>,
    projection: Option<std::collections::BTreeMap<String, i8>>,
    sort: Option<indexmap::IndexMap<String, i8>>,
    skip: Option<u64>,
    limit: Option<u64>,
}

impl Parser {
    fn has_group_stage(&self) -> bool {
        self.group.is_some()
            || self.bucket.is_some()
            || self.bucket_auto.is_some()
            || self.sort_by_count.is_some()
    }

    fn has_terminal_stage(&self) -> bool {
        self.has_group_stage() || self.count.is_some() || self.distinct.is_some()
    }

    fn finish(self) -> Result<AggregationPlan, QueryError> {
        if !self.has_terminal_stage() {
            return Err(QueryError::Invalid(
                "aggregate requires a $group, $bucket, $bucketAuto, $sortByCount, $count, or $distinct stage"
                    .into(),
            ));
        }
        Ok(AggregationPlan {
            input: self.input,
            group_matches: self.group_matches,
            group: self.group,
            bucket: self.bucket,
            bucket_auto: self.bucket_auto,
            sort_by_count: self.sort_by_count,
            count: self.count,
            distinct: self.distinct,
            projection: self.projection,
            sort: self.sort,
            skip: self.skip,
            limit: self.limit,
        })
    }

    fn invalid_stage(&self, index: usize, operator: &str) -> QueryError {
        match operator {
            "$match" => QueryError::Invalid(format!(
                "aggregate stage {index}.$match must precede $group, $bucket, $bucketAuto, or $sortByCount, or follow them before $project, $sort, $skip, or $limit"
            )),
            "$group" | "$bucket" | "$bucketAuto" | "$sortByCount" => QueryError::Invalid(
                "aggregate supports only one $group, $bucket, $bucketAuto, or $sortByCount stage"
                    .into(),
            ),
            "$project" => QueryError::Invalid(format!(
                "aggregate stage {index}.$project must be an input or group-output stage and appear once in its phase"
            )),
            "$sort" => QueryError::Invalid(format!(
                "aggregate stage {index}.$sort must be an input stage or follow $group, $bucket, $bucketAuto, or $sortByCount, and appear once in its phase"
            )),
            "$skip" => QueryError::Invalid(format!(
                "aggregate stage {index}.$skip must be an input stage or follow $group, $bucket, $bucketAuto, or $sortByCount, and appear once in its phase"
            )),
            "$limit" => QueryError::Invalid(format!(
                "aggregate stage {index}.$limit must be an input stage or follow $group, $bucket, $bucketAuto, or $sortByCount, and appear once in its phase"
            )),
            "$distinct" => {
                QueryError::Invalid("aggregate supports only one terminal $distinct stage".into())
            }
            "$unwind" => QueryError::Invalid(format!(
                "aggregate stage {index}.$unwind must precede $group, $bucket, $bucketAuto, $sortByCount, $count, or $distinct"
            )),
            "$set" | "$addFields" => QueryError::Invalid(format!(
                "aggregate stage {index}.{operator} must be an input stage and appear once"
            )),
            _ => QueryError::Invalid(format!("unsupported aggregate stage {operator}")),
        }
    }

    fn can_append_group_match_or_projection(&self) -> bool {
        self.has_group_stage()
            && self.projection.is_none()
            && self.sort.is_none()
            && self.skip.is_none()
            && self.limit.is_none()
    }

    fn can_append_group_sort(&self) -> bool {
        self.has_group_stage() && self.sort.is_none() && self.skip.is_none() && self.limit.is_none()
    }

    fn can_append_group_skip(&self) -> bool {
        self.has_group_stage() && self.skip.is_none() && self.limit.is_none()
    }

    fn can_append_group_limit(&self) -> bool {
        self.has_group_stage() && self.limit.is_none()
    }

    fn can_append_input(&self) -> bool {
        !self.has_terminal_stage()
    }
}
