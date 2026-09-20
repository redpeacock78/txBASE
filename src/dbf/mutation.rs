use super::*;

impl DbfTable {
    pub fn insert_record(&mut self, values: Map<String, Value>) -> Result<usize, DbfError> {
        let mut values = values;
        if let Some(schema) = &self.schema {
            schema.apply_defaults(&mut values);
        }
        let mut values = self.normalize_values(&values)?;
        let mut auto_increment_updates = Vec::new();
        for (field_index, field) in self.fields.iter().enumerate() {
            if !field.is_auto_increment(self.header.version) {
                continue;
            }
            if !values[&field.name].is_null() {
                return Err(DbfError::Invalid(format!(
                    "auto-increment field {} is read-only",
                    field.name
                )));
            }
            let Some((next, following, descriptor_offset, signed)) =
                self.next_auto_increment(field_index, field)?
            else {
                continue;
            };
            values.insert(field.name.clone(), Value::Number(next.into()));
            let following = if signed {
                i32::try_from(following)
                    .map_err(|_| {
                        DbfError::Invalid(format!(
                            "auto-increment field {} is exhausted",
                            field.name
                        ))
                    })?
                    .to_le_bytes()
            } else {
                u32::try_from(following)
                    .map_err(|_| {
                        DbfError::Invalid(format!(
                            "auto-increment field {} is exhausted",
                            field.name
                        ))
                    })?
                    .to_le_bytes()
            };
            auto_increment_updates.push((descriptor_offset, following));
        }
        if let Some(schema) = &self.schema {
            schema.validate_candidate(&values, &self.records, None)?;
        }
        let number = self
            .records
            .len()
            .checked_add(1)
            .ok_or_else(|| DbfError::Invalid("record count overflows usize".into()))?;
        let new_count = u32::try_from(number)
            .map_err(|_| DbfError::Invalid("record count exceeds DBF limit".into()))?;
        let mut storage_values = values.clone();
        let mut memo_updates = BTreeMap::new();
        if let Some(memo_format) = self.memo.as_ref().map(|memo| memo.format) {
            for field in self.fields.iter().filter(|field| is_sidecar_field(field)) {
                let value = values.get(&field.name).unwrap_or(&Value::Null);
                storage_values.insert(field.name.clone(), empty_memo_value(field));
                if let Some(update) = sidecar_update(value, field, memo_format)? {
                    memo_updates.insert((number - 1, field.name.clone()), update);
                }
            }
        } else {
            for field in self.fields.iter().filter(|field| is_sidecar_field(field)) {
                let value = values.get(&field.name).unwrap_or(&Value::Null);
                storage_values.insert(
                    field.name.clone(),
                    storage_value_without_sidecar(value, field)?,
                );
            }
        }
        let encoded = self.encode_record(&storage_values)?;
        let record_end = self.record_end()?;
        if self.bytes.len() < record_end {
            return Err(DbfError::Invalid(
                "record area is shorter than parsed data".into(),
            ));
        }
        let suffix = &self.bytes[record_end..];
        if !suffix.is_empty() && suffix != [EOF_MARKER] {
            return Err(DbfError::Invalid(
                "cannot mutate a DBF with trailing sidecar data".into(),
            ));
        }

        let mut bytes = self.bytes[..record_end].to_vec();
        bytes.extend_from_slice(&encoded);
        bytes.push(EOF_MARKER);
        write_record_count(&mut bytes, new_count)?;
        for (descriptor_offset, next) in auto_increment_updates {
            let target = bytes
                .get_mut(descriptor_offset..descriptor_offset + 4)
                .ok_or_else(|| {
                    DbfError::Invalid("auto-increment descriptor is truncated".into())
                })?;
            target.copy_from_slice(&next);
        }

        self.bytes = bytes;
        self.header.record_count = new_count;
        self.records.push(DbfRecord {
            number,
            deleted: false,
            values: values.clone(),
        });
        self.stored_values.push(storage_values);
        self.memo_updates.extend(memo_updates);
        Ok(number)
    }

    pub fn replace_record(
        &mut self,
        number: usize,
        values: Map<String, Value>,
    ) -> Result<(), DbfError> {
        let index = self.active_index(number)?;
        let changed_fields = values.keys().cloned().collect::<BTreeSet<_>>();
        let mut values = self.normalize_values(&values)?;
        self.preserve_auto_increment_fields(index, &mut values, &changed_fields)?;
        if let Some(schema) = &self.schema {
            schema.validate_candidate(&values, &self.records, Some(index))?;
        }
        let (storage_values, memo_updates) = self.prepare_existing_storage(index, &values, None)?;
        self.write_existing_record(index, &values, &storage_values)?;
        self.memo_updates = memo_updates;
        Ok(())
    }

    pub fn patch_record(
        &mut self,
        number: usize,
        patch: Map<String, Value>,
    ) -> Result<(), DbfError> {
        let index = self.active_index(number)?;
        let (values, changed_fields) = expand_update(&self.records[index].values, patch)?;
        let mut values = self.normalize_values(&values)?;
        self.preserve_auto_increment_fields(index, &mut values, &changed_fields)?;
        if let Some(schema) = &self.schema {
            schema.validate_candidate(&values, &self.records, Some(index))?;
        }
        let (storage_values, memo_updates) =
            self.prepare_existing_storage(index, &values, Some(&changed_fields))?;
        self.write_existing_record(index, &values, &storage_values)?;
        self.memo_updates = memo_updates;
        Ok(())
    }

    pub fn delete_record(&mut self, number: usize) -> Result<(), DbfError> {
        let index = self.active_index(number)?;
        let offset = self.record_offset(index)?;
        let mut bytes = self.bytes.clone();
        let marker = bytes
            .get_mut(offset)
            .ok_or_else(|| DbfError::Invalid("record area is truncated".into()))?;
        *marker = DELETED_RECORD;
        self.bytes = bytes;
        self.records[index].deleted = true;
        Ok(())
    }

    fn normalize_values(
        &self,
        values: &Map<String, Value>,
    ) -> Result<Map<String, Value>, DbfError> {
        for field in values.keys() {
            if !self
                .fields
                .iter()
                .any(|descriptor| !descriptor.is_system() && descriptor.name == *field)
            {
                return Err(DbfError::Invalid(format!("unknown field {field}")));
            }
        }
        Ok(self
            .fields
            .iter()
            .filter(|field| !field.is_system())
            .map(|field| {
                (
                    field.name.clone(),
                    values.get(&field.name).cloned().unwrap_or(Value::Null),
                )
            })
            .collect())
    }

    fn encode_record(&self, storage_values: &Map<String, Value>) -> Result<Vec<u8>, DbfError> {
        let mut record = Vec::with_capacity(usize::from(self.header.record_length));
        record.push(ACTIVE_RECORD);
        let null_flags = encode_null_flags(&self.fields, storage_values)?;
        for field in &self.fields {
            if field.is_system() {
                let flags = null_flags.as_deref().ok_or_else(|| {
                    DbfError::Invalid("system field requires _NullFlags bytes".into())
                })?;
                if flags.len() != usize::from(field.length) {
                    return Err(DbfError::Invalid(
                        "_NullFlags field length is inconsistent".into(),
                    ));
                }
                record.extend_from_slice(flags);
            } else {
                record.extend(encode_field_with_encoding(
                    field,
                    storage_values.get(&field.name).unwrap_or(&Value::Null),
                    self.header.language_driver,
                    self.encoding_override.as_deref(),
                )?);
            }
        }
        if record.len() != usize::from(self.header.record_length) {
            return Err(DbfError::Invalid(
                "encoded record length is incorrect".into(),
            ));
        }
        Ok(record)
    }

    fn active_index(&self, number: usize) -> Result<usize, DbfError> {
        let index = number
            .checked_sub(1)
            .ok_or_else(|| DbfError::Invalid("record id must be a positive integer".into()))?;
        match self.records.get(index) {
            Some(record) if !record.deleted => Ok(index),
            _ => Err(DbfError::Invalid("record not found".into())),
        }
    }

    fn record_end(&self) -> Result<usize, DbfError> {
        let record_bytes = self
            .records
            .len()
            .checked_mul(usize::from(self.header.record_length))
            .ok_or_else(|| DbfError::Invalid("record area overflows usize".into()))?;
        usize::from(self.header.header_length)
            .checked_add(record_bytes)
            .ok_or_else(|| DbfError::Invalid("record area overflows usize".into()))
    }

    pub(super) fn record_offset(&self, index: usize) -> Result<usize, DbfError> {
        let offset = index
            .checked_mul(usize::from(self.header.record_length))
            .and_then(|offset| offset.checked_add(usize::from(self.header.header_length)))
            .ok_or_else(|| DbfError::Invalid("record offset overflows usize".into()))?;
        let end = offset
            .checked_add(usize::from(self.header.record_length))
            .ok_or_else(|| DbfError::Invalid("record offset overflows usize".into()))?;
        if end > self.bytes.len() {
            return Err(DbfError::Invalid("record area is truncated".into()));
        }
        Ok(offset)
    }

    fn write_existing_record(
        &mut self,
        index: usize,
        values: &Map<String, Value>,
        storage_values: &Map<String, Value>,
    ) -> Result<(), DbfError> {
        let offset = self.record_offset(index)?;
        let mut bytes = self.bytes.clone();
        let mut changed_fields = BTreeSet::new();
        for field in &self.fields {
            if field.is_system() {
                continue;
            }
            if values.get(&field.name) == self.records[index].values.get(&field.name) {
                continue;
            }
            changed_fields.insert(field.name.clone());
            let encoded = encode_field_with_encoding(
                field,
                storage_values.get(&field.name).unwrap_or(&Value::Null),
                self.header.language_driver,
                self.encoding_override.as_deref(),
            )?;
            let start = offset
                .checked_add(field.offset)
                .ok_or_else(|| DbfError::Invalid("field offset overflows usize".into()))?;
            let end = start
                .checked_add(usize::from(field.length))
                .ok_or_else(|| DbfError::Invalid("field range overflows usize".into()))?;
            bytes
                .get_mut(start..end)
                .ok_or_else(|| DbfError::Invalid("stored field is truncated".into()))?
                .copy_from_slice(&encoded);
        }
        update_null_flags(&mut bytes, offset, &self.fields, values, &changed_fields)?;
        self.bytes = bytes;
        self.records[index].values = values.clone();
        self.stored_values[index] = storage_values.clone();
        Ok(())
    }
}

fn expand_update(
    current: &Map<String, Value>,
    update: Map<String, Value>,
) -> Result<(Map<String, Value>, BTreeSet<String>), DbfError> {
    let has_operator = update.keys().any(|key| key.starts_with('$'));
    if !has_operator {
        let changed_fields = update.keys().cloned().collect();
        let mut values = current.clone();
        values.extend(update);
        return Ok((values, changed_fields));
    }
    if update.keys().any(|key| !key.starts_with('$')) {
        return Err(DbfError::Invalid(
            "update cannot mix operators and fields".into(),
        ));
    }

    let mut values = current.clone();
    let mut changed_fields = BTreeSet::new();
    for (operator, operand) in update {
        match operator.as_str() {
            "$set" | "$unset" | "$inc" => {}
            _ => {
                return Err(DbfError::Invalid(format!(
                    "unsupported update operator {operator}"
                )));
            }
        }
        let fields = operand
            .as_object()
            .ok_or_else(|| DbfError::Invalid(format!("{operator} requires an object")))?;
        for field in fields.keys() {
            if !changed_fields.insert(field.clone()) {
                return Err(DbfError::Invalid(format!(
                    "field {field} appears in multiple update operators"
                )));
            }
        }
        match operator.as_str() {
            "$set" => values.extend(fields.clone()),
            "$unset" => {
                for field in fields.keys() {
                    values.insert(field.clone(), Value::Null);
                }
            }
            "$inc" => {
                for (field, increment) in fields {
                    let value = increment_value(values.get(field), increment, field)?;
                    values.insert(field.clone(), value);
                }
            }
            _ => unreachable!("update operator was validated above"),
        }
    }
    Ok((values, changed_fields))
}

fn increment_value(
    current: Option<&Value>,
    increment: &Value,
    field: &str,
) -> Result<Value, DbfError> {
    let Some(Value::Number(current)) = current else {
        return Err(DbfError::Invalid(format!(
            "$inc requires a numeric value in field {field}"
        )));
    };
    let Value::Number(increment) = increment else {
        return Err(DbfError::Invalid(format!(
            "$inc value for {field} must be a JSON number"
        )));
    };
    if let (Some(current), Some(increment)) = (current.as_i64(), increment.as_i64()) {
        let value = current
            .checked_add(increment)
            .ok_or_else(|| DbfError::Invalid(format!("$inc overflows integer field {field}")))?;
        return Ok(Value::Number(value.into()));
    }
    let value = current
        .as_f64()
        .and_then(|current| increment.as_f64().map(|increment| current + increment))
        .and_then(Number::from_f64)
        .ok_or_else(|| DbfError::Invalid(format!("$inc result for {field} is not finite")))?;
    Ok(Value::Number(value))
}
