use super::super::{IndexFile, IndexKey};
use super::{CompoundOrdered, equality::equality_prefix};
use serde_json::{Map, Value};

impl IndexFile {
    pub(crate) fn lookup_ordered_for_field(
        &self,
        field: &str,
        descending: bool,
    ) -> Result<Option<(String, Vec<usize>)>, super::super::IndexError> {
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
    ) -> Result<Option<CompoundOrdered>, super::super::IndexError> {
        if fields.is_empty() {
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

fn ordered_records(
    index: &super::super::SecondaryIndex,
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
        .all(|(actual, expected)| super::super::ordering::compare_keys(actual, expected).is_eq())
}
