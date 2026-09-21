#[path = "codec_cjk.rs"]
mod cjk;
#[path = "codec_decode.rs"]
mod decode;
#[path = "codec_encode.rs"]
mod encode;
#[path = "codec_fields.rs"]
mod fields;
#[path = "codec_temporal.rs"]
mod temporal;
#[path = "codec_values.rs"]
mod values;

pub(super) use cjk::canonical_encoding_name;
#[cfg(test)]
pub(super) use decode::{decode_field, text};
pub(super) use decode::{
    decode_field_with_encoding, decode_record_field_with_encoding, hex, text_with_encoding,
};
#[cfg(test)]
pub(super) use encode::encode_field;
pub(super) use encode::{encode_character_with_encoding, encode_field_with_encoding};
pub(super) use fields::{
    encode_null_flags, flag_is_set, null_flag_layout, parse_fields, read_u16, read_u32,
    system_field_index, update_null_flags,
};
#[cfg(test)]
pub(super) use temporal::foxpro_datetime_bytes;
pub(super) use values::value_text;

pub(super) fn encoding_name(language_driver: u8) -> Option<&'static str> {
    Some(match language_driver {
        0x01 => "CP437",
        0x02 => "CP850",
        0x1f | 0x22 | 0x23 | 0x40 | 0x64 | 0x87 => "CP852",
        0x26 | 0x65 => "CP866",
        0x03 | 0x57 => "Windows-1252",
        0xc8 => "Windows-1250",
        0xc9 => "Windows-1251",
        0xca => "Windows-1254",
        0xcb => "Windows-1253",
        0x7d => "Windows-1255",
        0x7e => "Windows-1256",
        0x4f | 0x78 => "Big5/CP950",
        0x4e | 0x79 => "EUC-KR/CP949",
        0x4d | 0x7a => "GBK/CP936",
        0x13 | 0x7b => "Windows-31J/CP932",
        _ => return None,
    })
}
