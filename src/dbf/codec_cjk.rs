use encoding_rs::{BIG5, EUC_KR, Encoding, GBK, SHIFT_JIS};

pub(super) fn decode(bytes: &[u8], language_driver: u8) -> Option<String> {
    let encoding = encoding(language_driver)?;
    Some(encoding.decode_without_bom_handling(bytes).0.into_owned())
}

pub(super) fn encode(text: &str, language_driver: u8) -> Option<(Vec<u8>, bool)> {
    let encoding = encoding(language_driver)?;
    let (bytes, _, had_errors) = encoding.encode(text);
    Some((bytes.into_owned(), had_errors))
}

pub(super) fn encoding_name(language_driver: u8) -> Option<&'static str> {
    Some(match language_driver {
        0x78 => "Big5/CP950",
        0x79 => "EUC-KR/CP949",
        0x7a => "GBK/CP936",
        0x7b => "Windows-31J/CP932",
        _ => return None,
    })
}

fn encoding(language_driver: u8) -> Option<&'static Encoding> {
    Some(match language_driver {
        0x78 => &BIG5,
        0x79 => &EUC_KR,
        0x7a => &GBK,
        0x7b => &SHIFT_JIS,
        _ => return None,
    })
}
