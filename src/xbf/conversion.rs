use super::{XbfError, XbfField, XbfRecord, XbfTable, XbfType, XbfValue};
use crate::dbf::{DbfTable, FieldDescriptor};
use serde_json::Value;

pub fn from_dbf(table: &DbfTable) -> Result<XbfTable, XbfError> {
    let constraints = table.xbf_field_constraints().map_err(|error| {
        XbfError::Invalid(format!(
            "DBF schema metadata cannot be represented in XBF: {error}"
        ))
    })?;
    let source_fields = table
        .fields
        .iter()
        .filter(|field| !field.is_system())
        .cloned()
        .collect::<Vec<_>>();
    let mut fields = Vec::with_capacity(source_fields.len());
    let mut types = Vec::with_capacity(source_fields.len());
    for field in &source_fields {
        if field.name.len() > 255 {
            return Err(invalid(field, "field name exceeds 255 UTF-8 bytes"));
        }
        let values = table
            .records()
            .iter()
            .map(|record| record.values.get(&field.name).unwrap_or(&Value::Null))
            .collect::<Vec<_>>();
        let ty = infer_type(field, &values)?;
        let (primary_key, unique, not_null) =
            constraints.get(&field.name).copied().unwrap_or_default();
        let nullable = !primary_key && !not_null && values.iter().any(|value| value.is_null());
        fields.push(XbfField {
            name: field.name.clone(),
            ty,
            nullable,
            primary_key,
            unique: primary_key || unique,
        });
        types.push(ty);
    }

    let mut records = Vec::with_capacity(table.records().len());
    for record in table.records() {
        let mut values = Vec::with_capacity(source_fields.len());
        for (field, ty) in source_fields.iter().zip(&types) {
            let value = record
                .values
                .get(&field.name)
                .ok_or_else(|| invalid(field, "record is missing the field"))?;
            values.push(convert_value(field, *ty, value)?);
        }
        records.push(XbfRecord {
            deleted: record.deleted,
            values,
        });
    }

    Ok(XbfTable {
        generation: 0,
        fields,
        records,
    })
}

fn infer_type(field: &FieldDescriptor, values: &[&Value]) -> Result<XbfType, XbfError> {
    let field_type = field.field_type.to_ascii_uppercase();
    match field_type {
        b'C' | b'M' | b'Q' | b'V' => Ok(if field.is_binary() {
            XbfType::Bytes
        } else {
            XbfType::String
        }),
        b'L' => Ok(XbfType::Boolean),
        b'I' | b'+' => Ok(XbfType::Signed32),
        b'O' => Ok(XbfType::Float64),
        b'B' if field.length == 8 => Ok(XbfType::Float64),
        b'B' => Ok(XbfType::Unsigned64),
        b'G' | b'P' | b'W' => {
            if field.is_binary()
                && values
                    .iter()
                    .all(|value| value.is_null() || value.as_str().is_some_and(is_hex))
            {
                Ok(XbfType::Bytes)
            } else {
                Ok(XbfType::Unsigned64)
            }
        }
        b'D' => Ok(XbfType::Date),
        b'T' => Ok(XbfType::Timestamp),
        b'@' => Ok(XbfType::Bytes),
        b'Y' => Ok(XbfType::String),
        b'N' | b'F' if field.decimal_count == 0 => {
            if values
                .iter()
                .all(|value| value.is_null() || value.as_i64().is_some())
            {
                Ok(XbfType::Signed64)
            } else {
                Ok(XbfType::Float64)
            }
        }
        b'N' | b'F' => Ok(XbfType::Float64),
        other => Err(invalid(
            field,
            &format!("field type 0x{other:02x} is not convertible to XBF"),
        )),
    }
}

fn convert_value(
    field: &FieldDescriptor,
    ty: XbfType,
    value: &Value,
) -> Result<XbfValue, XbfError> {
    if value.is_null() {
        return Ok(XbfValue::Null);
    }
    match ty {
        XbfType::Boolean => value
            .as_bool()
            .map(XbfValue::Boolean)
            .ok_or_else(|| invalid(field, "value is not boolean")),
        XbfType::Signed32 => value
            .as_i64()
            .and_then(|value| i32::try_from(value).ok())
            .map(XbfValue::Signed32)
            .ok_or_else(|| invalid(field, "value is not a signed 32-bit integer")),
        XbfType::Signed64 => value
            .as_i64()
            .map(XbfValue::Signed64)
            .ok_or_else(|| invalid(field, "value is not a signed 64-bit integer")),
        XbfType::Unsigned64 => value
            .as_u64()
            .map(XbfValue::Unsigned64)
            .ok_or_else(|| invalid(field, "value is not an unsigned 64-bit integer")),
        XbfType::Float64 => value
            .as_f64()
            .filter(|value| value.is_finite())
            .map(XbfValue::Float64)
            .ok_or_else(|| invalid(field, "value is not a finite number")),
        XbfType::String => value
            .as_str()
            .map(|value| XbfValue::String(value.to_owned()))
            .ok_or_else(|| invalid(field, "value is not text")),
        XbfType::Bytes => value
            .as_str()
            .and_then(decode_hex)
            .map(XbfValue::Bytes)
            .ok_or_else(|| invalid(field, "value is not an even-length hexadecimal string")),
        XbfType::Date => parse_date(field, value),
        XbfType::Timestamp => parse_timestamp(field, value),
        XbfType::Uuid | XbfType::Json | XbfType::Float32 => Err(invalid(
            field,
            "value type is not produced by the DBF converter",
        )),
    }
}

fn parse_date(field: &FieldDescriptor, value: &Value) -> Result<XbfValue, XbfError> {
    let text = value
        .as_str()
        .ok_or_else(|| invalid(field, "date value is not YYYYMMDD text"))?;
    if text.len() != 8 || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(invalid(field, "date value is not YYYYMMDD text"));
    }
    let year =
        parse_component(&text[0..4]).ok_or_else(|| invalid(field, "date year is invalid"))?;
    let month =
        parse_component(&text[4..6]).ok_or_else(|| invalid(field, "date month is invalid"))?;
    let day = parse_component(&text[6..8]).ok_or_else(|| invalid(field, "date day is invalid"))?;
    let days = checked_civil_days(field, year, month, day)?;
    let days = i32::try_from(days).map_err(|_| invalid(field, "date is outside XBF range"))?;
    Ok(XbfValue::Date(days))
}

fn parse_timestamp(field: &FieldDescriptor, value: &Value) -> Result<XbfValue, XbfError> {
    let text = value
        .as_str()
        .ok_or_else(|| invalid(field, "timestamp value is not ISO text"))?;
    let bytes = text.as_bytes();
    if bytes.len() != 19
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
    {
        return Err(invalid(field, "timestamp value is not YYYY-MM-DDTHH:MM:SS"));
    }
    let year =
        parse_component(&text[0..4]).ok_or_else(|| invalid(field, "timestamp year is invalid"))?;
    let month =
        parse_component(&text[5..7]).ok_or_else(|| invalid(field, "timestamp month is invalid"))?;
    let day =
        parse_component(&text[8..10]).ok_or_else(|| invalid(field, "timestamp day is invalid"))?;
    let hour = parse_component(&text[11..13])
        .ok_or_else(|| invalid(field, "timestamp hour is invalid"))?;
    let minute = parse_component(&text[14..16])
        .ok_or_else(|| invalid(field, "timestamp minute is invalid"))?;
    let second = parse_component(&text[17..19])
        .ok_or_else(|| invalid(field, "timestamp second is invalid"))?;
    if !(0..=23).contains(&hour) || !(0..=59).contains(&minute) || !(0..=59).contains(&second) {
        return Err(invalid(
            field,
            "timestamp time is outside the supported range",
        ));
    }
    let days = checked_civil_days(field, year, month, day)?;
    let milliseconds = days
        .checked_mul(86_400_000)
        .and_then(|value| value.checked_add(hour * 3_600_000))
        .and_then(|value| value.checked_add(minute * 60_000))
        .and_then(|value| value.checked_add(second * 1_000))
        .ok_or_else(|| invalid(field, "timestamp overflows XBF range"))?;
    Ok(XbfValue::Timestamp(milliseconds))
}

fn checked_civil_days(
    field: &FieldDescriptor,
    year: i64,
    month: i64,
    day: i64,
) -> Result<i64, XbfError> {
    if !(1..=9999).contains(&year) {
        return Err(invalid(field, "date year is outside the supported range"));
    }
    let days = days_from_civil(year, month, day);
    let (checked_year, checked_month, checked_day) = civil_from_days(days);
    if (year, month, day) != (checked_year, checked_month, checked_day) {
        return Err(invalid(field, "date contains an invalid calendar day"));
    }
    Ok(days)
}

fn parse_component(value: &str) -> Option<i64> {
    if !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    value.parse().ok()
}

fn decode_hex(value: &str) -> Option<Vec<u8>> {
    if !is_hex(value) {
        return None;
    }
    let mut bytes = Vec::with_capacity(value.len() / 2);
    for pair in value.as_bytes().chunks_exact(2) {
        bytes.push((hex_digit(pair[0])? << 4) | hex_digit(pair[1])?);
    }
    Some(bytes)
}

fn is_hex(value: &str) -> bool {
    value.len() % 2 == 0 && value.bytes().all(|byte| hex_digit(byte).is_some())
}

fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn invalid(field: &FieldDescriptor, message: &str) -> XbfError {
    XbfError::Invalid(format!("DBF field {}: {message}", field.name))
}

pub(super) fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let adjusted_year = year - i64::from(month <= 2);
    let era = if adjusted_year >= 0 {
        adjusted_year
    } else {
        adjusted_year - 399
    } / 400;
    let year_of_era = adjusted_year - era * 400;
    let month_prime = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * month_prime + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

pub(super) fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let shifted = days + 719_468;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    } / 146_097;
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    (year + i64::from(month <= 2), month, day)
}
