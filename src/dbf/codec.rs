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

pub(super) use cjk::{canonical_encoding_name, encoding_name};
pub(super) use decode::{
    decode_field, decode_field_with_encoding, decode_record_field_with_encoding, hex, text,
    text_with_encoding,
};
pub(super) use encode::{
    encode_character_with_encoding, encode_field, encode_field_with_encoding, value_text,
};
pub(super) use fields::{
    encode_null_flags, flag_is_set, null_flag_layout, parse_fields, read_u16, read_u32,
    system_field_index, update_null_flags,
};
#[cfg(test)]
pub(super) use temporal::foxpro_datetime_bytes;
