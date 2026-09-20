use super::super::{CLASSIC_DESCRIPTOR_SIZE, DbfError, FieldDescriptor, NullFlagBits};
use super::decode::text_with_encoding;
use serde_json::{Map, Value};
use std::collections::BTreeSet;

pub fn parse_fields(
    bytes: &[u8],
    start: usize,
    end: usize,
    descriptor_size: usize,
    language_driver: u8,
    encoding_override: Option<&str>,
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
        let name = text_with_encoding(&descriptor[..name_end], language_driver, encoding_override)
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

pub fn null_flag_layout(fields: &[FieldDescriptor]) -> Vec<Option<NullFlagBits>> {
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

pub fn system_field_index(fields: &[FieldDescriptor]) -> Option<usize> {
    fields.iter().position(FieldDescriptor::is_system)
}

pub fn flag_is_set(flags: &[u8], bit: usize) -> bool {
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

pub fn encode_null_flags(
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

pub fn update_null_flags(
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

pub fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, DbfError> {
    let bytes = bytes
        .get(offset..offset + 2)
        .ok_or_else(|| DbfError::Invalid("header is truncated".into()))?;
    Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
}

pub fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, DbfError> {
    let bytes = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| DbfError::Invalid("header is truncated".into()))?;
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}
