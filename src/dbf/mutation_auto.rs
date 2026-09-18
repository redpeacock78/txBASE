use super::*;

impl DbfTable {
    pub(super) fn next_auto_increment(
        &self,
        field_index: usize,
        field: &FieldDescriptor,
    ) -> Result<Option<(i64, i64, usize, bool)>, DbfError> {
        if self.header.version & 0x07 == 4 {
            let descriptor_offset =
                LEVEL7_HEADER_SIZE
                    .checked_add(field_index.checked_mul(LEVEL7_DESCRIPTOR_SIZE).ok_or_else(
                        || DbfError::Invalid("field descriptor offset overflows".into()),
                    )?)
                    .ok_or_else(|| DbfError::Invalid("field descriptor offset overflows".into()))?;
            let next_offset = descriptor_offset
                .checked_add(40)
                .ok_or_else(|| DbfError::Invalid("auto-increment offset overflows".into()))?;
            let next_end = next_offset
                .checked_add(4)
                .ok_or_else(|| DbfError::Invalid("auto-increment offset overflows".into()))?;
            if next_end > usize::from(self.header.header_length) {
                return Err(DbfError::Invalid(
                    "auto-increment descriptor is outside the header".into(),
                ));
            }
            let next = read_u32(&self.bytes, next_offset)?;
            return Ok(Some((
                i64::from(next),
                i64::from(next) + 1,
                next_offset,
                false,
            )));
        }

        if self.header.version != 0x31 {
            return Ok(None);
        }
        if field.length != 4 || !field.field_type.eq_ignore_ascii_case(&b'I') {
            return Err(DbfError::Invalid(format!(
                "auto-increment field {} must be a four-byte integer",
                field.name
            )));
        }
        let descriptor_offset = CLASSIC_HEADER_SIZE
            .checked_add(
                field_index
                    .checked_mul(CLASSIC_DESCRIPTOR_SIZE)
                    .ok_or_else(|| DbfError::Invalid("field descriptor offset overflows".into()))?,
            )
            .ok_or_else(|| DbfError::Invalid("field descriptor offset overflows".into()))?;
        let next_offset = descriptor_offset
            .checked_add(19)
            .ok_or_else(|| DbfError::Invalid("auto-increment offset overflows".into()))?;
        let step_offset = descriptor_offset
            .checked_add(23)
            .ok_or_else(|| DbfError::Invalid("auto-increment offset overflows".into()))?;
        let end = step_offset
            .checked_add(1)
            .ok_or_else(|| DbfError::Invalid("auto-increment offset overflows".into()))?;
        if end > usize::from(self.header.header_length) {
            return Err(DbfError::Invalid(
                "auto-increment descriptor is outside the header".into(),
            ));
        }
        let next = i64::from(i32::from_le_bytes(
            read_u32(&self.bytes, next_offset)?.to_le_bytes(),
        ));
        let step = self.bytes[step_offset];
        if step == 0 {
            return Err(DbfError::Invalid(format!(
                "auto-increment field {} has a zero step",
                field.name
            )));
        }
        let following = next.checked_add(i64::from(step)).ok_or_else(|| {
            DbfError::Invalid(format!("auto-increment field {} is exhausted", field.name))
        })?;
        Ok(Some((following, following, next_offset, true)))
    }

    pub(super) fn preserve_auto_increment_fields(
        &self,
        index: usize,
        values: &mut Map<String, Value>,
        changed_fields: &BTreeSet<String>,
    ) -> Result<(), DbfError> {
        for field in self
            .fields
            .iter()
            .filter(|field| field.is_auto_increment(self.header.version))
        {
            let current = &self.records[index].values[&field.name];
            if changed_fields.contains(&field.name) && values[&field.name] != *current {
                return Err(DbfError::Invalid(format!(
                    "auto-increment field {} is read-only",
                    field.name
                )));
            }
            values.insert(field.name.clone(), current.clone());
        }
        Ok(())
    }
}
