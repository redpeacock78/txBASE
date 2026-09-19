use super::{IndexError, IndexFile, IndexKey, ordering};
use serde_json::Value;

type CompoundOrdered = (String, Vec<String>, Vec<usize>);

impl IndexFile {
    pub fn lookup_eq(&self, index_name: &str, value: &Value) -> Result<Vec<usize>, IndexError> {
        let key = IndexKey::from_value(Some(value))?;
        let index = self
            .indexes
            .iter()
            .find(|index| index.definition.name == index_name)
            .ok_or_else(|| IndexError::Invalid(format!("index not found: {index_name}")))?;
        Ok(index
            .entries
            .iter()
            .find(|entry| ordering::compare_keys(&entry.key, &key) == std::cmp::Ordering::Equal)
            .map(|entry| entry.records.clone())
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
                .iter()
                .find(|entry| ordering::compare_keys(&entry.key, &key) == std::cmp::Ordering::Equal)
                .map(|entry| entry.records.clone())
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
        descending: bool,
    ) -> Result<Option<CompoundOrdered>, IndexError> {
        if fields.len() < 2 {
            return Ok(None);
        }
        let Some(index) = self
            .indexes
            .iter()
            .filter(|index| index.definition.fields.len() >= fields.len())
            .filter(|index| {
                index
                    .definition
                    .fields
                    .iter()
                    .zip(fields.iter())
                    .all(|(indexed, requested)| indexed == *requested)
            })
            .max_by_key(|index| index.definition.fields.len())
        else {
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
        Ok(Some((
            index.definition.name.clone(),
            index.definition.fields.clone(),
            records,
        )))
    }
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
