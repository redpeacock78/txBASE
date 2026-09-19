use super::*;

fn fixture() -> Vec<u8> {
    include_str!("../../../tests/fixtures/users.dbf.hex")
        .split_whitespace()
        .map(|token| u8::from_str_radix(token, 16).unwrap())
        .collect()
}

fn foxpro_variable_fixture() -> Vec<u8> {
    let mut bytes = vec![0; 142];
    bytes[0] = 0x32;
    bytes[4..8].copy_from_slice(&1u32.to_le_bytes());
    bytes[8..10].copy_from_slice(&129u16.to_le_bytes());
    bytes[10..12].copy_from_slice(&12u16.to_le_bytes());

    bytes[32..36].copy_from_slice(b"NAME");
    bytes[43] = b'V';
    bytes[48] = 5;
    bytes[50] = 0x02;

    bytes[64..72].copy_from_slice(b"_PAYLOAD");
    bytes[75] = b'Q';
    bytes[80] = 5;
    bytes[82] = 0x02;

    bytes[96..106].copy_from_slice(b"_NullFlags");
    bytes[107] = 0;
    bytes[112] = 1;
    bytes[114] = 1;

    bytes[128] = FIELD_TERMINATOR;
    bytes[129] = ACTIVE_RECORD;
    bytes[130..133].copy_from_slice(b"abc");
    bytes[133] = b' ';
    bytes[134] = 3;
    bytes[135..137].copy_from_slice(&[0, 0xff]);
    bytes[139] = 2;
    bytes[140] = 0x05;
    bytes[141] = EOF_MARKER;
    bytes
}

mod format;
#[path = "format_codepages.rs"]
mod format_codepages;
#[path = "format_foxpro.rs"]
mod format_foxpro;
mod maintenance;
mod memo;
#[path = "memo_foxpro.rs"]
mod memo_foxpro;
mod mutation;
mod persistence;
