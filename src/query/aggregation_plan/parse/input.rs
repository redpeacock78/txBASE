use super::super::set::parse_set;
use super::super::stage_parsers::{
    parse_limit, parse_projection, parse_skip, parse_sort, parse_unwind,
};
use super::super::types::InputStage;
use super::Parser;
use crate::query::{QueryError, validation};
use serde_json::Value;

impl Parser {
    pub(super) fn parse_input_stage(
        &mut self,
        index: usize,
        operator: &str,
        value: &Value,
    ) -> Result<bool, QueryError> {
        if !self.can_append_input() {
            return Ok(false);
        }
        match operator {
            "$match" => {
                let filter = value.as_object().ok_or_else(|| {
                    QueryError::Invalid(format!("aggregate stage {index}.$match must be an object"))
                })?;
                validation::validate_filter(filter, &format!("aggregate[{index}].$match"))?;
                self.input.push(InputStage::Match(filter.clone()));
                Ok(true)
            }
            "$unwind" => {
                self.input
                    .push(InputStage::Unwind(parse_unwind(value, index)?));
                Ok(true)
            }
            "$set" | "$addFields" if !self.input_set_seen => {
                self.input
                    .push(InputStage::Set(parse_set(value, index, operator)?));
                self.input_set_seen = true;
                Ok(true)
            }
            "$project" if !self.input_projection_seen => {
                self.input
                    .push(InputStage::Project(parse_projection(value, index)?));
                self.input_projection_seen = true;
                Ok(true)
            }
            "$sort" if !self.input_sort_seen => {
                self.input.push(InputStage::Sort(parse_sort(value, index)?));
                self.input_sort_seen = true;
                Ok(true)
            }
            "$skip" if !self.input_skip_seen => {
                self.input.push(InputStage::Skip(parse_skip(value, index)?));
                self.input_skip_seen = true;
                Ok(true)
            }
            "$limit" if !self.input_limit_seen => {
                self.input
                    .push(InputStage::Limit(parse_limit(value, index)?));
                self.input_limit_seen = true;
                Ok(true)
            }
            _ => Ok(false),
        }
    }
}
