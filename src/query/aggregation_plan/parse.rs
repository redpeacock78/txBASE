use super::super::{QueryError, validation};
use super::bucket::parse_bucket;
use super::group::parse_group;
use super::set::parse_set;
use super::stage_parsers::{
    parse_count, parse_distinct, parse_limit, parse_projection, parse_skip, parse_sort,
    parse_sort_by_count, parse_unwind,
};
use super::types::{AggregationPlan, InputStage};
use serde_json::{Map, Value};

pub(crate) fn parse(stages: &[Map<String, Value>]) -> Result<AggregationPlan, QueryError> {
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
    let mut bucket = None;
    let mut sort_by_count = None;
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
            "$match"
                if group.is_none()
                    && bucket.is_none()
                    && sort_by_count.is_none()
                    && count.is_none()
                    && distinct.is_none() =>
            {
                let filter = value.as_object().ok_or_else(|| {
                    QueryError::Invalid(format!("aggregate stage {index}.$match must be an object"))
                })?;
                validation::validate_filter(filter, &format!("aggregate[{index}].$match"))?;
                input.push(InputStage::Match(filter.clone()));
            }
            "$unwind"
                if group.is_none()
                    && bucket.is_none()
                    && sort_by_count.is_none()
                    && count.is_none()
                    && distinct.is_none() =>
            {
                input.push(InputStage::Unwind(parse_unwind(value, index)?));
            }
            "$set" | "$addFields"
                if group.is_none()
                    && bucket.is_none()
                    && sort_by_count.is_none()
                    && count.is_none()
                    && distinct.is_none()
                    && !input_set_seen =>
            {
                input.push(InputStage::Set(parse_set(value, index, operator)?));
                input_set_seen = true;
            }
            "$project"
                if group.is_none()
                    && bucket.is_none()
                    && sort_by_count.is_none()
                    && count.is_none()
                    && distinct.is_none()
                    && !input_projection_seen =>
            {
                input.push(InputStage::Project(parse_projection(value, index)?));
                input_projection_seen = true;
            }
            "$sort"
                if group.is_none()
                    && bucket.is_none()
                    && sort_by_count.is_none()
                    && count.is_none()
                    && distinct.is_none()
                    && !input_sort_seen =>
            {
                input.push(InputStage::Sort(parse_sort(value, index)?));
                input_sort_seen = true;
            }
            "$skip"
                if group.is_none()
                    && bucket.is_none()
                    && sort_by_count.is_none()
                    && count.is_none()
                    && distinct.is_none()
                    && !input_skip_seen =>
            {
                input.push(InputStage::Skip(parse_skip(value, index)?));
                input_skip_seen = true;
            }
            "$limit"
                if group.is_none()
                    && bucket.is_none()
                    && sort_by_count.is_none()
                    && count.is_none()
                    && distinct.is_none()
                    && !input_limit_seen =>
            {
                input.push(InputStage::Limit(parse_limit(value, index)?));
                input_limit_seen = true;
            }
            "$match"
                if (group.is_some() || bucket.is_some() || sort_by_count.is_some())
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
            "$group"
                if group.is_none()
                    && bucket.is_none()
                    && sort_by_count.is_none()
                    && count.is_none()
                    && distinct.is_none() =>
            {
                group = Some(parse_group(value)?);
            }
            "$bucket"
                if group.is_none()
                    && bucket.is_none()
                    && sort_by_count.is_none()
                    && count.is_none()
                    && distinct.is_none() =>
            {
                bucket = Some(parse_bucket(value, index)?);
            }
            "$sortByCount"
                if group.is_none()
                    && bucket.is_none()
                    && sort_by_count.is_none()
                    && count.is_none()
                    && distinct.is_none() =>
            {
                sort_by_count = Some(parse_sort_by_count(value, index)?);
            }
            "$count"
                if group.is_none()
                    && bucket.is_none()
                    && sort_by_count.is_none()
                    && count.is_none()
                    && distinct.is_none() =>
            {
                count = Some(parse_count(value, index)?);
            }
            "$distinct"
                if group.is_none()
                    && bucket.is_none()
                    && sort_by_count.is_none()
                    && count.is_none()
                    && distinct.is_none() =>
            {
                distinct = Some(parse_distinct(value, index)?);
            }
            "$project"
                if (group.is_some() || bucket.is_some() || sort_by_count.is_some())
                    && projection.is_none()
                    && sort.is_none()
                    && skip.is_none()
                    && limit.is_none() =>
            {
                projection = Some(parse_projection(value, index)?);
            }
            "$sort"
                if (group.is_some() || bucket.is_some() || sort_by_count.is_some())
                    && sort.is_none()
                    && skip.is_none()
                    && limit.is_none() =>
            {
                sort = Some(parse_sort(value, index)?);
            }
            "$skip"
                if (group.is_some() || bucket.is_some() || sort_by_count.is_some())
                    && skip.is_none()
                    && limit.is_none() =>
            {
                skip = Some(parse_skip(value, index)?);
            }
            "$limit"
                if (group.is_some() || bucket.is_some() || sort_by_count.is_some())
                    && limit.is_none() =>
            {
                limit = Some(parse_limit(value, index)?);
            }
            "$match" => {
                return Err(QueryError::Invalid(format!(
                    "aggregate stage {index}.$match must precede $group, $bucket, or $sortByCount, or follow them before $project, $sort, $skip, or $limit"
                )));
            }
            "$group" => {
                return Err(QueryError::Invalid(
                    "aggregate supports only one $group, $bucket, or $sortByCount stage".into(),
                ));
            }
            "$bucket" => {
                return Err(QueryError::Invalid(
                    "aggregate supports only one $group, $bucket, or $sortByCount stage".into(),
                ));
            }
            "$sortByCount" => {
                return Err(QueryError::Invalid(
                    "aggregate supports only one $group, $bucket, or $sortByCount stage".into(),
                ));
            }
            "$project" => {
                return Err(QueryError::Invalid(format!(
                    "aggregate stage {index}.$project must be an input or group-output stage and appear once in its phase"
                )));
            }
            "$sort" => {
                return Err(QueryError::Invalid(format!(
                    "aggregate stage {index}.$sort must be an input stage or follow $group, $bucket, or $sortByCount, and appear once in its phase"
                )));
            }
            "$skip" => {
                return Err(QueryError::Invalid(format!(
                    "aggregate stage {index}.$skip must be an input stage or follow $group, $bucket, or $sortByCount, and appear once in its phase"
                )));
            }
            "$limit" => {
                return Err(QueryError::Invalid(format!(
                    "aggregate stage {index}.$limit must be an input stage or follow $group, $bucket, or $sortByCount, and appear once in its phase"
                )));
            }
            "$distinct" => {
                return Err(QueryError::Invalid(
                    "aggregate supports only one terminal $distinct stage".into(),
                ));
            }
            "$unwind" => {
                return Err(QueryError::Invalid(format!(
                    "aggregate stage {index}.$unwind must precede $group, $bucket, $sortByCount, $count, or $distinct"
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

    if group.is_none()
        && bucket.is_none()
        && sort_by_count.is_none()
        && count.is_none()
        && distinct.is_none()
    {
        return Err(QueryError::Invalid(
            "aggregate requires a $group, $bucket, $sortByCount, $count, or $distinct stage".into(),
        ));
    }
    Ok(AggregationPlan {
        input,
        group_matches,
        group,
        bucket,
        sort_by_count,
        count,
        distinct,
        projection,
        sort,
        skip,
        limit,
    })
}
