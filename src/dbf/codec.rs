#[path = "codec_decode.rs"]
mod decode;
#[path = "codec_encode.rs"]
mod encode;
#[path = "codec_fields.rs"]
mod fields;
#[path = "codec_temporal.rs"]
mod temporal;

pub(super) use decode::{decode_field, decode_record_field, hex, text};
pub(super) use encode::{encode_character, encode_field, value_text};
pub(super) use fields::{
    encode_null_flags, flag_is_set, null_flag_layout, parse_fields, read_u16, read_u32,
    system_field_index, update_null_flags,
};
#[cfg(test)]
pub(super) use temporal::foxpro_datetime_bytes;
