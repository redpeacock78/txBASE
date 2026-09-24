use super::super::{QueryError, aggregation_plan};
use super::output;
use crate::dbf::DbfRecord;
use crate::query::expression::ScalarExpression;
use indexmap::IndexMap;

pub(super) fn execute<'a>(
    records: impl Iterator<Item = &'a DbfRecord>,
    expression: &ScalarExpression,
    plan: &aggregation_plan::AggregationPlan,
) -> Result<Vec<serde_json::Value>, QueryError> {
    let spec = aggregation_plan::GroupSpec {
        key_expression: Some(expression.clone()),
        accumulators: vec![aggregation_plan::AccumulatorSpec {
            name: String::from("count"),
            kind: aggregation_plan::AccumulatorKind::Count,
        }],
    };
    let mut sort = IndexMap::new();
    sort.insert(String::from("count"), -1);
    output::execute_group(records, &spec, plan, Some(&sort), "$sortByCount")
}
