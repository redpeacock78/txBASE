use super::super::{IndexError, IndexFile, IndexKey, ordering};
use serde_json::{Map, Value};

impl IndexFile {
    pub(crate) fn compound_indexes(&self) -> impl Iterator<Item = (&str, &[String])> {
        self.indexes.iter().filter_map(|index| {
            (index.definition.fields.len() > 1 && index.definition.collation().is_none()).then_some(
                (
                    index.definition.name.as_str(),
                    index.definition.fields.as_slice(),
                ),
            )
        })
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn has_exact_fields(&self, fields: &[&str]) -> bool {
        self.indexes.iter().any(|index| {
            index.definition.collation().is_none()
                && index.definition.fields.len() == fields.len()
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
            index.definition.collation().is_none()
                && index.definition.fields.len() == 1
                && index.definition.fields[0] == field
        })?;
        let distinct_keys = index.entries.len();
        if distinct_keys == 0 {
            return Some(0);
        }
        let records = self.active_record_count();
        let remainder = if records % distinct_keys == 0 { 0 } else { 1 };
        Some(records / distinct_keys + remainder)
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn equality_fanout_estimate(&self, fields: &[&str]) -> Option<usize> {
        let index = self.indexes.iter().find(|index| {
            index.definition.collation().is_none()
                && index.definition.fields.len() == fields.len()
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

    pub fn lookup_eq(&self, index_name: &str, value: &Value) -> Result<Vec<usize>, IndexError> {
        let index = self
            .indexes
            .iter()
            .find(|index| index.definition.name == index_name)
            .ok_or_else(|| IndexError::Invalid(format!("index not found: {index_name}")))?;
        if index.definition.collation().is_some() {
            return Err(IndexError::Invalid(
                "collated indexes cannot serve equality lookup".into(),
            ));
        }
        let key = IndexKey::from_value(Some(value), None)?;
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
            index.definition.collation().is_none()
                && index.definition.fields.len() == 1
                && index.definition.fields[0] == field
        }) else {
            return Ok(None);
        };
        let key = IndexKey::from_value(Some(value), None)?;
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

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn lookup_eq_for_fields(
        &self,
        fields: &[&str],
        values: &[Value],
    ) -> Result<Option<(String, Vec<usize>)>, IndexError> {
        let Some(index) = self.indexes.iter().find(|index| {
            index.definition.collation().is_none()
                && index.definition.fields.len() == fields.len()
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
            .find(|index| index.definition.name == name && index.definition.collation().is_none())
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
        let key = IndexKey::from_values(values.iter().map(Some).collect(), None)?;
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
}

pub(super) fn equality_prefix(
    filter: &Map<String, Value>,
    fields: &[String],
) -> Option<Vec<IndexKey>> {
    fields
        .iter()
        .map(|field| {
            let value = exact_equality_value(filter.get(field)?)?;
            IndexKey::from_value(Some(value), None).ok()
        })
        .collect()
}

fn exact_equality_value(condition: &Value) -> Option<&Value> {
    let Some(object) = condition.as_object() else {
        return (!condition.is_array() && !condition.is_object()).then_some(condition);
    };
    object.get("$eq")
}

#[cfg(not(target_arch = "wasm32"))]
fn is_joinable_key(key: &IndexKey) -> bool {
    match key {
        IndexKey::Scalar(Value::Bool(_) | Value::Number(_) | Value::String(_)) => true,
        IndexKey::Compound(values) => values.iter().all(is_joinable_key),
        IndexKey::Missing | IndexKey::Null | IndexKey::Scalar(_) => false,
    }
}
