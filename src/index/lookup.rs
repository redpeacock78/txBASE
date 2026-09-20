use super::{IndexError, IndexFile, IndexKey, ordering};
use serde_json::{Map, Value};

type CompoundOrdered = (String, Vec<String>, Vec<i8>, Vec<usize>);

impl IndexFile {
    pub(crate) fn has_exact_fields(&self, fields: &[&str]) -> bool {
        self.indexes.iter().any(|index| {
            index.definition.fields.len() == fields.len()
                && index
                    .definition
                    .fields
                    .iter()
                    .zip(fields)
                    .all(|(indexed, requested)| indexed == *requested)
        })
    }

    pub(crate) fn equality_selectivity_estimate(&self, field: &str) -> Option<usize> {
        let index = self.indexes.iter().find(|index| {
            index.definition.fields.len() == 1 && index.definition.fields[0] == field
        })?;
        let distinct_keys = index.entries.len();
        if distinct_keys == 0 {
            return Some(0);
        }
        let records = self.active_record_count();
        let remainder = if records % distinct_keys == 0 { 0 } else { 1 };
        Some(records / distinct_keys + remainder)
    }

    pub(crate) fn equality_fanout_estimate(&self, fields: &[&str]) -> Option<usize> {
        let index = self.indexes.iter().find(|index| {
            index.definition.fields.len() == fields.len()
                && index
                    .definition
                    .fields
                    .iter()
                    .zip(fields)
                    .all(|(indexed, requested)| indexed == *requested)
        })?;
        let joinable_entries = index
            .entries
            .iter()
            .filter(|entry| is_joinable_key(&entry.key));
        let (distinct_keys, indexed_records) = joinable_entries.fold(
            (0usize, 0usize),
            |(distinct_keys, indexed_records), entry| {
                (
                    distinct_keys + 1,
                    indexed_records.saturating_add(entry.records.len()),
                )
            },
        );
        if distinct_keys == 0 {
            return Some(0);
        }
        Some(indexed_records.div_ceil(distinct_keys))
    }

    pub(crate) fn range_selectivity_estimate(
        &self,
        field: &str,
        lower: Option<(&Value, bool)>,
        upper: Option<(&Value, bool)>,
    ) -> Option<usize> {
        let index = self.indexes.iter().find(|index| {
            index.definition.fields.len() == 1 && index.definition.fields[0] == field
        })?;
        self.statistics
            .as_ref()?
            .range_estimate(&index.definition.name, lower, upper)
    }

    pub fn lookup_eq(&self, index_name: &str, value: &Value) -> Result<Vec<usize>, IndexError> {
        let key = IndexKey::from_value(Some(value))?;
        let index = self
            .indexes
            .iter()
            .find(|index| index.definition.name == index_name)
            .ok_or_else(|| IndexError::Invalid(format!("index not found: {index_name}")))?;
        Ok(index
            .entries
            .binary_search_by(|entry| ordering::compare_keys(&entry.key, &key))
            .ok()
            .map(|position| index.entries[position].records.clone())
            .unwrap_or_default())
    }

    pub(crate) fn lookup_eq_for_field(
        &self,
        field: &str,
        value: &Value,
    ) -> Result<Option<(String, Vec<usize>)>, IndexError> {
        let Some(index) = self.indexes.iter().find(|index| {
            index.definition.fields.len() == 1 && index.definition.fields[0] == field
        }) else {
            return Ok(None);
        };
        let key = IndexKey::from_value(Some(value))?;
        Ok(Some((
            index.definition.name.clone(),
            index
                .entries
                .binary_search_by(|entry| ordering::compare_keys(&entry.key, &key))
                .ok()
                .map(|position| index.entries[position].records.clone())
                .unwrap_or_default(),
        )))
    }

    pub(crate) fn lookup_eq_for_fields(
        &self,
        fields: &[&str],
        values: &[Value],
    ) -> Result<Option<(String, Vec<usize>)>, IndexError> {
        let Some(index) = self.indexes.iter().find(|index| {
            index.definition.fields.len() == fields.len()
                && index
                    .definition
                    .fields
                    .iter()
                    .zip(fields)
                    .all(|(indexed, requested)| indexed == *requested)
        }) else {
            return Ok(None);
        };
        if values.len() != fields.len() {
            return Ok(None);
        }
        let key = IndexKey::from_values(values.iter().map(Some).collect())?;
        Ok(Some((
            index.definition.name.clone(),
            index
                .entries
                .binary_search_by(|entry| {
                    ordering::compare_index_keys(&entry.key, &key, index.definition.directions())
                })
                .ok()
                .map(|position| index.entries[position].records.clone())
                .unwrap_or_default(),
        )))
    }

    pub(crate) fn lookup_range_for_field(
        &self,
        field: &str,
        lower: Option<(&Value, bool)>,
        upper: Option<(&Value, bool)>,
    ) -> Result<Option<(String, Vec<usize>)>, IndexError> {
        if lower.is_none() && upper.is_none() {
            return Err(IndexError::Invalid("range must have a bound".into()));
        }
        let Some(index) = self.indexes.iter().find(|index| {
            index.definition.fields.len() == 1 && index.definition.fields[0] == field
        }) else {
            return Ok(None);
        };
        let domain = lower
            .or(upper)
            .map(|(value, _)| ordering::value_domain(value))
            .expect("range has at least one bound");
        if lower.zip(upper).is_some_and(|((left, _), (right, _))| {
            ordering::value_domain(left) != ordering::value_domain(right)
        }) {
            return Ok(Some((index.definition.name.clone(), Vec::new())));
        }
        let domain_start = lower_bound(&index.entries, |entry| {
            ordering::key_domain(&entry.key) >= domain
        });
        let domain_end = lower_bound(&index.entries, |entry| {
            ordering::key_domain(&entry.key) > domain
        });
        let entries = &index.entries[domain_start..domain_end];
        let start = lower
            .map(|(value, inclusive)| {
                lower_bound(entries, |entry| {
                    let ordering = ordering::compare_key_to_value(&entry.key, value)
                        .expect("range bound and index key share a domain");
                    if inclusive {
                        !ordering.is_lt()
                    } else {
                        ordering.is_gt()
                    }
                })
            })
            .unwrap_or_default();
        let end = upper
            .map(|(value, inclusive)| {
                lower_bound(entries, |entry| {
                    let ordering = ordering::compare_key_to_value(&entry.key, value)
                        .expect("range bound and index key share a domain");
                    if inclusive {
                        ordering.is_gt()
                    } else {
                        !ordering.is_lt()
                    }
                })
            })
            .unwrap_or(entries.len());
        let mut records = entries[start.min(end)..end]
            .iter()
            .flat_map(|entry| entry.records.iter().copied())
            .collect::<Vec<_>>();
        records.sort_unstable();
        Ok(Some((index.definition.name.clone(), records)))
    }

    pub(crate) fn lookup_ordered_for_field(
        &self,
        field: &str,
        descending: bool,
    ) -> Result<Option<(String, Vec<usize>)>, IndexError> {
        let Some(index) = self.indexes.iter().find(|index| {
            index.definition.fields.len() == 1 && index.definition.fields[0] == field
        }) else {
            return Ok(None);
        };
        let mut records = Vec::new();
        if descending {
            for entry in index.entries.iter().rev() {
                records.extend(entry.records.iter().copied());
            }
        } else {
            for entry in &index.entries {
                records.extend(entry.records.iter().copied());
            }
        }
        Ok(Some((index.definition.name.clone(), records)))
    }

    pub(crate) fn lookup_ordered_for_fields(
        &self,
        fields: &[&str],
        directions: &[i8],
        filter: &Map<String, Value>,
    ) -> Result<Option<CompoundOrdered>, IndexError> {
        if fields.len() < 2 {
            return Ok(None);
        }
        let mut best = None;
        for index in self
            .indexes
            .iter()
            .filter(|index| index.definition.fields.len() >= fields.len())
        {
            for offset in 0..=index.definition.fields.len() - fields.len() {
                let indexed_fields = &index.definition.fields[offset..offset + fields.len()];
                if !indexed_fields
                    .iter()
                    .zip(fields.iter())
                    .all(|(indexed, requested)| indexed == *requested)
                {
                    continue;
                }
                let Some(reverse) = traversal_is_reverse(
                    &index.definition.directions[offset..offset + fields.len()],
                    directions,
                ) else {
                    continue;
                };
                let Some(equality_prefix) =
                    equality_prefix(filter, &index.definition.fields[..offset])
                else {
                    continue;
                };
                let records = ordered_records(index, reverse, &equality_prefix);
                let score = (
                    records.len(),
                    index.definition.fields.len(),
                    offset,
                    index.definition.name.clone(),
                );
                let replace = best
                    .as_ref()
                    .is_none_or(|(best_score, _, _, _, _)| score < *best_score);
                if replace {
                    best = Some((
                        score,
                        index.definition.name.clone(),
                        index.definition.fields.clone(),
                        index.definition.directions.clone(),
                        records,
                    ));
                }
            }
        }
        Ok(best.map(|(_, name, fields, directions, records)| (name, fields, directions, records)))
    }
}

fn is_joinable_key(key: &IndexKey) -> bool {
    match key {
        IndexKey::Scalar(Value::Bool(_) | Value::Number(_) | Value::String(_)) => true,
        IndexKey::Compound(values) => values.iter().all(is_joinable_key),
        IndexKey::Missing | IndexKey::Null | IndexKey::Scalar(_) => false,
    }
}

fn traversal_is_reverse(indexed: &[i8], requested: &[i8]) -> Option<bool> {
    if indexed == requested {
        return Some(false);
    }
    indexed
        .iter()
        .zip(requested)
        .all(|(indexed, requested)| *indexed == -*requested)
        .then_some(true)
}

fn equality_prefix(filter: &Map<String, Value>, fields: &[String]) -> Option<Vec<IndexKey>> {
    fields
        .iter()
        .map(|field| {
            let value = exact_equality_value(filter.get(field)?)?;
            IndexKey::from_value(Some(value)).ok()
        })
        .collect()
}

fn exact_equality_value(condition: &Value) -> Option<&Value> {
    let Some(object) = condition.as_object() else {
        return (!condition.is_array() && !condition.is_object()).then_some(condition);
    };
    object.get("$eq")
}

fn ordered_records(
    index: &super::SecondaryIndex,
    reverse: bool,
    equality_prefix: &[IndexKey],
) -> Vec<usize> {
    let mut records = Vec::new();
    if reverse {
        for entry in index.entries().iter().rev() {
            if key_has_prefix(entry.key(), equality_prefix) {
                records.extend(entry.records().iter().copied());
            }
        }
    } else {
        for entry in index.entries() {
            if key_has_prefix(entry.key(), equality_prefix) {
                records.extend(entry.records().iter().copied());
            }
        }
    }
    records
}

fn key_has_prefix(key: &IndexKey, prefix: &[IndexKey]) -> bool {
    if prefix.is_empty() {
        return true;
    }
    let IndexKey::Compound(parts) = key else {
        return false;
    };
    parts
        .iter()
        .zip(prefix)
        .all(|(actual, expected)| ordering::compare_keys(actual, expected).is_eq())
}

fn lower_bound<T>(items: &[T], predicate: impl Fn(&T) -> bool) -> usize {
    let mut left = 0;
    let mut right = items.len();
    while left < right {
        let middle = left + (right - left) / 2;
        if predicate(&items[middle]) {
            right = middle;
        } else {
            left = middle + 1;
        }
    }
    left
}
