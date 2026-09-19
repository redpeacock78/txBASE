use super::{IndexEntry, IndexKey, SecondaryIndex, ordering};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::cmp::Ordering;

const HISTOGRAM_BUCKET_LIMIT: usize = 16;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CollectionStatistics {
    pub(super) active_record_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) indexes: Option<Vec<IndexStatistics>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct IndexStatistics {
    pub(super) name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) histogram: Vec<HistogramBucket>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct HistogramBucket {
    lower: IndexKey,
    upper: IndexKey,
    distinct_key_count: usize,
    record_count: usize,
}

pub(super) fn build(
    active_record_count: usize,
    indexes: &[SecondaryIndex],
) -> CollectionStatistics {
    CollectionStatistics {
        active_record_count,
        indexes: Some(
            indexes
                .iter()
                .map(|index| IndexStatistics {
                    name: index.definition().name().to_owned(),
                    histogram: build_histogram(index),
                })
                .collect(),
        ),
    }
}

impl CollectionStatistics {
    pub(super) fn histogram_bucket_count(&self, index_name: &str) -> usize {
        self.indexes
            .as_ref()
            .and_then(|indexes| indexes.iter().find(|index| index.name == index_name))
            .map_or(0, |index| index.histogram.len())
    }

    pub(super) fn range_estimate(
        &self,
        index_name: &str,
        lower: Option<(&Value, bool)>,
        upper: Option<(&Value, bool)>,
    ) -> Option<usize> {
        let index = self
            .indexes
            .as_ref()?
            .iter()
            .find(|index| index.name == index_name)?;
        let domain = lower
            .or(upper)
            .map(|(value, _)| ordering::value_domain(value))?;
        if lower.zip(upper).is_some_and(|((left, _), (right, _))| {
            ordering::value_domain(left) != ordering::value_domain(right)
        }) {
            return Some(0);
        }
        Some(
            index
                .histogram
                .iter()
                .filter(|bucket| {
                    ordering::key_domain(&bucket.lower) == domain
                        && bucket_overlaps(bucket, lower, upper)
                })
                .map(|bucket| bucket.record_count)
                .sum(),
        )
    }

    pub(super) fn matches_expected(&self, expected: &Self) -> bool {
        self.active_record_count == expected.active_record_count
            && self
                .indexes
                .as_ref()
                .map_or(true, |indexes| Some(indexes) == expected.indexes.as_ref())
    }
}

fn build_histogram(index: &SecondaryIndex) -> Vec<HistogramBucket> {
    if index.definition().fields().len() != 1 || index.entries().is_empty() {
        return Vec::new();
    }

    let mut histogram = Vec::new();
    let entries = index.entries();
    let mut domain_start = 0;
    while domain_start < entries.len() {
        let domain = ordering::key_domain(entries[domain_start].key());
        let domain_end = entries[domain_start..]
            .iter()
            .position(|entry| ordering::key_domain(entry.key()) != domain)
            .map_or(entries.len(), |offset| domain_start + offset);
        append_domain_histogram(&mut histogram, &entries[domain_start..domain_end]);
        domain_start = domain_end;
    }
    histogram
}

fn append_domain_histogram(histogram: &mut Vec<HistogramBucket>, entries: &[IndexEntry]) {
    let total_records = entries
        .iter()
        .map(|entry| entry.records().len())
        .sum::<usize>();
    let target_records = total_records.div_ceil(HISTOGRAM_BUCKET_LIMIT).max(1);
    let mut current: Option<HistogramBucket> = None;
    for (position, entry) in entries.iter().enumerate() {
        if let Some(bucket) = current.as_mut() {
            bucket.upper = entry.key().clone();
            bucket.distinct_key_count += 1;
            bucket.record_count += entry.records().len();
        } else {
            current = Some(HistogramBucket {
                lower: entry.key().clone(),
                upper: entry.key().clone(),
                distinct_key_count: 1,
                record_count: entry.records().len(),
            });
        }
        if current
            .as_ref()
            .is_some_and(|bucket| bucket.record_count >= target_records)
            && position + 1 < entries.len()
        {
            histogram.push(current.take().expect("histogram bucket was initialized"));
        }
    }
    if let Some(bucket) = current {
        histogram.push(bucket);
    }
}

fn bucket_overlaps(
    bucket: &HistogramBucket,
    lower: Option<(&Value, bool)>,
    upper: Option<(&Value, bool)>,
) -> bool {
    lower.is_none_or(|(value, inclusive)| {
        match ordering::compare_key_to_value(&bucket.upper, value) {
            Some(Ordering::Greater) => true,
            Some(Ordering::Equal) => inclusive,
            Some(Ordering::Less) | None => false,
        }
    }) && upper.is_none_or(|(value, inclusive)| {
        match ordering::compare_key_to_value(&bucket.lower, value) {
            Some(Ordering::Less) => true,
            Some(Ordering::Equal) => inclusive,
            Some(Ordering::Greater) | None => false,
        }
    })
}
