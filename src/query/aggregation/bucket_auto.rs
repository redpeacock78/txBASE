use super::MAX_BUCKET_AUTO_VALUES;
use super::accumulators;
use super::output::finish_group_output;
use crate::dbf::DbfRecord;
use crate::query::{QueryError, aggregation_plan};
use serde_json::{Map, Value};

struct BucketAutoValue {
    number: f64,
    value: Value,
}

struct BucketAutoRange {
    upper: f64,
    key: Value,
}

pub(super) fn execute<'a>(
    records: impl Iterator<Item = &'a DbfRecord>,
    bucket: &aggregation_plan::BucketAutoSpec,
    plan: &aggregation_plan::AggregationPlan,
) -> Result<Vec<Value>, QueryError> {
    let mut input_records = Vec::new();
    let mut values = Vec::new();
    for record in records {
        let value = crate::query::expression::evaluate_scalar(
            &record.values,
            &bucket.group_by,
            "$bucketAuto.groupBy",
        )?;
        let Some(number) = value
            .as_ref()
            .and_then(Value::as_number)
            .and_then(|number| number.as_f64())
            .filter(|number| number.is_finite())
        else {
            return Err(QueryError::Invalid(
                "aggregate $bucketAuto groupBy expression is missing or non-numeric".to_string(),
            ));
        };
        if values.len() >= MAX_BUCKET_AUTO_VALUES {
            return Err(QueryError::Invalid(format!(
                "aggregate $bucketAuto value count exceeds {MAX_BUCKET_AUTO_VALUES}"
            )));
        }
        input_records.push((record, number));
        values.push(BucketAutoValue {
            number,
            value: value.expect("validated bucketAuto value"),
        });
    }
    if values.is_empty() {
        return finish_group_output(Vec::new(), plan);
    }

    values.sort_by(|left, right| {
        left.number
            .partial_cmp(&right.number)
            .expect("finite bucketAuto values compare")
    });
    let unique_starts = values
        .iter()
        .enumerate()
        .filter_map(|(index, value)| {
            (index == 0 || values[index - 1].number != value.number).then_some(index)
        })
        .collect::<Vec<_>>();
    let bucket_count = bucket.buckets.min(unique_starts.len());
    let mut ranges = Vec::with_capacity(bucket_count);
    for bucket_index in 0..bucket_count {
        let start_unique = bucket_index * unique_starts.len() / bucket_count;
        let end_unique = (bucket_index + 1) * unique_starts.len() / bucket_count;
        let start = unique_starts[start_unique];
        let end = if bucket_index + 1 == bucket_count {
            values.len()
        } else {
            unique_starts[end_unique]
        };
        let lower = &values[start];
        let upper = &values[end - 1];
        let upper_value = if bucket_index + 1 == bucket_count {
            upper.value.clone()
        } else {
            values[unique_starts[end_unique]].value.clone()
        };
        let mut key = Map::new();
        key.insert("min".into(), lower.value.clone());
        key.insert("max".into(), upper_value);
        ranges.push(BucketAutoRange {
            upper: if bucket_index + 1 == bucket_count {
                upper.number
            } else {
                values[unique_starts[end_unique]].number
            },
            key: Value::Object(key),
        });
    }

    let mut groups = ranges
        .iter()
        .map(|range| Some(accumulators::new_group(range.key.clone(), &bucket.output)))
        .collect::<Vec<_>>();
    let mut collected_values = 0;
    for (record, number) in input_records {
        let bucket_index = ranges
            .iter()
            .position(|range| number < range.upper)
            .unwrap_or(ranges.len() - 1);
        let group = groups[bucket_index]
            .as_mut()
            .expect("bucketAuto group was initialized");
        accumulators::accumulate_record(group, record, &bucket.output, &mut collected_values)?;
    }

    let output = groups
        .into_iter()
        .flatten()
        .map(|group| accumulators::finish_group(group, &bucket.output))
        .collect::<Result<Vec<_>, _>>()?;
    finish_group_output(output, plan)
}
