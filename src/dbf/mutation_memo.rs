use super::*;

impl DbfTable {
    pub(super) fn prepare_existing_storage(
        &self,
        index: usize,
        values: &Map<String, Value>,
        changed_fields: Option<&BTreeSet<String>>,
    ) -> Result<PreparedStorage, DbfError> {
        let mut storage_values = values.clone();
        let mut memo_updates = self.memo_updates.clone();
        for field in self.fields.iter().filter(|field| is_sidecar_field(field)) {
            let changed = changed_fields.is_none_or(|fields| fields.contains(&field.name));
            let key = (index, field.name.clone());
            if !changed || values.get(&field.name) == self.records[index].values.get(&field.name) {
                if self.memo.is_some() {
                    storage_values.insert(
                        field.name.clone(),
                        self.stored_values[index][&field.name].clone(),
                    );
                }
                continue;
            }

            if self.memo.is_none() {
                storage_values.insert(
                    field.name.clone(),
                    storage_value_without_sidecar(&values[&field.name], field)?,
                );
                continue;
            }

            if self.memo.is_some() {
                if !field.is_binary() {
                    let text = value_text(&values[&field.name], field)?;
                    storage_values.insert(field.name.clone(), empty_memo_value(field));
                    if text.is_empty() {
                        memo_updates.remove(&key);
                    } else {
                        memo_updates.insert(key, MemoUpdate::Text(text));
                    }
                } else {
                    let memo_format = self
                        .memo
                        .as_ref()
                        .map(|memo| memo.format)
                        .expect("memo presence checked above");
                    let update = sidecar_update(&values[&field.name], field, memo_format)?;
                    storage_values.insert(field.name.clone(), empty_memo_value(field));
                    if let Some(update) = update {
                        memo_updates.insert(key, update);
                    } else {
                        memo_updates.remove(&key);
                    }
                }
            }
        }
        Ok((storage_values, memo_updates))
    }

    pub(super) fn apply_memo_updates(
        &mut self,
        path: &Path,
    ) -> Result<Option<MemoSnapshot>, DbfError> {
        if self.memo_updates.is_empty() {
            return Ok(None);
        }
        if find_memo_path(path).is_none() {
            return Err(DbfError::Invalid(
                "memo sidecar is missing for pending memo updates".into(),
            ));
        }
        let mut memo = self
            .memo
            .clone()
            .ok_or_else(|| DbfError::Invalid("memo sidecar is not loaded".into()))?;

        for ((index, field_name), value) in self.memo_updates.clone() {
            let Some(record) = self.records.get(index) else {
                return Err(DbfError::Invalid(
                    "memo update record is out of range".into(),
                ));
            };
            if record.deleted {
                continue;
            }
            let field = self
                .fields
                .iter()
                .find(|field| field.name == field_name && is_sidecar_field(field))
                .cloned()
                .ok_or_else(|| {
                    DbfError::Invalid(format!("sidecar field {field_name} not found"))
                })?;
            let pointer = match value {
                MemoUpdate::Text(value) => {
                    if value.is_empty() {
                        encode_memo_pointer(&field, 0, memo.format)?
                    } else {
                        let bytes = encode_character_with_encoding(
                            &Value::String(value),
                            &field,
                            self.header.language_driver,
                            self.encoding_override.as_deref(),
                        )?;
                        let block = memo.append_text(&bytes)?;
                        encode_memo_pointer(&field, block, memo.format)?
                    }
                }
                MemoUpdate::Binary(bytes) => {
                    if bytes.is_empty() {
                        encode_memo_pointer(&field, 0, memo.format)?
                    } else {
                        let block = memo.append_binary(&bytes)?;
                        encode_memo_pointer(&field, block, memo.format)?
                    }
                }
            };
            let record_offset = self.record_offset(index)?;
            let start = record_offset
                .checked_add(field.offset)
                .ok_or_else(|| DbfError::Invalid("memo pointer offset overflows usize".into()))?;
            let end = start
                .checked_add(usize::from(field.length))
                .ok_or_else(|| DbfError::Invalid("memo pointer end overflows usize".into()))?;
            let target = self
                .bytes
                .get_mut(start..end)
                .ok_or_else(|| DbfError::Invalid("memo pointer area is truncated".into()))?;
            target.copy_from_slice(&pointer);
            self.stored_values[index].insert(
                field.name,
                decode_field_with_encoding(
                    field.field_type,
                    target,
                    self.header.language_driver,
                    Some(memo.format),
                    self.encoding_override.as_deref(),
                ),
            );
        }

        self.memo = Some(memo.clone());
        self.memo_updates.clear();
        Ok(Some(MemoSnapshot {
            format: memo.format,
            bytes: memo.bytes,
        }))
    }
}
