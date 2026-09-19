use super::{DbfError, DbfTable, encoding_name};
use serde_json::{Value, json};

impl DbfTable {
    pub fn schema_json(&self) -> Value {
        json!({
            "format": "dbf",
            "version": self.header.version,
            "last_update": self.header.last_update,
            "language_driver": self.header.language_driver,
            "encoding": encoding_name(self.header.language_driver),
            "encoding_override": self.encoding_override,
            "schema_metadata": self
                .schema
                .as_ref()
                .map_or(Value::Null, |metadata| metadata.json()),
            "header_length": self.header.header_length,
            "record_length": self.header.record_length,
            "record_count": self.header.record_count,
            "active_record_count": self.active_records().count(),
            "memo_sidecar": self.memo.is_some(),
            "fields": self.fields.iter().map(|field| {
                json!({
                    "name": field.name,
                    "type": String::from_utf8_lossy(&[field.field_type]),
                    "offset": field.offset,
                    "length": field.length,
                    "decimal_count": field.decimal_count,
                    "flags": field.flags,
                    "system": field.is_system(),
                    "nullable": field.is_nullable(),
                    "variable": field.is_variable(),
                    "binary": field.is_binary(),
                    "auto_increment": field.is_auto_increment(self.header.version),
                })
            }).collect::<Vec<_>>(),
        })
    }

    pub fn verify(&self) -> Result<(), DbfError> {
        let parsed = Self::from_bytes(&self.bytes)?;
        if parsed.header != self.header || parsed.fields != self.fields {
            return Err(DbfError::Invalid(
                "serialized DBF metadata differs from the loaded table".into(),
            ));
        }
        let expected_count = usize::try_from(self.header.record_count)
            .map_err(|_| DbfError::Invalid("record count overflows usize".into()))?;
        if self.records.len() != expected_count || self.stored_values.len() != expected_count {
            return Err(DbfError::Invalid(
                "loaded record count differs from the DBF header".into(),
            ));
        }
        for index in 0..self.records.len() {
            self.record_offset(index)?;
        }
        Ok(())
    }
}
