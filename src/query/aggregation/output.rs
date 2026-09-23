use super::super::{QueryError, aggregation_plan, matches_filter};
use super::{MAX_DISTINCT_VALUES, MAX_GROUPS, accumulators};
use crate::dbf::DbfRecord;
use indexmap::IndexMap;
use serde_json::{Map, Value};
use std::cmp::Ordering;
use std::collections::{BTreeMap, btree_map::Entry};

pub(super) fn execute_materialized<'a>(
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
            let value =
                crate::query_path::field_value(&record.values, field).unwrap_or(Value::Null);
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
            .map(|field| {
                crate::query_path::field_value(&record.values, field).unwrap_or(Value::Null)
            })
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

fn compare_output_values(left: &Value, right: &Value, sort: &IndexMap<String, i8>) -> Ordering {
    let left = left.as_object();
    let right = right.as_object();
    for (field, direction) in sort {
        let ordering = super::super::ordering::compare_for_sort(
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
