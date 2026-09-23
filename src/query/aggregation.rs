use super::{QueryError, aggregation_plan};
use crate::dbf::DbfRecord;
use serde_json::{Map, Value};

mod accumulators;
mod input;
mod output;

pub(super) const MAX_GROUPS: usize = 10_000;
// ponytail: bound expanded input rows at the existing query scale; add streaming or spill-to-disk only if larger reports become required.
pub(super) const MAX_UNWOUND_RECORDS: usize = 10_000;
// ponytail: bound distinct materialization at the existing query scale; add spill-to-disk only if larger reports become required.
pub(super) const MAX_DISTINCT_VALUES: usize = 10_000;
// ponytail: bound group-array materialization globally; add spill-to-disk only if larger reports become required.
pub(super) const MAX_COLLECTED_VALUES: usize = 10_000;
pub(super) use super::aggregation_plan::validate;

pub(super) fn execute(
    records: &[&DbfRecord],
    stages: &[Map<String, Value>],
) -> Result<Vec<Value>, QueryError> {
    let plan = aggregation_plan::parse(stages)?;
    let mut records = input::InputRecords::Borrowed(records.to_vec());
    let mut unwound_records = 0;
    for stage in &plan.input {
        records = input::apply_input_stage(records, stage, &mut unwound_records)?;
    }
    match records {
        input::InputRecords::Borrowed(records) => output::execute_materialized(records, &plan),
        input::InputRecords::Owned(records) => output::execute_materialized(records.iter(), &plan),
    }
}
