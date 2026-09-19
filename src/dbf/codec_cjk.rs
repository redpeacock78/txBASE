use encoding_rs::{BIG5, EUC_JP, EUC_KR, Encoding, GB18030, GBK, SHIFT_JIS};

pub(super) fn decode(
    bytes: &[u8],
    language_driver: u8,
    encoding_override: Option<&str>,
) -> Option<String> {
    let encoding = encoding(language_driver, encoding_override)?;
    Some(encoding.decode_without_bom_handling(bytes).0.into_owned())
}

pub(super) fn encode(
    text: &str,
    language_driver: u8,
    encoding_override: Option<&str>,
) -> Option<(Vec<u8>, bool)> {
    let encoding = encoding(language_driver, encoding_override)?;
    let (bytes, _, had_errors) = encoding.encode(text);
    Some((bytes.into_owned(), had_errors))
}

pub(crate) fn canonical_encoding_name(name: &str) -> Option<&'static str> {
    match name.to_ascii_lowercase().as_str() {
        "windows-31j" | "cp932" | "windows-31j/cp932" => Some("Windows-31J/CP932"),
        "gbk" | "cp936" | "gbk/cp936" => Some("GBK/CP936"),
        "gb18030" => Some("GB18030"),
        "euc-kr" | "cp949" | "euc-kr/cp949" => Some("EUC-KR/CP949"),
        "euc-jp" => Some("EUC-JP"),
        "big5" | "cp950" | "big5/cp950" => Some("Big5/CP950"),
        _ => None,
    }
}

pub(crate) fn encoding_name(language_driver: u8) -> Option<&'static str> {
    Some(match language_driver {
        0x78 => "Big5/CP950",
        0x79 => "EUC-KR/CP949",
        0x7a => "GBK/CP936",
        0x7b => "Windows-31J/CP932",
        _ => return None,
    })
}

fn encoding(language_driver: u8, encoding_override: Option<&str>) -> Option<&'static Encoding> {
    if let Some(encoding_override) = encoding_override {
        return Some(match canonical_encoding_name(encoding_override)? {
            "Big5/CP950" => BIG5,
            "EUC-KR/CP949" => EUC_KR,
            "EUC-JP" => EUC_JP,
            "GB18030" => GB18030,
            "GBK/CP936" => GBK,
            "Windows-31J/CP932" => SHIFT_JIS,
            _ => return None,
        });
    }
    Some(match language_driver {
        0x78 => BIG5,
        0x79 => EUC_KR,
        0x7a => GBK,
        0x7b => SHIFT_JIS,
        _ => return None,
    })
}
