use super::super::{IndexError, IndexFile, IndexKey, ordering};
use super::equality::equality_prefix;
use serde_json::{Map, Value};

impl IndexFile {
    pub(crate) fn range_selectivity_estimate(
        &self,
        field: &str,
        lower: Option<(&Value, bool)>,
        upper: Option<(&Value, bool)>,
    ) -> Option<usize> {
        let index = self.indexes.iter().find(|index| {
            index.definition.collation().is_none()
                && index.definition.fields.len() == 1
                && index.definition.fields[0] == field
        })?;
        self.statistics
            .as_ref()?
            .range_estimate(&index.definition.name, lower, upper)
    }

    pub(crate) fn lookup_eq_prefix_for_named_fields(
        &self,
        name: &str,
        fields: &[&str],
        values: &[Value],
    ) -> Result<Option<Vec<usize>>, IndexError> {
        let Some(index) = self
            .indexes
            .iter()
            .find(|index| index.definition.name == name && index.definition.collation().is_none())
        else {
            return Ok(None);
        };
        if fields.is_empty()
            || values.len() != fields.len()
            || fields.len() >= index.definition.fields.len()
            || !index
                .definition
                .fields
                .iter()
                .zip(fields)
                .all(|(indexed, requested)| indexed == *requested)
        {
            return Ok(None);
        }
        let prefix = values
            .iter()
            .map(|value| IndexKey::from_value(Some(value), None))
            .collect::<Result<Vec<_>, _>>()?;
        let prefix_start = lower_bound(&index.entries, |entry| {
            !compare_index_prefix(&entry.key, &prefix, index.definition.directions()).is_lt()
        });
        let prefix_entries = &index.entries[prefix_start..];
        let prefix_end = lower_bound(prefix_entries, |entry| {
            compare_index_prefix(&entry.key, &prefix, index.definition.directions()).is_gt()
        });
        let mut records = prefix_entries[..prefix_end]
            .iter()
            .flat_map(|entry| entry.records.iter().copied())
            .collect::<Vec<_>>();
        records.sort_unstable();
        Ok(Some(records))
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
            index.definition.collation().is_none()
                && index.definition.fields.len() == 1
                && index.definition.fields[0] == field
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
            .map(|(value, inclusive)| {
                IndexKey::from_value(Some(value), None).map(|key| (key, inclusive))
            })
            .transpose()?;
        let upper = upper
            .map(|(value, inclusive)| {
                IndexKey::from_value(Some(value), None).map(|key| (key, inclusive))
            })
            .transpose()?;

        for index in &self.indexes {
            if index.definition.collation().is_some() {
                continue;
            }
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
