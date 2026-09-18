use super::{
    DBT_BLOCK_SIZE, DbfError, EOF_MARKER, FieldDescriptor, MemoFile, MemoFormat, MemoUpdate,
    value_text,
};
use serde_json::Value;
use std::fs;
use std::path::Path;

impl MemoFile {
    pub(super) fn open(path: &Path, dbf_version: u8) -> Result<Self, DbfError> {
        let bytes = fs::read(path)?;
        if path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("fpt"))
        {
            let block_size = bytes
                .get(6..8)
                .map(|header| usize::from(u16::from_be_bytes([header[0], header[1]])))
                .ok_or_else(|| DbfError::Invalid("FPT header is truncated".into()))?;
            if block_size < 8 || bytes.len() < block_size {
                return Err(DbfError::Invalid(
                    "FPT block size or header is invalid".into(),
                ));
            }
            Ok(Self {
                bytes,
                block_size,
                format: MemoFormat::FoxPro,
            })
        } else {
            if bytes.len() < DBT_BLOCK_SIZE {
                return Err(DbfError::Invalid("DBT header is truncated".into()));
            }
            let format = if dbf_version == 0x83 {
                MemoFormat::Dbase3
            } else {
                MemoFormat::Dbase4
            };
            let block_size = if format == MemoFormat::Dbase4 {
                let block_size = usize::from(u16::from_le_bytes([bytes[20], bytes[21]]));
                if block_size == 0 {
                    DBT_BLOCK_SIZE
                } else {
                    block_size
                }
            } else {
                DBT_BLOCK_SIZE
            };
            if block_size < DBT_BLOCK_SIZE
                || block_size % DBT_BLOCK_SIZE != 0
                || bytes.len() < block_size
            {
                return Err(DbfError::Invalid(
                    "dBASE DBT block size or header is invalid".into(),
                ));
            }
            Ok(Self {
                bytes,
                block_size,
                format,
            })
        }
    }

    pub(super) fn read(&self, block: u32) -> Result<Option<Vec<u8>>, DbfError> {
        if block == 0 {
            return Ok(None);
        }
        let start = usize::try_from(block)
            .ok()
            .and_then(|block| block.checked_mul(self.block_size))
            .ok_or_else(|| DbfError::Invalid("memo block offset overflows usize".into()))?;
        if start >= self.bytes.len() {
            return Err(DbfError::Invalid(format!(
                "memo block {block} is outside the sidecar"
            )));
        }
        match self.format {
            MemoFormat::Dbase3 => {
                let data = &self.bytes[start..];
                let end = data
                    .windows(2)
                    .position(|pair| pair == [EOF_MARKER, EOF_MARKER])
                    .or_else(|| {
                        data.iter()
                            .position(|byte| *byte == EOF_MARKER)
                            .filter(|&index| {
                                data[index + 1..]
                                    .iter()
                                    .all(|byte| matches!(*byte, 0 | b' ' | EOF_MARKER))
                            })
                    })
                    .unwrap_or(data.len());
                Ok(Some(data[..end].to_vec()))
            }
            MemoFormat::Dbase4 => {
                let header_end = start
                    .checked_add(8)
                    .ok_or_else(|| DbfError::Invalid("memo header overflows usize".into()))?;
                let header = self
                    .bytes
                    .get(start..header_end)
                    .ok_or_else(|| DbfError::Invalid("memo block header is truncated".into()))?;
                let total_length = usize::try_from(u32::from_le_bytes([
                    header[4], header[5], header[6], header[7],
                ]))
                .map_err(|_| DbfError::Invalid("memo length overflows usize".into()))?;
                let length = total_length.checked_sub(8).ok_or_else(|| {
                    DbfError::Invalid("dBASE IV memo length is smaller than its header".into())
                })?;
                let end = header_end
                    .checked_add(length)
                    .ok_or_else(|| DbfError::Invalid("memo data length overflows usize".into()))?;
                let data = self
                    .bytes
                    .get(header_end..end)
                    .ok_or_else(|| DbfError::Invalid("memo data is truncated".into()))?;
                Ok(Some(data.to_vec()))
            }
            MemoFormat::FoxPro => {
                let header_end = start
                    .checked_add(8)
                    .ok_or_else(|| DbfError::Invalid("memo header overflows usize".into()))?;
                let header = self
                    .bytes
                    .get(start..header_end)
                    .ok_or_else(|| DbfError::Invalid("memo block header is truncated".into()))?;
                let length = usize::try_from(u32::from_be_bytes([
                    header[4], header[5], header[6], header[7],
                ]))
                .map_err(|_| DbfError::Invalid("memo length overflows usize".into()))?;
                let end = header_end
                    .checked_add(length)
                    .ok_or_else(|| DbfError::Invalid("memo data length overflows usize".into()))?;
                let data = self
                    .bytes
                    .get(header_end..end)
                    .ok_or_else(|| DbfError::Invalid("memo data is truncated".into()))?;
                Ok(Some(data.to_vec()))
            }
        }
    }

    pub(super) fn append_text(&mut self, data: &[u8]) -> Result<u32, DbfError> {
        let mut payload = Vec::new();
        match self.format {
            MemoFormat::Dbase3 => {
                if data.contains(&EOF_MARKER) {
                    return Err(DbfError::Invalid(
                        "dBASE III memo text contains the end marker".into(),
                    ));
                }
                payload.extend_from_slice(data);
                payload.extend_from_slice(&[EOF_MARKER, EOF_MARKER]);
            }
            MemoFormat::Dbase4 => {
                let length = u32::try_from(
                    data.len()
                        .checked_add(8)
                        .ok_or_else(|| DbfError::Invalid("memo text is too long".into()))?,
                )
                .map_err(|_| DbfError::Invalid("memo text is too long".into()))?;
                payload.extend_from_slice(&[0xff, 0xff, 0x08, 0x00]);
                payload.extend_from_slice(&length.to_le_bytes());
                payload.extend_from_slice(data);
            }
            MemoFormat::FoxPro => {
                let length = u32::try_from(data.len())
                    .map_err(|_| DbfError::Invalid("memo text is too long".into()))?;
                payload.extend_from_slice(&1u32.to_be_bytes());
                payload.extend_from_slice(&length.to_be_bytes());
                payload.extend_from_slice(data);
            }
        }
        self.append_payload(payload)
    }

    pub(super) fn append_binary(&mut self, data: &[u8]) -> Result<u32, DbfError> {
        let length = u32::try_from(data.len())
            .map_err(|_| DbfError::Invalid("binary data is too long".into()))?;
        let mut payload = Vec::with_capacity(8usize.saturating_add(data.len()));
        match self.format {
            MemoFormat::Dbase3 => {
                // ponytail: dBASE III has no binary length; reserve 0x1a1a as its
                // terminator and reject payloads that cannot round-trip through it.
                if data.last() == Some(&EOF_MARKER)
                    || data.windows(2).any(|pair| pair == [EOF_MARKER, EOF_MARKER])
                {
                    return Err(DbfError::Invalid(
                        "dBASE III binary data cannot contain its 0x1a1a terminator".into(),
                    ));
                }
            }
            MemoFormat::Dbase4 => {
                let total_length = length
                    .checked_add(8)
                    .ok_or_else(|| DbfError::Invalid("binary data is too long".into()))?;
                payload.extend_from_slice(&[0xff, 0xff, 0x08, 0x00]);
                payload.extend_from_slice(&total_length.to_le_bytes());
            }
            MemoFormat::FoxPro => {
                payload.extend_from_slice(&0u32.to_be_bytes());
                payload.extend_from_slice(&length.to_be_bytes());
            }
        }
        payload.extend_from_slice(data);
        if self.format == MemoFormat::Dbase3 {
            payload.extend_from_slice(&[EOF_MARKER, EOF_MARKER]);
        }
        self.append_payload(payload)
    }

    pub(super) fn append_payload(&mut self, mut payload: Vec<u8>) -> Result<u32, DbfError> {
        if self.bytes.len() < 4 {
            return Err(DbfError::Invalid("memo header is truncated".into()));
        }
        let start_block = self.bytes.len() / self.block_size
            + usize::from(self.bytes.len() % self.block_size != 0);
        let aligned_length = start_block
            .checked_mul(self.block_size)
            .ok_or_else(|| DbfError::Invalid("memo block offset overflows usize".into()))?;
        if self.bytes.len() < aligned_length {
            self.bytes.resize(aligned_length, 0);
        }
        let block_count =
            payload.len() / self.block_size + usize::from(payload.len() % self.block_size != 0);
        let padded_length = block_count
            .checked_mul(self.block_size)
            .ok_or_else(|| DbfError::Invalid("memo text is too long".into()))?;
        let padding = if self.format == MemoFormat::FoxPro {
            0
        } else {
            b' '
        };
        payload.resize(padded_length, padding);
        self.bytes.extend_from_slice(&payload);

        let next_block = start_block
            .checked_add(block_count)
            .and_then(|block| u32::try_from(block).ok())
            .ok_or_else(|| DbfError::Invalid("memo block number overflows u32".into()))?;
        let next_block_bytes = if self.format == MemoFormat::Dbase4 {
            next_block.to_le_bytes()
        } else {
            next_block.to_be_bytes()
        };
        self.bytes[0..4].copy_from_slice(&next_block_bytes);
        u32::try_from(start_block)
            .map_err(|_| DbfError::Invalid("memo block number overflows u32".into()))
    }
}

pub(super) fn find_memo_path(path: &Path) -> Option<std::path::PathBuf> {
    let stem = path.file_stem()?;
    let directory = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    for extension in ["fpt", "dbt"] {
        let candidate = path.with_extension(extension);
        if candidate.is_file() {
            return Some(candidate);
        }
        let candidate = path.with_extension(extension.to_ascii_uppercase());
        if candidate.is_file() {
            return Some(candidate);
        }
        let entries = fs::read_dir(directory).ok()?;
        if let Some(candidate) = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|candidate| {
                candidate.file_stem() == Some(stem)
                    && candidate
                        .extension()
                        .is_some_and(|value| value.eq_ignore_ascii_case(extension))
            })
        {
            return Some(candidate);
        }
    }
    None
}

pub(super) fn memo_index(bytes: &[u8], format: MemoFormat) -> Result<Option<u32>, DbfError> {
    if bytes.iter().all(|byte| matches!(byte, b' ' | b'\0')) {
        return Ok(None);
    }
    if bytes.len() == 4 {
        let block = if format == MemoFormat::FoxPro {
            u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
        } else {
            u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
        };
        return Ok(Some(block));
    }
    let end = bytes
        .iter()
        .rposition(|byte| !matches!(byte, b' ' | b'\0'))
        .map_or(0, |index| index + 1);
    if end == 0 {
        return Ok(None);
    }
    let text = std::str::from_utf8(&bytes[..end])
        .map_err(|_| DbfError::Invalid("memo block number is not ASCII".into()))?
        .trim_matches([' ', '\0']);
    text.parse::<u32>()
        .map(Some)
        .map_err(|_| DbfError::Invalid("memo block number is not an integer".into()))
}

pub(super) fn is_memo_field(field_type: u8) -> bool {
    field_type.eq_ignore_ascii_case(&b'M')
}

pub(super) fn is_sidecar_field(field: &FieldDescriptor) -> bool {
    is_memo_field(field.field_type)
        || matches!(field.field_type.to_ascii_uppercase(), b'G' | b'P' | b'W')
        || (field.field_type.eq_ignore_ascii_case(&b'B') && field.length != 8)
}

pub(super) fn storage_value_without_sidecar(
    value: &Value,
    field: &FieldDescriptor,
) -> Result<Value, DbfError> {
    let empty = if field.is_binary() {
        binary_value(value, field)?.is_empty()
    } else {
        value_text(value, field)?.is_empty()
    };
    if !empty {
        return Err(DbfError::Invalid(format!(
            "memo sidecar is missing for non-empty field {}",
            field.name
        )));
    }
    Ok(empty_memo_value(field))
}

pub(super) fn sidecar_update(
    value: &Value,
    field: &FieldDescriptor,
    memo_format: MemoFormat,
) -> Result<Option<MemoUpdate>, DbfError> {
    if !field.is_binary() {
        let text = value_text(value, field)?;
        return Ok((!text.is_empty()).then_some(MemoUpdate::Text(text)));
    }

    let bytes = binary_value(value, field)?;
    if bytes.is_empty() {
        return Ok(None);
    }
    if memo_format == MemoFormat::Dbase3
        && (bytes.last() == Some(&EOF_MARKER)
            || bytes
                .windows(2)
                .any(|pair| pair == [EOF_MARKER, EOF_MARKER]))
    {
        return Err(DbfError::Invalid(
            "dBASE III binary data cannot contain its 0x1a1a terminator".into(),
        ));
    }
    Ok(Some(MemoUpdate::Binary(bytes)))
}

pub(super) fn binary_value(value: &Value, field: &FieldDescriptor) -> Result<Vec<u8>, DbfError> {
    let Some(text) = value.as_str() else {
        if value.is_null() {
            return Ok(Vec::new());
        }
        return Err(DbfError::Invalid(format!(
            "binary field {} requires an even-length hexadecimal string or null",
            field.name
        )));
    };
    let bytes = text.as_bytes();
    if bytes.len() % 2 != 0 {
        return Err(DbfError::Invalid(format!(
            "binary field {} requires an even-length hexadecimal string",
            field.name
        )));
    }
    bytes
        .chunks_exact(2)
        .map(|pair| {
            let high = hex_digit(pair[0]);
            let low = hex_digit(pair[1]);
            match (high, low) {
                (Some(high), Some(low)) => Ok(high << 4 | low),
                _ => Err(DbfError::Invalid(format!(
                    "binary field {} contains a non-hexadecimal character",
                    field.name
                ))),
            }
        })
        .collect()
}

pub(super) fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

pub(super) fn empty_memo_value(field: &FieldDescriptor) -> Value {
    if field.length == 4 {
        Value::Number(0.into())
    } else {
        Value::String(String::new())
    }
}

pub(super) fn encode_memo_pointer(
    field: &FieldDescriptor,
    block: u32,
    format: MemoFormat,
) -> Result<Vec<u8>, DbfError> {
    if field.length == 4 {
        let bytes = if format == MemoFormat::FoxPro {
            block.to_be_bytes()
        } else {
            block.to_le_bytes()
        };
        return Ok(bytes.to_vec());
    }
    let text = block.to_string();
    let length = usize::from(field.length);
    if text.len() > length {
        return Err(DbfError::Invalid(format!(
            "memo block number for {} exceeds field width {}",
            field.name, field.length
        )));
    }
    let mut pointer = vec![b' '; length];
    let start = length - text.len();
    pointer[start..].copy_from_slice(text.as_bytes());
    Ok(pointer)
}
