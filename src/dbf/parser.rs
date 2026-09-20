use super::persistence::read_transaction_state;
use super::*;

impl DbfTable {
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, DbfError> {
        Self::from_path_with_encoding(path, None)
    }

    pub fn from_path_with_encoding(
        path: impl AsRef<Path>,
        encoding: Option<&str>,
    ) -> Result<Self, DbfError> {
        let path = path.as_ref();
        let encoding = normalize_encoding_override(encoding)?;
        let _lock = TableLock::acquire(path)?;
        let recovered = Self::recover_wal_with_encoding(path, encoding.as_deref())?;
        let export_recovered = super::schema_export::recover_schema_export_locked(path)?;
        let table = Self::load_path_with_encoding(path, encoding.as_deref())?;
        if recovered || export_recovered {
            let _ = crate::index::refresh_if_present(path, &table);
        }
        Ok(table)
    }

    pub(super) fn has_sidecar_fields(&self) -> bool {
        self.fields.iter().any(is_sidecar_field)
    }

    pub(super) fn resolve_memos(&mut self, memo: &MemoFile) -> Result<(), DbfError> {
        let fields = self
            .fields
            .iter()
            .filter(|field| is_sidecar_field(field))
            .cloned()
            .collect::<Vec<_>>();
        for index in 0..self.records.len() {
            if self.records[index].deleted {
                continue;
            }
            let record_offset = self.record_offset(index)?;
            for field in &fields {
                if self.records[index]
                    .values
                    .get(&field.name)
                    .is_some_and(Value::is_null)
                {
                    continue;
                }
                let start = record_offset + field.offset;
                let end = start + usize::from(field.length);
                let Some(block) = memo_index(&self.bytes[start..end], memo.format)? else {
                    continue;
                };
                let Some(data) = memo.read(block)? else {
                    continue;
                };
                let value = if field.is_binary() {
                    Value::String(hex(&data))
                } else {
                    Value::String(text_with_encoding(
                        &data,
                        self.header.language_driver,
                        self.encoding_override.as_deref(),
                    ))
                };
                self.records[index].values.insert(field.name.clone(), value);
            }
        }
        Ok(())
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, DbfError> {
        Self::from_bytes_with_encoding(bytes, None)
    }

    pub(super) fn from_bytes_with_encoding(
        bytes: &[u8],
        encoding_override: Option<&str>,
    ) -> Result<Self, DbfError> {
        if bytes.len() < CLASSIC_HEADER_SIZE {
            return Err(DbfError::Invalid("header is truncated".into()));
        }

        let header = DbfHeader {
            version: bytes[0],
            last_update: [bytes[1], bytes[2], bytes[3]],
            record_count: read_u32(bytes, 4)?,
            header_length: read_u16(bytes, 8)?,
            record_length: read_u16(bytes, 10)?,
            language_driver: bytes[29],
        };
        let header_length = usize::from(header.header_length);
        let record_length = usize::from(header.record_length);

        if header_length < CLASSIC_HEADER_SIZE + 1 || header_length > bytes.len() {
            return Err(DbfError::Invalid(format!(
                "header length {} is outside the file",
                header.header_length
            )));
        }
        if record_length == 0 {
            return Err(DbfError::Invalid("record length is zero".into()));
        }

        let (descriptor_start, descriptor_size) = if header.version & 0x07 == 4 {
            (LEVEL7_HEADER_SIZE, LEVEL7_DESCRIPTOR_SIZE)
        } else {
            (CLASSIC_HEADER_SIZE, CLASSIC_DESCRIPTOR_SIZE)
        };
        if descriptor_start >= header_length {
            return Err(DbfError::Invalid("field descriptor area is missing".into()));
        }

        let terminator = (descriptor_start..header_length)
            .step_by(descriptor_size)
            .find(|offset| bytes[*offset] == FIELD_TERMINATOR)
            .ok_or_else(|| DbfError::Invalid("field descriptor terminator is missing".into()))?;

        let fields = parse_fields(bytes, descriptor_start, terminator, descriptor_size)?;
        let flag_layout = null_flag_layout(&fields);
        let system_field = system_field_index(&fields);
        let foxpro_table = matches!(header.version, 0x30..=0x32);
        let variable_fields = header.version == 0x32;
        let memo_format = memo_format_for_version(header.version);
        let expected_record_length = fields
            .iter()
            .map(|field| field.length as usize)
            .sum::<usize>()
            .checked_add(1)
            .ok_or_else(|| DbfError::Invalid("record length overflows usize".into()))?;
        if expected_record_length != record_length {
            return Err(DbfError::Invalid(format!(
                "record length {} does not match fields ({expected_record_length})",
                header.record_length
            )));
        }

        let record_start = header_length;
        let record_bytes = usize::try_from(header.record_count)
            .ok()
            .and_then(|count| count.checked_mul(record_length))
            .ok_or_else(|| DbfError::Invalid("record area overflows usize".into()))?;
        let record_end = record_start
            .checked_add(record_bytes)
            .ok_or_else(|| DbfError::Invalid("record area overflows usize".into()))?;
        if record_end > bytes.len() {
            return Err(DbfError::Invalid("record area is truncated".into()));
        }

        let record_count = usize::try_from(header.record_count)
            .map_err(|_| DbfError::Invalid("record count overflows usize".into()))?;
        let mut records = Vec::with_capacity(record_count);
        let mut stored_values = Vec::with_capacity(record_count);
        for number in 1..=record_count {
            let start = record_start + (number - 1) * record_length;
            let record = &bytes[start..start + record_length];
            let deleted = match record[0] {
                ACTIVE_RECORD => false,
                DELETED_RECORD => true,
                marker => {
                    return Err(DbfError::Invalid(format!(
                        "record {number} has unknown deletion marker 0x{marker:02x}"
                    )));
                }
            };

            let null_flags = system_field.and_then(|index| {
                let field = &fields[index];
                record.get(field.offset..field.offset + usize::from(field.length))
            });
            let mut values = Map::new();
            for (field_index, field) in fields.iter().enumerate() {
                if field.is_system() {
                    continue;
                }
                let start = field.offset;
                let end = start + field.length as usize;
                let is_null = foxpro_table
                    && flag_layout[field_index]
                        .and_then(|bits| bits.nullable)
                        .is_some_and(|bit| null_flags.is_some_and(|flags| flag_is_set(flags, bit)));
                let value = if is_null {
                    Value::Null
                } else if variable_fields && field.is_variable() {
                    decode_record_field_with_encoding(
                        field,
                        &record[start..end],
                        header.language_driver,
                        null_flags,
                        flag_layout[field_index],
                        encoding_override,
                    )
                } else if field.field_type.eq_ignore_ascii_case(&b'C') && field.is_binary() {
                    Value::String(hex(&record[start..end]))
                } else {
                    decode_field_with_encoding(
                        field.field_type,
                        &record[start..end],
                        header.language_driver,
                        memo_format,
                        encoding_override,
                    )
                };
                values.insert(field.name.clone(), value);
            }
            stored_values.push(values.clone());
            records.push(DbfRecord {
                number,
                deleted,
                values,
            });
        }

        Ok(Self {
            header,
            fields,
            records,
            stored_values,
            bytes: bytes.to_vec(),
            memo: None,
            schema: None,
            encoding_override: encoding_override.map(ToOwned::to_owned),
            memo_updates: BTreeMap::new(),
            transaction_id: None,
            source: None,
        })
    }
}

fn normalize_encoding_override(encoding: Option<&str>) -> Result<Option<String>, DbfError> {
    encoding
        .map(|name| {
            canonical_encoding_name(name)
                .map(str::to_owned)
                .ok_or_else(|| DbfError::Invalid(format!("unsupported encoding override: {name}")))
        })
        .transpose()
}
