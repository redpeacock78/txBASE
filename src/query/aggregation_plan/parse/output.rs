use super::super::bucket::parse_bucket;
use super::super::bucket_auto::parse_bucket_auto;
use super::super::group::parse_group;
use super::super::stage_parsers::{
    parse_count, parse_distinct, parse_limit, parse_projection, parse_skip, parse_sort,
    parse_sort_by_count,
};
use super::Parser;
use crate::query::{QueryError, validation};
use serde_json::Value;

impl Parser {
    pub(super) fn parse_output_stage(
        &mut self,
        index: usize,
        operator: &str,
        value: &Value,
    ) -> Result<bool, QueryError> {
        match operator {
            "$match" if self.can_append_group_match_or_projection() => {
                let filter = value.as_object().ok_or_else(|| {
                    QueryError::Invalid(format!("aggregate stage {index}.$match must be an object"))
                })?;
                validation::validate_filter(filter, &format!("aggregate[{index}].$match"))?;
                self.group_matches.push(filter.clone());
                return Ok(true);
            }
            "$project" if self.can_append_group_match_or_projection() => {
                self.projection = Some(parse_projection(value, index)?);
                return Ok(true);
            }
            "$sort" if self.can_append_group_sort() => {
                self.sort = Some(parse_sort(value, index)?);
                return Ok(true);
            }
            "$skip" if self.can_append_group_skip() => {
                self.skip = Some(parse_skip(value, index)?);
                return Ok(true);
            }
            "$limit" if self.can_append_group_limit() => {
                self.limit = Some(parse_limit(value, index)?);
                return Ok(true);
            }
            _ => {}
        }

        if self.has_terminal_stage() {
            return Ok(false);
        }
        match operator {
            "$group" => {
                self.group = Some(parse_group(value)?);
                Ok(true)
            }
            "$bucket" => {
                self.bucket = Some(parse_bucket(value, index)?);
                Ok(true)
            }
            "$bucketAuto" => {
                self.bucket_auto = Some(parse_bucket_auto(value, index)?);
                Ok(true)
            }
            "$sortByCount" => {
                self.sort_by_count = Some(parse_sort_by_count(value, index)?);
                Ok(true)
            }
            "$count" => {
                self.count = Some(parse_count(value, index)?);
                Ok(true)
            }
            "$distinct" => {
                self.distinct = Some(parse_distinct(value, index)?);
                Ok(true)
            }
            _ => Ok(false),
        }
    }
}
