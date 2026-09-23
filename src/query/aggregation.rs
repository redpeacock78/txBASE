use super::{QueryError, aggregation_plan, matches_filter};
use crate::dbf::DbfRecord;
use crate::query_path::field_value;
use indexmap::IndexMap;
use serde_json::{Map, Value};
use std::cmp::Ordering;
use std::collections::{BTreeMap, btree_map::Entry};

mod accumulators;

pub(super) const MAX_GROUPS: usize = 10_000;
// ponytail: bound expanded input rows at the existing query scale; add streaming or spill-to-disk only if larger reports become required.
pub(super) const MAX_UNWOUND_RECORDS: usize = 10_000;
// ponytail: bound distinct materialization at the existing query scale; add spill-to-disk only if larger reports become required.
pub(super) const MAX_DISTINCT_VALUES: usize = 10_000;
// ponytail: bound group-array materialization globally; add spill-to-disk only if larger reports become required.
pub(super) const MAX_COLLECTED_VALUES: usize = 10_000;
pub(super) use super::aggregation_plan::validate;

enum InputRecords<'a> {
    Borrowed(Vec<&'a DbfRecord>),
    Owned(Vec<DbfRecord>),
}

pub(super) fn execute(
    records: &[&DbfRecord],
    stages: &[Map<String, Value>],
) -> Result<Vec<Value>, QueryError> {
    let plan = aggregation_plan::parse(stages)?;
    let mut records = InputRecords::Borrowed(records.to_vec());
    let mut unwound_records = 0;
    for stage in &plan.input {
        records = apply_input_stage(records, stage, &mut unwound_records)?;
    }
    match records {
        InputRecords::Borrowed(records) => execute_materialized(records, &plan),
        InputRecords::Owned(records) => execute_materialized(records.iter(), &plan),
    }
}

fn apply_input_stage<'a>(
    records: InputRecords<'a>,
    stage: &aggregation_plan::InputStage,
    unwound_records: &mut usize,
) -> Result<InputRecords<'a>, QueryError> {
    match stage {
        aggregation_plan::InputStage::Match(filter) => match records {
            InputRecords::Borrowed(records) => {
                Ok(InputRecords::Borrowed(filter_borrowed(records, filter)?))
            }
            InputRecords::Owned(records) => Ok(InputRecords::Owned(filter_owned(records, filter)?)),
        },
        aggregation_plan::InputStage::Unwind(field) => {
            let records = match records {
                InputRecords::Borrowed(records) => records.into_iter().cloned().collect(),
                InputRecords::Owned(records) => records,
            };
            Ok(InputRecords::Owned(unwind_records(
                records,
                field,
                unwound_records,
            )?))
        }
        aggregation_plan::InputStage::Project(projection) => {
            let records = match records {
                InputRecords::Borrowed(records) => records
                    .into_iter()
                    .map(|record| project_record(record.clone(), projection))
                    .collect(),
                InputRecords::Owned(records) => records
                    .into_iter()
                    .map(|record| project_record(record, projection))
                    .collect(),
            };
            Ok(InputRecords::Owned(records))
        }
        aggregation_plan::InputStage::Sort(sort) => match records {
            InputRecords::Borrowed(mut records) => {
                records.sort_by(|left, right| {
                    super::ordering::compare_records_with_collation(left, right, sort, None)
                });
                Ok(InputRecords::Borrowed(records))
            }
            InputRecords::Owned(mut records) => {
                records.sort_by(|left, right| {
                    super::ordering::compare_records_with_collation(left, right, sort, None)
                });
                Ok(InputRecords::Owned(records))
            }
        },
        aggregation_plan::InputStage::Skip(skip) => match records {
            InputRecords::Borrowed(mut records) => {
                skip_records(&mut records, *skip);
                Ok(InputRecords::Borrowed(records))
            }
            InputRecords::Owned(mut records) => {
                skip_records(&mut records, *skip);
                Ok(InputRecords::Owned(records))
            }
        },
        aggregation_plan::InputStage::Limit(limit) => match records {
            InputRecords::Borrowed(mut records) => {
                records.truncate((*limit).try_into().unwrap_or(usize::MAX));
                Ok(InputRecords::Borrowed(records))
            }
            InputRecords::Owned(mut records) => {
                records.truncate((*limit).try_into().unwrap_or(usize::MAX));
                Ok(InputRecords::Owned(records))
            }
        },
    }
}

fn filter_borrowed<'a>(
    records: Vec<&'a DbfRecord>,
    filter: &Map<String, Value>,
) -> Result<Vec<&'a DbfRecord>, QueryError> {
    let mut filtered = Vec::with_capacity(records.len());
    for record in records {
        if matches_filter(&record.values, filter)? {
            filtered.push(record);
        }
    }
    Ok(filtered)
}

fn filter_owned(
    records: Vec<DbfRecord>,
    filter: &Map<String, Value>,
) -> Result<Vec<DbfRecord>, QueryError> {
    let mut filtered = Vec::with_capacity(records.len());
    for record in records {
        if matches_filter(&record.values, filter)? {
            filtered.push(record);
        }
    }
    Ok(filtered)
}

fn project_record(mut record: DbfRecord, projection: &BTreeMap<String, i8>) -> DbfRecord {
    record.values = match crate::query_path::project_values(&record.values, projection) {
        Value::Object(values) => values,
        Value::Array(_) | Value::String(_) | Value::Number(_) | Value::Bool(_) | Value::Null => {
            unreachable!("record projection always returns an object")
        }
    };
    record
}

fn skip_records<T>(records: &mut Vec<T>, skip: u64) {
    let skip = skip.try_into().unwrap_or(usize::MAX).min(records.len());
    records.drain(..skip);
}

fn execute_materialized<'a>(
    records: impl IntoIterator<Item = &'a DbfRecord>,
    plan: &aggregation_plan::AggregationPlan,
) -> Result<Vec<Value>, QueryError> {
    let records = records.into_iter();
    if let Some(field) = &plan.count {
        let count = u64::try_from(records.count())
            .map_err(|_| QueryError::Invalid("aggregate count does not fit u64".into()))?;
        let mut output = Map::new();
        output.insert(field.clone(), Value::Number(count.into()));
        return Ok(vec![Value::Object(output)]);
    }

    if let Some(field) = &plan.distinct {
        let mut values = BTreeMap::new();
        let mut distinct_count = 0;
        for record in records {
            let value = field_value(&record.values, field).unwrap_or(Value::Null);
            let key = serde_json::to_string(&value).map_err(|error| {
                QueryError::Invalid(format!("distinct value encoding failed: {error}"))
            })?;
            if let Entry::Vacant(entry) = values.entry(key) {
                if distinct_count >= MAX_DISTINCT_VALUES {
                    return Err(QueryError::Invalid(format!(
                        "aggregate distinct value count exceeds {MAX_DISTINCT_VALUES}"
                    )));
                }
                entry.insert(value);
                distinct_count += 1;
            }
        }
        return Ok(values.into_values().collect());
    }

    let spec = plan
        .group
        .as_ref()
        .expect("validated aggregation has a group or count stage");
    let mut groups = BTreeMap::<String, accumulators::GroupState>::new();
    let mut collected_values = 0;
    if spec.key_field.is_none() {
        groups.insert(
            String::from("null"),
            accumulators::new_group(Value::Null, spec),
        );
    }

    for record in records {
        let key = spec
            .key_field
            .as_deref()
            .map(|field| field_value(&record.values, field).unwrap_or(Value::Null))
            .unwrap_or(Value::Null);
        let encoded_key = serde_json::to_string(&key)
            .map_err(|error| QueryError::Invalid(format!("group key encoding failed: {error}")))?;
        if !groups.contains_key(&encoded_key) {
            if groups.len() >= MAX_GROUPS {
                return Err(QueryError::Invalid(format!(
                    "aggregate group count exceeds {MAX_GROUPS}"
                )));
            }
            groups.insert(encoded_key.clone(), accumulators::new_group(key, spec));
        }
        let group = groups
            .get_mut(&encoded_key)
            .expect("group was inserted or already present");
        accumulators::accumulate_record(group, record, spec, &mut collected_values)?;
    }

    let mut output = groups
        .into_values()
        .map(|group| accumulators::finish_group(group, spec))
        .collect::<Result<Vec<_>, _>>()?;
    for filter in &plan.group_matches {
        let mut filtered = Vec::with_capacity(output.len());
        for value in output {
            let values = value.as_object().expect("group output is always an object");
            if matches_filter(values, filter)? {
                filtered.push(value);
            }
        }
        output = filtered;
    }
    if let Some(projection) = &plan.projection {
        output = output
            .into_iter()
            .map(|value| match value {
                Value::Object(values) => crate::query_path::project_values(&values, projection),
                value => value,
            })
            .collect();
    }
    if let Some(sort) = &plan.sort {
        output.sort_by(|left, right| compare_output_values(left, right, sort));
    }
    if let Some(skip) = plan.skip {
        let skip = skip.try_into().unwrap_or(usize::MAX).min(output.len());
        output.drain(..skip);
    }
    if let Some(limit) = plan.limit {
        output.truncate(limit.try_into().unwrap_or(usize::MAX));
    }
    Ok(output)
}

fn unwind_records(
    records: Vec<DbfRecord>,
    field: &str,
    unwound_records: &mut usize,
) -> Result<Vec<DbfRecord>, QueryError> {
    let mut expanded = Vec::new();
    for record in records {
        let Some(value) = record.values.get(field).cloned() else {
            continue;
        };
        match value {
            Value::Array(values) => {
                for value in values {
                    if *unwound_records >= MAX_UNWOUND_RECORDS {
                        return Err(QueryError::Invalid(format!(
                            "aggregate unwound record count exceeds {MAX_UNWOUND_RECORDS}"
                        )));
                    }
                    let mut record = record.clone();
                    record.values.insert(field.to_owned(), value);
                    expanded.push(record);
                    *unwound_records += 1;
                }
            }
            Value::Null => {}
            _ => {
                return Err(QueryError::Invalid(format!(
                    "aggregate $unwind field {field} must be an array"
                )));
            }
        }
    }
    Ok(expanded)
}

fn compare_output_values(left: &Value, right: &Value, sort: &IndexMap<String, i8>) -> Ordering {
    let left = left.as_object();
    let right = right.as_object();
    for (field, direction) in sort {
        let ordering = super::ordering::compare_for_sort(
            left.and_then(|values| values.get(field)),
            right.and_then(|values| values.get(field)),
        );
        if ordering != Ordering::Equal {
            return if *direction == 1 {
                ordering
            } else {
                ordering.reverse()
            };
        }
    }
    Ordering::Equal
}
