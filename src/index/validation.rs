use super::{
    IndexDefinition, IndexEntry, IndexError, IndexFile, IndexKey, SecondaryIndex, ordering,
};
use crate::dbf::DbfTable;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn validate_shape(index_file: &IndexFile) -> Result<(), IndexError> {
    if index_file.format != super::INDEX_FORMAT {
        return Err(IndexError::Invalid(format!(
            "unsupported format: {}",
            index_file.format
        )));
    }
    if index_file.version != super::INDEX_VERSION {
        return Err(IndexError::Invalid(format!(
            "unsupported version: {}",
            index_file.version
        )));
    }
    if index_file.indexes.is_empty() {
        return Err(IndexError::Invalid("index file has no indexes".into()));
    }

    let mut names = BTreeSet::new();
    for index in &index_file.indexes {
        validate_definition(&index.definition, None, &mut names)?;

        let mut previous_key = None;
        for entry in &index.entries {
            if entry.records.is_empty() {
                return Err(IndexError::Invalid(format!(
                    "index {} has an empty record list",
                    index.definition.name
                )));
            }
            validate_key(
                &entry.key,
                index.definition.fields.len(),
                &index.definition.name,
            )?;
            let key = &entry.key;
            if previous_key.as_ref().is_some_and(|previous| {
                ordering::compare_keys(previous, key) != std::cmp::Ordering::Less
            }) {
                return Err(IndexError::Invalid(format!(
                    "index {} entries are not sorted",
                    index.definition.name
                )));
            }
            previous_key = Some(key.clone());

            let mut previous_record = 0;
            for record in &entry.records {
                if *record == 0 || *record <= previous_record {
                    return Err(IndexError::Invalid(format!(
                        "index {} record numbers are not strictly increasing",
                        index.definition.name
                    )));
                }
                previous_record = *record;
            }
        }
    }
    Ok(())
}

pub(super) fn validate_for_table(
    index_file: &IndexFile,
    table: &DbfTable,
) -> Result<(), IndexError> {
    validate_shape(index_file)?;
    let definitions = index_file
        .indexes
        .iter()
        .map(|index| index.definition.clone())
        .collect::<Vec<_>>();
    let expected = build_indexes(table, &definitions)?;
    if expected != index_file.indexes {
        return Err(IndexError::Invalid(
            "index entries do not match the current DBF records".into(),
        ));
    }
    Ok(())
}

pub(super) fn build_indexes(
    table: &DbfTable,
    definitions: &[IndexDefinition],
) -> Result<Vec<SecondaryIndex>, IndexError> {
    if definitions.is_empty() {
        return Err(IndexError::Invalid("at least one index is required".into()));
    }
    let mut names = BTreeSet::new();
    let mut indexes = Vec::with_capacity(definitions.len());
    for definition in definitions {
        validate_definition(definition, Some(table), &mut names)?;

        let mut grouped = BTreeMap::<String, (IndexKey, Vec<usize>)>::new();
        for record in table.active_records() {
            let key = IndexKey::from_values(
                definition
                    .fields
                    .iter()
                    .map(|field| record.values.get(field))
                    .collect(),
            )?;
            let token = key_token(&key)?;
            grouped
                .entry(token)
                .or_insert_with(|| (key, Vec::new()))
                .1
                .push(record.number);
        }
        let mut entries = grouped
            .into_values()
            .map(|(key, records)| IndexEntry { key, records })
            .collect::<Vec<_>>();
        entries.sort_by(|left, right| ordering::compare_keys(&left.key, &right.key));
        let mut merged = Vec::<IndexEntry>::with_capacity(entries.len());
        for mut entry in entries {
            if let Some(previous) = merged.last_mut() {
                if ordering::compare_keys(&previous.key, &entry.key) == std::cmp::Ordering::Equal {
                    previous.records.append(&mut entry.records);
                    previous.records.sort_unstable();
                    continue;
                }
            }
            merged.push(entry);
        }
        indexes.push(SecondaryIndex {
            definition: definition.clone(),
            entries: merged,
        });
    }
    Ok(indexes)
}

fn validate_definition(
    definition: &IndexDefinition,
    table: Option<&DbfTable>,
    names: &mut BTreeSet<String>,
) -> Result<(), IndexError> {
    if definition.name.trim().is_empty() {
        return Err(IndexError::Invalid("index name is empty".into()));
    }
    if !names.insert(definition.name.clone()) {
        return Err(IndexError::Invalid(format!(
            "duplicate index name: {}",
            definition.name
        )));
    }
    if definition.fields.is_empty() {
        return Err(IndexError::Invalid(format!(
            "index {} has no fields",
            definition.name
        )));
    }
    let mut fields = BTreeSet::new();
    for field_name in &definition.fields {
        if field_name.trim().is_empty() {
            return Err(IndexError::Invalid(format!(
                "index {} has an empty field",
                definition.name
            )));
        }
        if !fields.insert(field_name) {
            return Err(IndexError::Invalid(format!(
                "index {} repeats field: {}",
                definition.name, field_name
            )));
        }
        if table.is_some_and(|table| {
            !table.fields.iter().any(|field| {
                field.name == *field_name
                    && field.flags & 0x01 == 0
                    && !field.name.eq_ignore_ascii_case("_NULLFLAGS")
            })
        }) {
            return Err(IndexError::Invalid(format!(
                "index field is not a user field: {}",
                field_name
            )));
        }
    }
    Ok(())
}

fn validate_key(key: &IndexKey, field_count: usize, index_name: &str) -> Result<(), IndexError> {
    match (field_count, key) {
        (1, IndexKey::Scalar(Value::Array(_)) | IndexKey::Scalar(Value::Object(_))) => Err(
            IndexError::Invalid("only scalar values can be indexed".into()),
        ),
        (1, IndexKey::Missing | IndexKey::Null | IndexKey::Scalar(_)) => Ok(()),
        (1, IndexKey::Compound(_)) => Err(IndexError::Invalid(format!(
            "index {index_name} has a compound key for one field"
        ))),
        (_, IndexKey::Compound(parts)) if parts.len() == field_count => {
            if parts.iter().any(|part| {
                matches!(
                    part,
                    IndexKey::Compound(_)
                        | IndexKey::Scalar(Value::Array(_))
                        | IndexKey::Scalar(Value::Object(_))
                )
            }) {
                return Err(IndexError::Invalid(
                    "only scalar values can be indexed".into(),
                ));
            }
            Ok(())
        }
        (_, IndexKey::Compound(_)) => Err(IndexError::Invalid(format!(
            "index {index_name} has a compound key with the wrong field count"
        ))),
        (_, _) => Err(IndexError::Invalid(format!(
            "index {index_name} requires a compound key"
        ))),
    }
}

fn key_token(key: &IndexKey) -> Result<String, IndexError> {
    Ok(serde_json::to_string(key)?)
}
