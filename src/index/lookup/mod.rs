use super::{IndexError, IndexFile, IndexKey, ordering};
use serde_json::{Map, Value};

pub(crate) type CompoundOrdered = (String, Vec<String>, Vec<i8>, Vec<usize>);

mod ordered;

impl IndexFile {
    pub(crate) fn compound_indexes(&self) -> impl Iterator<Item = (&str, &[String])> {
        self.indexes.iter().filter_map(|index| {
            (index.definition.fields.len() > 1).then_some((
                index.definition.name.as_str(),
                index.definition.fields.as_slice(),
            ))
        })
    }

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

    pub(crate) fn index_traversal_cost(&self, name: &str) -> Option<usize> {
        let index = self
            .indexes
            .iter()
            .find(|index| index.definition.name == name)?;
        let entry_count = index.entries.len();
        // ponytail: normalize tiny sidecars to the base probe; page/cache terms need measurements.
        Some(if entry_count <= 2 {
            0
        } else {
            (entry_count - 1).ilog2() as usize
        })
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
        let name = index.definition.name.clone();
        Ok(self
            .lookup_eq_for_named_fields(&name, fields, values)?
            .map(|records| (name, records)))
    }

    pub(crate) fn lookup_eq_for_named_fields(
        &self,
        name: &str,
        fields: &[&str],
        values: &[Value],
    ) -> Result<Option<Vec<usize>>, IndexError> {
        let Some(index) = self
            .indexes
            .iter()
            .find(|index| index.definition.name == name)
        else {
            return Ok(None);
        };
        if values.len() != fields.len()
            || index.definition.fields.len() != fields.len()
            || !index
                .definition
                .fields
                .iter()
                .zip(fields)
                .all(|(indexed, requested)| indexed == *requested)
        {
            return Ok(None);
        }
        let key = IndexKey::from_values(values.iter().map(Some).collect())?;
        Ok(Some(
            index
                .entries
                .binary_search_by(|entry| {
                    ordering::compare_index_keys(&entry.key, &key, index.definition.directions())
                })
                .ok()
                .map(|position| index.entries[position].records.clone())
                .unwrap_or_default(),
        ))
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

    pub(crate) fn lookup_compound_range_for_field(
        &self,
        field: &str,
        lower: Option<(&Value, bool)>,
        upper: Option<(&Value, bool)>,
        filter: &Map<String, Value>,
    ) -> Result<Option<(String, Vec<usize>)>, IndexError> {
        if lower.is_none() && upper.is_none() {
            return Err(IndexError::Invalid("range must have a bound".into()));
        }
        let lower = lower
            .map(|(value, inclusive)| IndexKey::from_value(Some(value)).map(|key| (key, inclusive)))
            .transpose()?;
        let upper = upper
            .map(|(value, inclusive)| IndexKey::from_value(Some(value)).map(|key| (key, inclusive)))
            .transpose()?;

        for index in &self.indexes {
            let Some(offset) = index
                .definition
                .fields
                .iter()
                .position(|indexed| indexed == field)
            else {
                continue;
            };
            if offset == 0 {
                continue;
            }
            let Some(prefix) = equality_prefix(filter, &index.definition.fields[..offset]) else {
                continue;
            };
            let prefix_start = lower_bound(&index.entries, |entry| {
                !compare_index_prefix(&entry.key, &prefix, index.definition.directions()).is_lt()
            });
            let prefix_entries = &index.entries[prefix_start..];
            let prefix_end = lower_bound(prefix_entries, |entry| {
                compare_index_prefix(&entry.key, &prefix, index.definition.directions()).is_gt()
            });
            let mut records = Vec::new();
            // ponytail: scan only the equality-prefix interval; add range-field seeks if prefix groups become large.
            for entry in &prefix_entries[..prefix_end] {
                let IndexKey::Compound(parts) = &entry.key else {
                    continue;
                };
                let Some(actual) = parts.get(offset) else {
                    continue;
                };
                let in_lower = lower.as_ref().is_none_or(|(bound, inclusive)| {
                    let ordering = ordering::compare_keys(actual, bound);
                    if *inclusive {
                        !ordering.is_lt()
                    } else {
                        ordering.is_gt()
                    }
                });
                let in_upper = upper.as_ref().is_none_or(|(bound, inclusive)| {
                    let ordering = ordering::compare_keys(actual, bound);
                    if *inclusive {
                        !ordering.is_gt()
                    } else {
                        ordering.is_lt()
                    }
                });
                if in_lower && in_upper {
                    records.extend(entry.records.iter().copied());
                }
            }
            records.sort_unstable();
            return Ok(Some((index.definition.name.clone(), records)));
        }
        Ok(None)
    }
}

fn is_joinable_key(key: &IndexKey) -> bool {
    match key {
        IndexKey::Scalar(Value::Bool(_) | Value::Number(_) | Value::String(_)) => true,
        IndexKey::Compound(values) => values.iter().all(is_joinable_key),
        IndexKey::Missing | IndexKey::Null | IndexKey::Scalar(_) => false,
    }
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

fn compare_index_prefix(
    key: &IndexKey,
    prefix: &[IndexKey],
    directions: &[i8],
) -> std::cmp::Ordering {
    let IndexKey::Compound(parts) = key else {
        return std::cmp::Ordering::Greater;
    };
    parts
        .iter()
        .zip(prefix)
        .enumerate()
        .map(|(position, (actual, expected))| {
            let ordering = ordering::compare_keys(actual, expected);
            if directions.get(position) == Some(&-1) {
                ordering.reverse()
            } else {
                ordering
            }
        })
        .find(|ordering| !ordering.is_eq())
        .unwrap_or(std::cmp::Ordering::Equal)
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
