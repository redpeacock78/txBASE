use super::super::conversion::{civil_from_days, days_from_civil};
use super::super::{XbfError, XbfField, XbfTable, XbfType, XbfValue};
use crate::dbf::DbfTable;
use serde_json::{Map, Number, Value, json};
use std::collections::BTreeSet;

const CLASSIC_HEADER_SIZE: usize = 32;
const CLASSIC_DESCRIPTOR_SIZE: usize = 32;
const EOF_MARKER: u8 = 0x1a;
const JULIAN_DAY_UNIX_EPOCH: i64 = 2_440_588;
const MILLISECONDS_PER_DAY: i64 = 86_400_000;

#[derive(Debug, Clone, Copy)]
pub(super) struct ExportField {
    pub(super) field_type: u8,
    pub(super) length: u8,
    pub(super) decimal_count: u8,
    pub(super) flags: u8,
}

pub(super) fn to_dbf_inner(
    table: &XbfTable,
    preserve_constraints: bool,
) -> Result<DbfTable, XbfError> {
    for record in &table.records {
        if record.values.len() != table.fields.len() {
            return Err(XbfError::Invalid(
                "XBF record value count does not match schema".into(),
            ));
        }
    }
    let mut names = BTreeSet::new();
    let mut descriptors = Vec::with_capacity(table.fields.len());
    for (index, field) in table.fields.iter().enumerate() {
        if !names.insert(field.name.clone()) {
            return Err(invalid(field, "field name is duplicated"));
        }
        let values = table
            .records
            .iter()
            .map(|record| &record.values[index])
            .collect::<Vec<_>>();
        descriptors.push(descriptor(field, &values, preserve_constraints)?);
    }

    let mut dbf =
        DbfTable::from_bytes(&empty_dbf(&table.fields, &descriptors)?).map_err(dbf_error)?;
    for record in &table.records {
        let mut values = Map::new();
        for (field, value) in table.fields.iter().zip(&record.values) {
            values.insert(field.name.clone(), value_to_dbf(field, value)?);
        }
        let number = dbf.insert_record(values).map_err(dbf_error)?;
        if record.deleted {
            dbf.delete_record(number).map_err(dbf_error)?;
        }
    }
    Ok(dbf)
}

pub(super) fn descriptor(
    field: &XbfField,
    values: &[&XbfValue],
    preserve_constraints: bool,
) -> Result<ExportField, XbfError> {
    if field.name.is_empty() || field.name.len() > 10 || !field.name.is_ascii() {
        return Err(invalid(
            field,
            "DBF field names must be non-empty ASCII of at most 10 bytes",
        ));
    }
    if !preserve_constraints {
        if field.primary_key || field.unique {
            return Err(invalid(
                field,
                "DBF export cannot preserve primary or unique constraints",
            ));
        }
        if !field.nullable {
            return Err(invalid(
                field,
                "DBF export cannot preserve a not-null constraint",
            ));
        }
    }
    if !field.nullable && values.iter().any(|value| matches!(value, XbfValue::Null)) {
        return Err(invalid(field, "non-nullable field contains NULL"));
    }

    let result = match field.ty {
        XbfType::Boolean => ExportField {
            field_type: b'L',
            length: 1,
            decimal_count: 0,
            flags: 0,
        },
        XbfType::Signed32 | XbfType::Signed64 | XbfType::Unsigned64 => ExportField {
            field_type: b'N',
            length: 20,
            decimal_count: 0,
            flags: 0,
        },
        XbfType::Float32 | XbfType::Float64 => ExportField {
            field_type: b'N',
            length: 24,
            decimal_count: 17,
            flags: 0,
        },
        XbfType::String => ExportField {
            field_type: b'C',
            length: variable_width(field, values, false)?,
            decimal_count: 0,
            flags: 0,
        },
        XbfType::Bytes => ExportField {
            field_type: b'C',
            length: variable_width(field, values, true)?,
            decimal_count: 0,
            flags: 0x04,
        },
        XbfType::Date => ExportField {
            field_type: b'D',
            length: 8,
            decimal_count: 0,
            flags: 0,
        },
        XbfType::Timestamp => ExportField {
            field_type: b'T',
            length: 8,
            decimal_count: 0,
            flags: 0,
        },
        XbfType::Uuid | XbfType::Json => {
            return Err(invalid(
                field,
                "DBF export has no lossless representation for this type",
            ));
        }
    };

    if matches!(field.ty, XbfType::String | XbfType::Bytes)
        && values.iter().any(|value| matches!(value, XbfValue::Null))
    {
        return Err(invalid(
            field,
            "DBF character fields cannot preserve NULL separately from empty values",
        ));
    }
    Ok(result)
}

pub(super) fn schema_metadata(table: &XbfTable) -> Value {
    let mut fields = Map::new();
    for field in &table.fields {
        let mut metadata = Map::new();
        if field.primary_key {
            metadata.insert(String::from("primary"), Value::Bool(true));
        } else if field.unique {
            metadata.insert(String::from("unique"), Value::Bool(true));
        }
        if !field.nullable {
            metadata.insert(String::from("not_null"), Value::Bool(true));
        }
        fields.insert(field.name.clone(), Value::Object(metadata));
    }
    json!({
        "format": "txbase-schema",
        "version": 1,
        "fields": fields,
    })
}

fn variable_width(field: &XbfField, values: &[&XbfValue], binary: bool) -> Result<u8, XbfError> {
    let mut width = 1usize;
    for value in values {
        let length = match (binary, *value) {
            (_, XbfValue::Null) => 0,
            (false, XbfValue::String(value)) => {
                if value.as_bytes().contains(&0) {
                    return Err(invalid(field, "text contains an embedded NUL"));
                }
                value.len()
            }
            (true, XbfValue::Bytes(value)) => value.len(),
            _ => return Err(invalid(field, "value type does not match its XBF field")),
        };
        width = width.max(length);
    }
    if width > 254 {
        return Err(invalid(field, "value exceeds the 254-byte DBF field limit"));
    }
    Ok(width as u8)
}

pub(in crate::xbf) fn value_to_dbf(field: &XbfField, value: &XbfValue) -> Result<Value, XbfError> {
    match (field.ty, value) {
        (_, XbfValue::Null) => Ok(Value::Null),
        (XbfType::Boolean, XbfValue::Boolean(value)) => Ok(Value::Bool(*value)),
        (XbfType::Signed32, XbfValue::Signed32(value)) => Ok(Value::Number((*value).into())),
        (XbfType::Signed64, XbfValue::Signed64(value)) => Ok(Value::Number((*value).into())),
        (XbfType::Unsigned64, XbfValue::Unsigned64(value)) => {
            if *value > i64::MAX as u64 {
                return Err(invalid(
                    field,
                    "unsigned integer exceeds the exact DBF numeric range",
                ));
            }
            Ok(Value::Number(Number::from(*value)))
        }
        (XbfType::Float32, XbfValue::Float32(value)) => number(field, f64::from(*value)),
        (XbfType::Float64, XbfValue::Float64(value)) => number(field, *value),
        (XbfType::String, XbfValue::String(value)) => Ok(Value::String(value.clone())),
        (XbfType::Bytes, XbfValue::Bytes(value)) => Ok(Value::String(hex(value))),
        (XbfType::Date, XbfValue::Date(value)) => date_text(field, *value),
        (XbfType::Timestamp, XbfValue::Timestamp(value)) => timestamp_hex(field, *value),
        _ => Err(invalid(field, "value type does not match its XBF field")),
    }
}

fn number(field: &XbfField, value: f64) -> Result<Value, XbfError> {
    if !value.is_finite() {
        return Err(invalid(field, "DBF numeric fields require finite values"));
    }
    Number::from_f64(value)
        .map(Value::Number)
        .ok_or_else(|| invalid(field, "DBF numeric value cannot be encoded as JSON"))
}

fn date_text(field: &XbfField, days: i32) -> Result<Value, XbfError> {
    let (year, month, day) = civil_from_days(i64::from(days));
    if !(1..=9999).contains(&year) || days_from_civil(year, month, day) != i64::from(days) {
        return Err(invalid(field, "date is outside the DBF date range"));
    }
    Ok(Value::String(format!("{year:04}{month:02}{day:02}")))
}

fn timestamp_hex(field: &XbfField, milliseconds: i64) -> Result<Value, XbfError> {
    let days = milliseconds.div_euclid(MILLISECONDS_PER_DAY);
    let day_milliseconds = milliseconds.rem_euclid(MILLISECONDS_PER_DAY);
    let (year, month, day) = civil_from_days(days);
    if !(1..=9999).contains(&year) || days_from_civil(year, month, day) != days {
        return Err(invalid(field, "timestamp is outside the DBF date range"));
    }
    let julian_day = u32::try_from(days + JULIAN_DAY_UNIX_EPOCH)
        .map_err(|_| invalid(field, "timestamp Julian day is out of range"))?;
    let day_milliseconds = u32::try_from(day_milliseconds)
        .map_err(|_| invalid(field, "timestamp time is out of range"))?;
    let mut bytes = [0; 8];
    bytes[..4].copy_from_slice(&julian_day.to_le_bytes());
    bytes[4..].copy_from_slice(&day_milliseconds.to_le_bytes());
    Ok(Value::String(hex(&bytes)))
}

pub(super) fn empty_dbf(
    fields: &[XbfField],
    descriptors: &[ExportField],
) -> Result<Vec<u8>, XbfError> {
    let header_length = CLASSIC_HEADER_SIZE
        .checked_add(
            descriptors
                .len()
                .checked_mul(CLASSIC_DESCRIPTOR_SIZE)
                .ok_or_else(|| XbfError::Invalid("DBF header length overflows".into()))?,
        )
        .and_then(|length| length.checked_add(1))
        .ok_or_else(|| XbfError::Invalid("DBF header length overflows".into()))?;
    let record_length = descriptors
        .iter()
        .try_fold(1usize, |length, descriptor| {
            length.checked_add(usize::from(descriptor.length))
        })
        .ok_or_else(|| XbfError::Invalid("DBF record length overflows".into()))?;
    let header_length = u16::try_from(header_length)
        .map_err(|_| XbfError::Invalid("DBF header length exceeds u16".into()))?;
    let record_length = u16::try_from(record_length)
        .map_err(|_| XbfError::Invalid("DBF record length exceeds u16".into()))?;
    let mut bytes = vec![0; usize::from(header_length)];
    bytes[0] = if descriptors
        .iter()
        .any(|descriptor| descriptor.field_type == b'T')
    {
        0x30
    } else {
        0x03
    };
    bytes[1..4].copy_from_slice(&[0x7e, 0x09, 0x14]);
    bytes[8..10].copy_from_slice(&header_length.to_le_bytes());
    bytes[10..12].copy_from_slice(&record_length.to_le_bytes());
    let mut offset = CLASSIC_HEADER_SIZE;
    for (field, descriptor) in fields.iter().zip(descriptors) {
        let name = field.name.as_bytes();
        bytes[offset..offset + name.len()].copy_from_slice(name);
        bytes[offset + 11] = descriptor.field_type;
        bytes[offset + 16] = descriptor.length;
        bytes[offset + 17] = descriptor.decimal_count;
        bytes[offset + 18] = descriptor.flags;
        offset += CLASSIC_DESCRIPTOR_SIZE;
    }
    bytes[usize::from(header_length) - 1] = 0x0d;
    bytes.push(EOF_MARKER);
    Ok(bytes)
}

fn hex(bytes: &[u8]) -> String {
    let mut value = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write;
        let _ = write!(value, "{byte:02x}");
    }
    value
}

fn invalid(field: &XbfField, message: &str) -> XbfError {
    XbfError::Invalid(format!("XBF field {}: {message}", field.name))
}

pub(super) fn dbf_error(error: crate::dbf::DbfError) -> XbfError {
    XbfError::Invalid(format!("XBF to DBF export failed: {error}"))
}
