use super::{
    CLASSIC_DESCRIPTOR_SIZE, CURRENCY_SCALE, DbfError, FieldDescriptor, JULIAN_DAY_UNIX_EPOCH,
    MILLISECONDS_PER_DAY, MemoFormat, NullFlagBits, binary_value,
};
use serde_json::{Map, Number, Value};
use std::collections::BTreeSet;

const CP437_UPPER: &str = concat!(
    "\u{c7}\u{fc}\u{e9}\u{e2}\u{e4}\u{e0}\u{e5}\u{e7}\u{ea}\u{eb}\u{e8}\u{ef}\u{ee}\u{ec}\u{c4}\u{c5}\u{c9}\u{e6}\u{c6}\u{f4}\u{f6}\u{f2}\u{fb}\u{f9}\u{ff}\u{d6}\u{dc}\u{a2}\u{a3}\u{a5}\u{20a7}\u{192}",
    "\u{e1}\u{ed}\u{f3}\u{fa}\u{f1}\u{d1}\u{aa}\u{ba}\u{bf}\u{2310}\u{ac}\u{bd}\u{bc}\u{a1}\u{ab}\u{bb}\u{2591}\u{2592}\u{2593}\u{2502}\u{2524}\u{2561}\u{2562}\u{2556}\u{2555}\u{2563}\u{2551}\u{2557}\u{255d}\u{255c}\u{255b}\u{2510}",
    "\u{2514}\u{2534}\u{252c}\u{251c}\u{2500}\u{253c}\u{255e}\u{255f}\u{255a}\u{2554}\u{2569}\u{2566}\u{2560}\u{2550}\u{256c}\u{2567}\u{2568}\u{2564}\u{2565}\u{2559}\u{2558}\u{2552}\u{2553}\u{256b}\u{256a}\u{2518}\u{250c}\u{2588}\u{2584}\u{258c}\u{2590}\u{2580}",
    "\u{3b1}\u{df}\u{393}\u{3c0}\u{3a3}\u{3c3}\u{b5}\u{3c4}\u{3a6}\u{398}\u{3a9}\u{3b4}\u{221e}\u{3c6}\u{3b5}\u{2229}\u{2261}\u{b1}\u{2265}\u{2264}\u{2320}\u{2321}\u{f7}\u{2248}\u{b0}\u{2219}\u{b7}\u{221a}\u{207f}\u{b2}\u{25a0}\u{a0}"
);
const CP850_UPPER: &str = concat!(
    "\u{c7}\u{fc}\u{e9}\u{e2}\u{e4}\u{e0}\u{e5}\u{e7}\u{ea}\u{eb}\u{e8}\u{ef}\u{ee}\u{ec}\u{c4}\u{c5}\u{c9}\u{e6}\u{c6}\u{f4}\u{f6}\u{f2}\u{fb}\u{f9}\u{ff}\u{d6}\u{dc}\u{f8}\u{a3}\u{d8}\u{d7}\u{192}",
    "\u{e1}\u{ed}\u{f3}\u{fa}\u{f1}\u{d1}\u{aa}\u{ba}\u{bf}\u{ae}\u{ac}\u{bd}\u{bc}\u{a1}\u{ab}\u{bb}\u{2591}\u{2592}\u{2593}\u{2502}\u{2524}\u{c1}\u{c2}\u{c0}\u{a9}\u{2563}\u{2551}\u{2557}\u{255d}\u{a2}\u{a5}",
    "\u{2510}\u{2514}\u{2534}\u{252c}\u{251c}\u{2500}\u{253c}\u{e3}\u{c3}\u{255a}\u{2554}\u{2569}\u{2566}\u{2560}\u{2550}\u{256c}\u{a4}\u{f0}\u{d0}\u{ca}\u{cb}\u{c8}\u{131}\u{cd}\u{ce}\u{cf}\u{2518}\u{250c}\u{2588}\u{2584}\u{a6}\u{cc}\u{2580}",
    "\u{d3}\u{df}\u{d4}\u{d2}\u{f5}\u{d5}\u{b5}\u{fe}\u{de}\u{da}\u{db}\u{d9}\u{fd}\u{dd}\u{af}\u{b4}\u{ad}\u{b1}\u{2017}\u{be}\u{b6}\u{a7}\u{f7}\u{b8}\u{b0}\u{a8}\u{b7}\u{b9}\u{b3}\u{b2}\u{25a0}\u{a0}"
);
const CP852_UPPER: &str = concat!(
    "\u{c7}\u{fc}\u{e9}\u{e2}\u{e4}\u{16f}\u{107}\u{e7}\u{142}\u{eb}\u{150}\u{151}\u{ee}\u{179}\u{c4}\u{106}\u{c9}\u{139}\u{13a}\u{f4}\u{f6}\u{13d}\u{13e}\u{15a}\u{15b}\u{d6}\u{dc}\u{164}\u{165}\u{141}\u{d7}\u{10d}",
    "\u{e1}\u{ed}\u{f3}\u{fa}\u{104}\u{105}\u{17d}\u{17e}\u{118}\u{119}\u{ac}\u{17a}\u{10c}\u{15f}\u{ab}\u{bb}\u{2591}\u{2592}\u{2593}\u{2502}\u{2524}\u{c1}\u{c2}\u{11a}\u{15e}\u{2563}\u{2551}\u{2557}\u{255d}\u{17b}\u{17c}\u{2510}",
    "\u{2514}\u{2534}\u{252c}\u{251c}\u{2500}\u{253c}\u{102}\u{103}\u{255a}\u{2554}\u{2569}\u{2566}\u{2560}\u{2550}\u{256c}\u{a4}\u{111}\u{110}\u{10e}\u{cb}\u{10f}\u{147}\u{cd}\u{ce}\u{11b}\u{2518}\u{250c}\u{2588}\u{2584}\u{162}\u{16e}\u{2580}",
    "\u{d3}\u{df}\u{d4}\u{143}\u{144}\u{148}\u{160}\u{161}\u{154}\u{da}\u{155}\u{170}\u{fd}\u{dd}\u{163}\u{b4}\u{ad}\u{2dd}\u{2db}\u{2c7}\u{2d8}\u{a7}\u{f7}\u{b8}\u{b0}\u{a8}\u{2d9}\u{171}\u{158}\u{159}\u{25a0}\u{a0}"
);
const CP866_UPPER: &str = concat!(
    "\u{410}\u{411}\u{412}\u{413}\u{414}\u{415}\u{416}\u{417}\u{418}\u{419}\u{41a}\u{41b}\u{41c}\u{41d}\u{41e}\u{41f}\u{420}\u{421}\u{422}\u{423}\u{424}\u{425}\u{426}\u{427}\u{428}\u{429}\u{42a}\u{42b}\u{42c}\u{42d}\u{42e}\u{42f}",
    "\u{430}\u{431}\u{432}\u{433}\u{434}\u{435}\u{436}\u{437}\u{438}\u{439}\u{43a}\u{43b}\u{43c}\u{43d}\u{43e}\u{43f}\u{2591}\u{2592}\u{2593}\u{2502}\u{2524}\u{2561}\u{2562}\u{2556}\u{2555}\u{2563}\u{2551}\u{2557}\u{255d}\u{255c}\u{255b}\u{2510}",
    "\u{2514}\u{2534}\u{252c}\u{251c}\u{2500}\u{253c}\u{255e}\u{255f}\u{255a}\u{2554}\u{2569}\u{2566}\u{2560}\u{2550}\u{256c}\u{2567}\u{2568}\u{2564}\u{2565}\u{2559}\u{2558}\u{2552}\u{2553}\u{256b}\u{256a}\u{2518}\u{250c}\u{2588}\u{2584}\u{258c}\u{2590}\u{2580}",
    "\u{440}\u{441}\u{442}\u{443}\u{444}\u{445}\u{446}\u{447}\u{448}\u{449}\u{44a}\u{44b}\u{44c}\u{44d}\u{44e}\u{44f}\u{401}\u{451}\u{404}\u{454}\u{407}\u{457}\u{40e}\u{45e}\u{b0}\u{2219}\u{b7}\u{221a}\u{2116}\u{a4}\u{25a0}\u{a0}"
);
const CP1250_UPPER: &str = concat!(
    "\u{20ac}\u{fffd}\u{201a}\u{fffd}\u{201e}\u{2026}\u{2020}\u{2021}\u{fffd}\u{2030}\u{160}\u{2039}\u{15a}\u{164}\u{17d}\u{179}\u{fffd}\u{2018}\u{2019}\u{201c}\u{201d}\u{2022}\u{2013}\u{2014}\u{fffd}\u{2122}\u{161}\u{203a}\u{15b}\u{165}\u{17e}\u{17a}",
    "\u{a0}\u{2c7}\u{2d8}\u{141}\u{a4}\u{104}\u{a6}\u{a7}\u{a8}\u{a9}\u{15e}\u{ab}\u{ac}\u{ad}\u{ae}\u{17b}\u{b0}\u{b1}\u{2db}\u{142}\u{b4}\u{b5}\u{b6}\u{b7}\u{b8}\u{105}\u{15f}\u{bb}\u{13d}\u{2dd}\u{13e}\u{17c}",
    "\u{154}\u{c1}\u{c2}\u{102}\u{c4}\u{139}\u{106}\u{c7}\u{10c}\u{c9}\u{118}\u{cb}\u{11a}\u{cd}\u{ce}\u{10e}\u{110}\u{143}\u{147}\u{d3}\u{d4}\u{150}\u{d6}\u{d7}\u{158}\u{16e}\u{da}\u{170}\u{dc}\u{dd}\u{162}\u{df}",
    "\u{155}\u{e1}\u{e2}\u{103}\u{e4}\u{13a}\u{107}\u{e7}\u{10d}\u{e9}\u{119}\u{eb}\u{11b}\u{ed}\u{ee}\u{10f}\u{111}\u{144}\u{148}\u{f3}\u{f4}\u{151}\u{f6}\u{f7}\u{159}\u{16f}\u{fa}\u{171}\u{fc}\u{fd}\u{163}\u{2d9}"
);
const CP1251_UPPER: &str = concat!(
    "\u{402}\u{403}\u{201a}\u{453}\u{201e}\u{2026}\u{2020}\u{2021}\u{20ac}\u{2030}\u{409}\u{2039}\u{40a}\u{40c}\u{40b}\u{40f}\u{452}\u{2018}\u{2019}\u{201c}\u{201d}\u{2022}\u{2013}\u{2014}\u{fffd}\u{2122}\u{459}\u{203a}\u{45a}\u{45c}\u{45b}\u{45f}",
    "\u{a0}\u{40e}\u{45e}\u{408}\u{a4}\u{490}\u{a6}\u{a7}\u{401}\u{a9}\u{404}\u{ab}\u{ac}\u{ad}\u{ae}\u{407}\u{b0}\u{b1}\u{406}\u{456}\u{491}\u{b5}\u{b6}\u{b7}\u{451}\u{2116}\u{454}\u{bb}\u{458}\u{405}\u{455}\u{457}",
    "\u{410}\u{411}\u{412}\u{413}\u{414}\u{415}\u{416}\u{417}\u{418}\u{419}\u{41a}\u{41b}\u{41c}\u{41d}\u{41e}\u{41f}\u{420}\u{421}\u{422}\u{423}\u{424}\u{425}\u{426}\u{427}\u{428}\u{429}\u{42a}\u{42b}\u{42c}\u{42d}\u{42e}\u{42f}",
    "\u{430}\u{431}\u{432}\u{433}\u{434}\u{435}\u{436}\u{437}\u{438}\u{439}\u{43a}\u{43b}\u{43c}\u{43d}\u{43e}\u{43f}\u{440}\u{441}\u{442}\u{443}\u{444}\u{445}\u{446}\u{447}\u{448}\u{449}\u{44a}\u{44b}\u{44c}\u{44d}\u{44e}\u{44f}"
);

pub(super) fn encode_field(
    field: &FieldDescriptor,
    value: &Value,
    language_driver: u8,
) -> Result<Vec<u8>, DbfError> {
    let length = usize::from(field.length);
    match field.field_type.to_ascii_uppercase() {
        b'Q' | b'V' => encode_variable_field(value, field, language_driver),
        b'C' => {
            let bytes = if field.is_binary() {
                binary_value(value, field)?
            } else {
                encode_character(value, field, language_driver)?
            };
            if bytes.len() > length {
                return Err(DbfError::Invalid(format!(
                    "value for {} exceeds field width {}",
                    field.name, field.length
                )));
            }
            let mut output = vec![if field.is_binary() { 0 } else { b' ' }; length];
            output[..bytes.len()].copy_from_slice(&bytes);
            Ok(output)
        }
        b'B' if length == 8 => Ok(value_f64(value, field)?.to_le_bytes().to_vec()),
        b'Y' => {
            if length != 8 {
                return Err(DbfError::Invalid(format!(
                    "currency field {} must be eight bytes",
                    field.name
                )));
            }
            Ok(currency_i64(value, field)?.to_le_bytes().to_vec())
        }
        b'B' | b'G' | b'M' | b'P' | b'W' if length == 4 => {
            Ok(value_u32(value, field)?.to_le_bytes().to_vec())
        }
        b'D' => {
            if length != 8 {
                return Err(DbfError::Invalid(format!(
                    "date field {} must be eight bytes",
                    field.name
                )));
            }
            let text = value_text(value, field)?;
            if text.is_empty() {
                return Ok(vec![b' '; length]);
            }
            if text.len() != length || !text.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(DbfError::Invalid(format!(
                    "date field {} requires YYYYMMDD or null",
                    field.name
                )));
            }
            Ok(text.into_bytes())
        }
        b'B' | b'G' | b'M' => {
            let text = value_text(value, field)?;
            if text.len() > length {
                return Err(DbfError::Invalid(format!(
                    "value for {} exceeds field width {}",
                    field.name, field.length
                )));
            }
            let mut bytes = vec![b' '; length];
            bytes[..text.len()].copy_from_slice(text.as_bytes());
            Ok(bytes)
        }
        b'@' | b'T' => {
            if value.is_null() {
                return Ok(if field.field_type.eq_ignore_ascii_case(&b'T') {
                    vec![0; length]
                } else {
                    vec![b' '; length]
                });
            }
            if field.field_type.eq_ignore_ascii_case(&b'T') && length == 8 {
                if let Some(text) = value.as_str() {
                    if let Some(bytes) = foxpro_datetime_bytes(text)? {
                        return Ok(bytes.to_vec());
                    }
                }
            }
            let bytes = binary_value(value, field)?;
            if bytes.len() != length {
                return Err(DbfError::Invalid(format!(
                    "timestamp field {} requires exactly {length} bytes",
                    field.name
                )));
            }
            Ok(bytes)
        }
        b'N' | b'F' => {
            let text = numeric_field_text(value, field)?;
            if text.len() > length {
                return Err(DbfError::Invalid(format!(
                    "value for {} exceeds field width {}",
                    field.name, field.length
                )));
            }
            let mut bytes = vec![b' '; length];
            let start = length - text.len();
            bytes[start..].copy_from_slice(text.as_bytes());
            Ok(bytes)
        }
        b'L' => {
            let marker = match value {
                Value::Null => b' ',
                Value::Bool(true) => b'T',
                Value::Bool(false) => b'F',
                Value::String(text) if text.len() == 1 => text.as_bytes()[0].to_ascii_uppercase(),
                _ => {
                    return Err(DbfError::Invalid(format!(
                        "logical field {} requires boolean, one-byte text, or null",
                        field.name
                    )));
                }
            };
            if !matches!(marker, b' ' | b'T' | b'F' | b'Y' | b'N') {
                return Err(DbfError::Invalid(format!(
                    "invalid logical marker for {}",
                    field.name
                )));
            }
            Ok(vec![marker; length])
        }
        b'I' | b'+' => {
            if length != 4 {
                return Err(DbfError::Invalid(format!(
                    "integer field {} must be four bytes",
                    field.name
                )));
            }
            let integer = value_i64(value, field)?;
            let integer = i32::try_from(integer).map_err(|_| {
                DbfError::Invalid(format!("integer value for {} is out of range", field.name))
            })?;
            Ok(integer.to_le_bytes().to_vec())
        }
        b'O' => {
            if length != 8 {
                return Err(DbfError::Invalid(format!(
                    "double field {} must be eight bytes",
                    field.name
                )));
            }
            let number = value_f64(value, field)?;
            Ok(number.to_le_bytes().to_vec())
        }
        field_type => Err(DbfError::Invalid(format!(
            "writing field type 0x{field_type:02x} is unsupported"
        ))),
    }
}

pub(super) fn encode_variable_field(
    value: &Value,
    field: &FieldDescriptor,
    language_driver: u8,
) -> Result<Vec<u8>, DbfError> {
    let length = usize::from(field.length);
    if length == 0 {
        return Err(DbfError::Invalid(format!(
            "variable field {} must not be empty",
            field.name
        )));
    }
    if value.is_null() {
        return Ok(vec![0; length]);
    }

    let data = if field.is_binary() {
        binary_value(value, field)?
    } else {
        encode_character(value, field, language_driver)?
    };
    let max_length = length - 1;
    if data.len() > max_length {
        return Err(DbfError::Invalid(format!(
            "value for {} exceeds variable field width {}",
            field.name, field.length
        )));
    }
    let mut output = vec![if field.is_binary() { 0 } else { b' ' }; length];
    output[..data.len()].copy_from_slice(&data);
    output[max_length] = u8::try_from(data.len()).map_err(|_| {
        DbfError::Invalid(format!(
            "value for {} exceeds variable field width {}",
            field.name, field.length
        ))
    })?;
    Ok(output)
}

pub(super) fn encode_character(
    value: &Value,
    field: &FieldDescriptor,
    language_driver: u8,
) -> Result<Vec<u8>, DbfError> {
    let text = value_text(value, field)?;
    let encoded = match language_driver {
        0x01 => encode_codepage(&text, CP437_UPPER),
        0x02 => encode_codepage(&text, CP850_UPPER),
        0x1f | 0x22 | 0x23 | 0x40 | 0x64 | 0x87 => encode_codepage(&text, CP852_UPPER),
        0x26 | 0x65 => encode_codepage(&text, CP866_UPPER),
        0xc8 => encode_codepage(&text, CP1250_UPPER),
        0xc9 => encode_codepage(&text, CP1251_UPPER),
        0x03 | 0x57 => encode_windows_1252(&text),
        _ => Some(text.into_bytes()),
    };
    encoded.ok_or_else(|| {
        let code_page = match language_driver {
            0x01 => "CP437",
            0x02 => "CP850",
            0x1f | 0x22 | 0x23 | 0x40 | 0x64 | 0x87 => "CP852",
            0x26 | 0x65 => "CP866",
            0xc8 => "Windows-1250",
            0xc9 => "Windows-1251",
            0x03 | 0x57 => "Windows-1252",
            _ => "the declared code page",
        };
        DbfError::Invalid(format!(
            "value for {} contains a character outside {code_page}",
            field.name
        ))
    })
}

pub(super) fn value_text(value: &Value, field: &FieldDescriptor) -> Result<String, DbfError> {
    match value {
        Value::Null => Ok(String::new()),
        Value::String(text) => Ok(text.clone()),
        Value::Number(number) => Ok(number.to_string()),
        Value::Bool(boolean) => Ok(boolean.to_string()),
        _ => Err(DbfError::Invalid(format!(
            "field {} requires a scalar value",
            field.name
        ))),
    }
}

pub(super) fn numeric_field_text(
    value: &Value,
    field: &FieldDescriptor,
) -> Result<String, DbfError> {
    let text = value_text(value, field)?;
    if text.is_empty() {
        return Ok(text);
    }
    let text = text.trim();
    let number = text.parse::<f64>().map_err(|_| {
        DbfError::Invalid(format!(
            "numeric field {} requires a finite number or null",
            field.name
        ))
    })?;
    if !number.is_finite() {
        return Err(DbfError::Invalid(format!(
            "numeric field {} requires a finite number or null",
            field.name
        )));
    }
    Ok(text.to_owned())
}

pub(super) fn value_i64(value: &Value, field: &FieldDescriptor) -> Result<i64, DbfError> {
    match value {
        Value::Null => Ok(0),
        Value::Number(number) => number
            .as_i64()
            .ok_or_else(|| DbfError::Invalid(format!("field {} requires an integer", field.name))),
        Value::String(text) => text
            .trim()
            .parse::<i64>()
            .map_err(|_| DbfError::Invalid(format!("field {} requires an integer", field.name))),
        _ => Err(DbfError::Invalid(format!(
            "field {} requires an integer",
            field.name
        ))),
    }
}

pub(super) fn value_u32(value: &Value, field: &FieldDescriptor) -> Result<u32, DbfError> {
    match value {
        Value::Null => Ok(0),
        Value::Number(number) => number
            .as_u64()
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| DbfError::Invalid(format!("field {} requires a uint32", field.name))),
        Value::String(text) => text
            .trim()
            .parse::<u32>()
            .map_err(|_| DbfError::Invalid(format!("field {} requires a uint32", field.name))),
        _ => Err(DbfError::Invalid(format!(
            "field {} requires a uint32",
            field.name
        ))),
    }
}

pub(super) fn value_f64(value: &Value, field: &FieldDescriptor) -> Result<f64, DbfError> {
    let number = match value {
        Value::Null => 0.0,
        Value::Number(number) => number.as_f64().ok_or_else(|| {
            DbfError::Invalid(format!("field {} requires a finite number", field.name))
        })?,
        Value::String(text) => text.trim().parse::<f64>().map_err(|_| {
            DbfError::Invalid(format!("field {} requires a finite number", field.name))
        })?,
        _ => {
            return Err(DbfError::Invalid(format!(
                "field {} requires a number",
                field.name
            )));
        }
    };
    if number.is_finite() {
        Ok(number)
    } else {
        Err(DbfError::Invalid(format!(
            "field {} requires a finite number",
            field.name
        )))
    }
}

pub(super) fn foxpro_datetime_bytes(text: &str) -> Result<Option<[u8; 8]>, DbfError> {
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

pub(super) fn foxpro_datetime_text(bytes: &[u8]) -> Option<String> {
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

pub(super) fn currency_text(bytes: &[u8]) -> String {
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

pub(super) fn currency_i64(value: &Value, field: &FieldDescriptor) -> Result<i64, DbfError> {
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

pub(super) fn parse_fields(
    bytes: &[u8],
    start: usize,
    end: usize,
    descriptor_size: usize,
) -> Result<Vec<FieldDescriptor>, DbfError> {
    if (end - start) % descriptor_size != 0 {
        return Err(DbfError::Invalid(
            "field descriptor area is misaligned".into(),
        ));
    }

    let (name_size, type_offset, length_offset, decimal_offset) =
        if descriptor_size == CLASSIC_DESCRIPTOR_SIZE {
            (11, 11, 16, 17)
        } else {
            (32, 32, 33, 34)
        };
    let mut names = BTreeSet::new();
    let mut offset = 1;
    let mut fields = Vec::with_capacity((end - start) / descriptor_size);
    for descriptor in bytes[start..end].chunks_exact(descriptor_size) {
        let name_end = descriptor[..name_size]
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(name_size);
        let name = String::from_utf8_lossy(&descriptor[..name_end])
            .trim()
            .to_owned();
        if name.is_empty() || !names.insert(name.clone()) {
            return Err(DbfError::Invalid(
                "field names must be non-empty and unique".into(),
            ));
        }
        let length = descriptor[length_offset];
        if length == 0 {
            return Err(DbfError::Invalid(format!("field {name} has zero width")));
        }
        fields.push(FieldDescriptor {
            name,
            field_type: descriptor[type_offset],
            length,
            decimal_count: descriptor[decimal_offset],
            flags: if descriptor_size == CLASSIC_DESCRIPTOR_SIZE {
                descriptor[18]
            } else {
                0
            },
            offset,
        });
        offset = offset
            .checked_add(length as usize)
            .ok_or_else(|| DbfError::Invalid("field offsets overflow usize".into()))?;
    }
    Ok(fields)
}

pub(super) fn null_flag_layout(fields: &[FieldDescriptor]) -> Vec<Option<NullFlagBits>> {
    let mut next_bit = 0;
    fields
        .iter()
        .map(|field| {
            if field.is_system() {
                return None;
            }
            let varlength = field.is_variable().then(|| {
                let bit = next_bit;
                next_bit += 1;
                bit
            });
            let nullable = field.is_nullable().then(|| {
                let bit = next_bit;
                next_bit += 1;
                bit
            });
            (varlength.is_some() || nullable.is_some()).then_some(NullFlagBits {
                varlength,
                nullable,
            })
        })
        .collect()
}

pub(super) fn system_field_index(fields: &[FieldDescriptor]) -> Option<usize> {
    fields.iter().position(FieldDescriptor::is_system)
}

pub(super) fn flag_is_set(flags: &[u8], bit: usize) -> bool {
    flags
        .get(bit / 8)
        .is_some_and(|byte| byte & (1 << (bit % 8)) != 0)
}

pub(super) fn set_flag(flags: &mut [u8], bit: usize, value: bool) -> Result<(), DbfError> {
    let Some(byte) = flags.get_mut(bit / 8) else {
        return Err(DbfError::Invalid(
            "_NullFlags field is too short for its descriptors".into(),
        ));
    };
    let mask = 1 << (bit % 8);
    if value {
        *byte |= mask;
    } else {
        *byte &= !mask;
    }
    Ok(())
}

pub(super) fn encode_null_flags(
    fields: &[FieldDescriptor],
    values: &Map<String, Value>,
) -> Result<Option<Vec<u8>>, DbfError> {
    let layout = null_flag_layout(fields);
    let Some(system_index) = system_field_index(fields) else {
        if layout.iter().any(Option::is_some) {
            return Err(DbfError::Invalid(
                "VFP variable/null fields require a _NullFlags system field".into(),
            ));
        }
        return Ok(None);
    };

    let system = &fields[system_index];
    let mut flags = vec![0; usize::from(system.length)];
    for (index, field) in fields.iter().enumerate() {
        if field.is_system() {
            continue;
        }
        let Some(bits) = layout[index] else {
            continue;
        };
        if let Some(bit) = bits.varlength {
            set_flag(&mut flags, bit, true)?;
        }
        if let Some(bit) = bits.nullable {
            let is_null = values.get(&field.name).is_none_or(|value| value.is_null());
            set_flag(&mut flags, bit, is_null)?;
        }
    }
    Ok(Some(flags))
}

pub(super) fn update_null_flags(
    bytes: &mut [u8],
    record_offset: usize,
    fields: &[FieldDescriptor],
    values: &Map<String, Value>,
    changed_fields: &BTreeSet<String>,
) -> Result<(), DbfError> {
    let layout = null_flag_layout(fields);
    let relevant_change = fields.iter().enumerate().any(|(index, field)| {
        !field.is_system() && changed_fields.contains(&field.name) && layout[index].is_some()
    });
    if !relevant_change {
        return Ok(());
    }

    let Some(system_index) = system_field_index(fields) else {
        return Err(DbfError::Invalid(
            "VFP variable/null fields require a _NullFlags system field".into(),
        ));
    };
    let system = &fields[system_index];
    let start = record_offset
        .checked_add(system.offset)
        .ok_or_else(|| DbfError::Invalid("_NullFlags offset overflows usize".into()))?;
    let end = start
        .checked_add(usize::from(system.length))
        .ok_or_else(|| DbfError::Invalid("_NullFlags range overflows usize".into()))?;
    let flags = bytes
        .get_mut(start..end)
        .ok_or_else(|| DbfError::Invalid("stored _NullFlags field is truncated".into()))?;

    for (index, field) in fields.iter().enumerate() {
        if field.is_system() || !changed_fields.contains(&field.name) {
            continue;
        }
        let Some(bits) = layout[index] else {
            continue;
        };
        if let Some(bit) = bits.varlength {
            set_flag(flags, bit, true)?;
        }
        if let Some(bit) = bits.nullable {
            let is_null = values.get(&field.name).is_none_or(|value| value.is_null());
            set_flag(flags, bit, is_null)?;
        }
    }
    Ok(())
}

pub(super) fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, DbfError> {
    let bytes = bytes
        .get(offset..offset + 2)
        .ok_or_else(|| DbfError::Invalid("header is truncated".into()))?;
    Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
}

pub(super) fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, DbfError> {
    let bytes = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| DbfError::Invalid("header is truncated".into()))?;
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

pub(super) fn decode_record_field(
    field: &FieldDescriptor,
    bytes: &[u8],
    language_driver: u8,
    null_flags: Option<&[u8]>,
    flag_bits: Option<NullFlagBits>,
) -> Value {
    let is_null = flag_bits
        .and_then(|bits| bits.nullable)
        .is_some_and(|bit| null_flags.is_some_and(|flags| flag_is_set(flags, bit)));
    if is_null {
        return Value::Null;
    }

    let data = match flag_bits.and_then(|bits| bits.varlength) {
        Some(bit) if null_flags.is_some_and(|flags| flag_is_set(flags, bit)) => {
            match bytes.last().map(|length| usize::from(*length)) {
                Some(length) if length <= bytes.len().saturating_sub(1) => &bytes[..length],
                _ => bytes,
            }
        }
        _ => bytes,
    };
    if field.is_binary() {
        Value::String(hex(data))
    } else {
        Value::String(text(data, language_driver))
    }
}

pub(super) fn decode_field(
    field_type: u8,
    bytes: &[u8],
    language_driver: u8,
    memo_format: Option<MemoFormat>,
) -> Value {
    match field_type.to_ascii_uppercase() {
        b'C' => Value::String(text(bytes, language_driver)),
        b'D' if bytes.iter().all(|byte| matches!(*byte, b' ' | 0)) => Value::Null,
        b'T' if memo_format == Some(MemoFormat::FoxPro) && bytes.len() >= 8 => {
            let day = u32::from_le_bytes(bytes[..4].try_into().unwrap());
            if day == 0 {
                Value::Null
            } else if let Some(value) = foxpro_datetime_text(bytes) {
                Value::String(value)
            } else {
                Value::String(hex(bytes))
            }
        }
        b'Y' if bytes.len() >= 8 => Value::String(currency_text(bytes)),
        b'B' if memo_format.is_some_and(|format| format != MemoFormat::FoxPro)
            && bytes.len() != 4 =>
        {
            numeric(bytes)
        }
        b'B' if bytes.len() >= 8 => {
            let value = f64::from_le_bytes(bytes[..8].try_into().unwrap());
            if value.is_finite() {
                Number::from_f64(value)
                    .map(Value::Number)
                    .unwrap_or(Value::Null)
            } else {
                Value::Null
            }
        }
        b'B' | b'G' | b'M' | b'P' | b'W' if bytes.len() == 4 => {
            let block = if memo_format == Some(MemoFormat::FoxPro) {
                u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
            } else {
                u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
            };
            Value::Number(block.into())
        }
        b'D' | b'B' | b'G' | b'M' | b'P' | b'W' => Value::String(text(bytes, 0)),
        b'F' | b'N' => numeric(bytes),
        b'L' => match bytes.first().map(|byte| byte.to_ascii_uppercase()) {
            Some(b'T') | Some(b'Y') => Value::Bool(true),
            Some(b'F') | Some(b'N') => Value::Bool(false),
            _ => Value::Null,
        },
        b'I' | b'+' if bytes.len() >= 4 => {
            Value::Number(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]).into())
        }
        b'O' if bytes.len() >= 8 => {
            let value = f64::from_le_bytes(bytes[..8].try_into().unwrap());
            if value.is_finite() {
                Number::from_f64(value)
                    .map(Value::Number)
                    .unwrap_or(Value::Null)
            } else {
                Value::Null
            }
        }
        b'@' | b'T' => Value::String(hex(bytes)),
        _ => Value::String(text(bytes, language_driver)),
    }
}

pub(super) fn text(bytes: &[u8], language_driver: u8) -> String {
    let end = bytes
        .iter()
        .rposition(|byte| !matches!(byte, b' ' | b'\0'))
        .map_or(0, |index| index + 1);
    let bytes = &bytes[..end];
    match language_driver {
        0x01 => decode_codepage(bytes, CP437_UPPER),
        0x02 => decode_codepage(bytes, CP850_UPPER),
        0x1f | 0x22 | 0x23 | 0x40 | 0x64 | 0x87 => decode_codepage(bytes, CP852_UPPER),
        0x26 | 0x65 => decode_codepage(bytes, CP866_UPPER),
        0xc8 => decode_codepage(bytes, CP1250_UPPER),
        0xc9 => decode_codepage(bytes, CP1251_UPPER),
        0x03 | 0x57 => decode_windows_1252(bytes),
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}

pub(super) fn decode_codepage(bytes: &[u8], upper: &str) -> String {
    bytes
        .iter()
        .map(|byte| {
            if *byte < 0x80 {
                char::from(*byte)
            } else {
                upper
                    .chars()
                    .nth(usize::from(*byte - 0x80))
                    .unwrap_or('\u{fffd}')
            }
        })
        .collect()
}

pub(super) fn encode_codepage(text: &str, upper: &str) -> Option<Vec<u8>> {
    text.chars()
        .map(|character| {
            if character <= '\u{7f}' {
                u8::try_from(u32::from(character)).ok()
            } else {
                upper
                    .chars()
                    .position(|candidate| candidate == character)
                    .and_then(|index| u8::try_from(index + 0x80).ok())
            }
        })
        .collect()
}

pub(super) fn decode_windows_1252(bytes: &[u8]) -> String {
    const EXTENDED: [char; 32] = [
        '€', '\u{fffd}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{fffd}', 'Ž',
        '\u{fffd}', '\u{fffd}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ',
        '\u{fffd}', 'ž', 'Ÿ',
    ];
    bytes
        .iter()
        .map(|byte| match byte {
            0x00..=0x7f => char::from(*byte),
            0x80..=0x9f => EXTENDED[usize::from(*byte - 0x80)],
            byte => char::from_u32(u32::from(*byte)).expect("Windows-1252 byte is valid"),
        })
        .collect()
}

pub(super) fn encode_windows_1252(text: &str) -> Option<Vec<u8>> {
    text.chars()
        .map(|character| match character {
            '\u{0000}'..='\u{007f}' | '\u{00a0}'..='\u{00ff}' => Some(character as u32 as u8),
            '€' => Some(0x80),
            '‚' => Some(0x82),
            'ƒ' => Some(0x83),
            '„' => Some(0x84),
            '…' => Some(0x85),
            '†' => Some(0x86),
            '‡' => Some(0x87),
            'ˆ' => Some(0x88),
            '‰' => Some(0x89),
            'Š' => Some(0x8a),
            '‹' => Some(0x8b),
            'Œ' => Some(0x8c),
            'Ž' => Some(0x8e),
            '‘' => Some(0x91),
            '’' => Some(0x92),
            '“' => Some(0x93),
            '”' => Some(0x94),
            '•' => Some(0x95),
            '–' => Some(0x96),
            '—' => Some(0x97),
            '˜' => Some(0x98),
            '™' => Some(0x99),
            'š' => Some(0x9a),
            '›' => Some(0x9b),
            'œ' => Some(0x9c),
            'ž' => Some(0x9e),
            'Ÿ' => Some(0x9f),
            _ => None,
        })
        .collect()
}

pub(super) fn numeric(bytes: &[u8]) -> Value {
    let value = text(bytes, 0).trim().to_owned();
    if value.is_empty() {
        return Value::Null;
    }
    if let Ok(integer) = value.parse::<i64>() {
        return Value::Number(integer.into());
    }
    if let Ok(float) = value.parse::<f64>() {
        if let Some(number) = Number::from_f64(float) {
            return Value::Number(number);
        }
    }
    Value::String(value)
}

pub(super) fn hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write;
        let _ = write!(output, "{byte:02x}");
    }
    output
}
