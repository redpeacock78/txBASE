use super::super::{QueryError, aggregation_plan, matches_filter};
use super::MAX_UNWOUND_RECORDS;
use crate::dbf::DbfRecord;
use crate::query::expression::evaluate_scalar;
use serde_json::{Map, Value};
use std::collections::BTreeMap;

pub(super) enum InputRecords<'a> {
    Borrowed(Vec<&'a DbfRecord>),
    Owned(Vec<DbfRecord>),
}

pub(super) fn apply_input_stage<'a>(
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
        aggregation_plan::InputStage::Unwind(spec) => {
            let records = match records {
                InputRecords::Borrowed(records) => records.into_iter().cloned().collect(),
                InputRecords::Owned(records) => records,
            };
            Ok(InputRecords::Owned(unwind_records(
                records,
                spec,
                unwound_records,
            )?))
        }
        aggregation_plan::InputStage::Set(expressions) => {
            let records = match records {
                InputRecords::Borrowed(records) => records
                    .into_iter()
                    .map(|record| set_record(record.clone(), expressions))
                    .collect::<Result<Vec<_>, _>>()?,
                InputRecords::Owned(records) => records
                    .into_iter()
                    .map(|record| set_record(record, expressions))
                    .collect::<Result<Vec<_>, _>>()?,
            };
            Ok(InputRecords::Owned(records))
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
                    super::super::ordering::compare_records_with_collation(left, right, sort, None)
                });
                Ok(InputRecords::Borrowed(records))
            }
            InputRecords::Owned(mut records) => {
                records.sort_by(|left, right| {
                    super::super::ordering::compare_records_with_collation(left, right, sort, None)
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

fn set_record(
    mut record: DbfRecord,
    expressions: &BTreeMap<String, aggregation_plan::SetExpression>,
) -> Result<DbfRecord, QueryError> {
    let source = record.values.clone();
    for (field, expression) in expressions {
        let value = evaluate_scalar(&source, expression, &format!("aggregate.$set.{field}"))?
            .unwrap_or(Value::Null);
        record.values.insert(field.clone(), value);
    }
    Ok(record)
}

fn skip_records<T>(records: &mut Vec<T>, skip: u64) {
    let skip = skip.try_into().unwrap_or(usize::MAX).min(records.len());
    records.drain(..skip);
}

fn unwind_records(
    records: Vec<DbfRecord>,
    spec: &aggregation_plan::UnwindSpec,
    unwound_records: &mut usize,
) -> Result<Vec<DbfRecord>, QueryError> {
    let mut expanded = Vec::new();
    for record in records {
        let value = record.values.get(&spec.field).cloned();
        match value {
            Some(Value::Array(values)) => {
                if values.is_empty() {
                    if spec.preserve_null_and_empty {
                        let mut record = record.clone();
                        record.values.remove(&spec.field);
                        if let Some(index_field) = &spec.include_array_index {
                            record.values.insert(index_field.clone(), Value::Null);
                        }
                        push_unwound_record(&mut expanded, record, unwound_records)?;
                    }
                } else {
                    for (index, value) in values.into_iter().enumerate() {
                        let mut record = record.clone();
                        record.values.insert(spec.field.clone(), value);
                        if let Some(index_field) = &spec.include_array_index {
                            record
                                .values
                                .insert(index_field.clone(), Value::Number((index as u64).into()));
                        }
                        push_unwound_record(&mut expanded, record, unwound_records)?;
                    }
                }
            }
            Some(Value::Null) | None => {
                if spec.preserve_null_and_empty {
                    let mut record = record.clone();
                    if let Some(index_field) = &spec.include_array_index {
                        record.values.insert(index_field.clone(), Value::Null);
                    }
                    push_unwound_record(&mut expanded, record, unwound_records)?;
                }
            }
            Some(_) => {
                return Err(QueryError::Invalid(format!(
                    "aggregate $unwind field {} must be an array",
                    spec.field
                )));
            }
        }
    }
    Ok(expanded)
}

fn push_unwound_record(
    expanded: &mut Vec<DbfRecord>,
    record: DbfRecord,
    unwound_records: &mut usize,
) -> Result<(), QueryError> {
    if *unwound_records >= MAX_UNWOUND_RECORDS {
        return Err(QueryError::Invalid(format!(
            "aggregate unwound record count exceeds {MAX_UNWOUND_RECORDS}"
        )));
    }
    expanded.push(record);
    *unwound_records += 1;
    Ok(())
}
