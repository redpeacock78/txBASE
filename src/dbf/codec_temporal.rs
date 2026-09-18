use super::super::{
    CURRENCY_SCALE, DbfError, FieldDescriptor, JULIAN_DAY_UNIX_EPOCH, MILLISECONDS_PER_DAY,
};
use super::value_text;
use serde_json::Value;

pub fn foxpro_datetime_bytes(text: &str) -> Result<Option<[u8; 8]>, DbfError> {
    let bytes = text.as_bytes();
    if bytes.len() != 19
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || !matches!(bytes[10], b'T' | b' ')
        || bytes[13] != b':'
        || bytes[16] != b':'
    {
        return Ok(None);
    }
    let parse = |part: &[u8]| {
        if !part.iter().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        std::str::from_utf8(part).ok()?.parse::<i64>().ok()
    };
    let Some(year) = parse(&bytes[0..4]) else {
        return Err(DbfError::Invalid(
            "datetime field requires YYYY-MM-DDTHH:MM:SS".into(),
        ));
    };
    let Some(month) = parse(&bytes[5..7]) else {
        return Err(DbfError::Invalid(
            "datetime field requires YYYY-MM-DDTHH:MM:SS".into(),
        ));
    };
    let Some(day) = parse(&bytes[8..10]) else {
        return Err(DbfError::Invalid(
            "datetime field requires YYYY-MM-DDTHH:MM:SS".into(),
        ));
    };
    let Some(hour) = parse(&bytes[11..13]) else {
        return Err(DbfError::Invalid(
            "datetime field requires YYYY-MM-DDTHH:MM:SS".into(),
        ));
    };
    let Some(minute) = parse(&bytes[14..16]) else {
        return Err(DbfError::Invalid(
            "datetime field requires YYYY-MM-DDTHH:MM:SS".into(),
        ));
    };
    let Some(second) = parse(&bytes[17..19]) else {
        return Err(DbfError::Invalid(
            "datetime field requires YYYY-MM-DDTHH:MM:SS".into(),
        ));
    };
    if !(1..=9999).contains(&year)
        || !(0..=23).contains(&hour)
        || !(0..=59).contains(&minute)
        || !(0..=59).contains(&second)
    {
        return Err(DbfError::Invalid(
            "datetime field is outside the supported range".into(),
        ));
    }
    let days = days_from_civil(year, month, day);
    let (checked_year, checked_month, checked_day) = civil_from_days(days);
    if (year, month, day) != (checked_year, checked_month, checked_day) {
        return Err(DbfError::Invalid(
            "datetime field contains an invalid date".into(),
        ));
    }
    let milliseconds = u32::try_from(hour * 3_600_000 + minute * 60_000 + second * 1_000)
        .expect("datetime components fit in milliseconds");
    let julian_day = u32::try_from(days + JULIAN_DAY_UNIX_EPOCH)
        .map_err(|_| DbfError::Invalid("datetime field date is out of range".into()))?;
    let mut encoded = [0; 8];
    encoded[..4].copy_from_slice(&julian_day.to_le_bytes());
    encoded[4..].copy_from_slice(&milliseconds.to_le_bytes());
    Ok(Some(encoded))
}

pub fn foxpro_datetime_text(bytes: &[u8]) -> Option<String> {
    let julian_day = u32::from_le_bytes(bytes[..4].try_into().ok()?);
    let milliseconds = u32::from_le_bytes(bytes[4..8].try_into().ok()?);
    if julian_day == 0 {
        return None;
    }
    if milliseconds >= MILLISECONDS_PER_DAY {
        return None;
    }
    let (year, month, day) = civil_from_days(i64::from(julian_day) - JULIAN_DAY_UNIX_EPOCH);
    if !(1..=9999).contains(&year) {
        return None;
    }
    let hour = milliseconds / 3_600_000;
    let minute = milliseconds / 60_000 % 60;
    let second = milliseconds / 1_000 % 60;
    Some(format!(
        "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}"
    ))
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

pub fn currency_text(bytes: &[u8]) -> String {
    let value = i64::from_le_bytes(bytes[..8].try_into().unwrap());
    let negative = value < 0;
    let magnitude = value.unsigned_abs();
    let whole = magnitude / CURRENCY_SCALE;
    let fraction = magnitude % CURRENCY_SCALE;
    if negative {
        format!("-{whole}.{fraction:04}")
    } else {
        format!("{whole}.{fraction:04}")
    }
}

pub fn currency_i64(value: &Value, field: &FieldDescriptor) -> Result<i64, DbfError> {
    let text = value_text(value, field)?;
    let text = text.trim();
    if text.is_empty() {
        return Ok(0);
    }

    let (negative, text) = if let Some(rest) = text.strip_prefix('-') {
        (true, rest)
    } else if let Some(rest) = text.strip_prefix('+') {
        (false, rest)
    } else {
        (false, text)
    };
    let mut parts = text.split('.');
    let whole_text = parts.next().unwrap_or_default();
    let fraction_text = parts.next().unwrap_or_default();
    if parts.next().is_some()
        || (whole_text.is_empty() && fraction_text.is_empty())
        || !whole_text.bytes().all(|byte| byte.is_ascii_digit())
        || !fraction_text.bytes().all(|byte| byte.is_ascii_digit())
        || fraction_text.len() > 4
    {
        return Err(DbfError::Invalid(format!(
            "currency field {} requires a fixed-point number with at most four decimals or null",
            field.name
        )));
    }
    let whole = if whole_text.is_empty() {
        0
    } else {
        whole_text.parse::<u64>().map_err(|_| {
            DbfError::Invalid(format!("currency field {} is out of range", field.name))
        })?
    };
    let fraction = if fraction_text.is_empty() {
        0
    } else {
        fraction_text.parse::<u64>().map_err(|_| {
            DbfError::Invalid(format!("currency field {} is out of range", field.name))
        })?
    };
    let scale = match fraction_text.len() {
        0 => CURRENCY_SCALE,
        1 => 1_000,
        2 => 100,
        3 => 10,
        4 => 1,
        _ => unreachable!(),
    };
    let magnitude = whole
        .checked_mul(CURRENCY_SCALE)
        .and_then(|whole| whole.checked_add(fraction * scale))
        .ok_or_else(|| {
            DbfError::Invalid(format!("currency field {} is out of range", field.name))
        })?;
    let limit = i64::MAX as u64 + u64::from(negative);
    if magnitude > limit {
        return Err(DbfError::Invalid(format!(
            "currency field {} is out of range",
            field.name
        )));
    }
    if negative {
        if magnitude == i64::MAX as u64 + 1 {
            Ok(i64::MIN)
        } else {
            Ok(-(magnitude as i64))
        }
    } else {
        Ok(magnitude as i64)
    }
}
