use encoding_rs::{BIG5, EUC_JP, EUC_KR, Encoding, GB18030, GBK, ISO_2022_JP, SHIFT_JIS};

const STRICT_SHIFT_JIS: &str = "Shift_JIS";

pub(super) fn decode(
    bytes: &[u8],
    language_driver: u8,
    encoding_override: Option<&str>,
) -> Option<String> {
    if is_strict_shift_jis(encoding_override) {
        return Some(decode_strict_shift_jis(bytes));
    }
    let encoding = encoding(language_driver, encoding_override)?;
    Some(encoding.decode_without_bom_handling(bytes).0.into_owned())
}

pub(super) fn encode(
    text: &str,
    language_driver: u8,
    encoding_override: Option<&str>,
) -> Option<(Vec<u8>, bool)> {
    if is_strict_shift_jis(encoding_override) {
        return Some(match encode_strict_shift_jis(text) {
            Some(bytes) => (bytes, false),
            None => (Vec::new(), true),
        });
    }
    let encoding = encoding(language_driver, encoding_override)?;
    let (bytes, _, had_errors) = encoding.encode(text);
    Some((bytes.into_owned(), had_errors))
}

pub(crate) fn canonical_encoding_name(name: &str) -> Option<&'static str> {
    match name.to_ascii_lowercase().as_str() {
        "windows-31j" | "cp932" | "windows-31j/cp932" => Some("Windows-31J/CP932"),
        "shift_jis" | "shift-jis" | "sjis" => Some(STRICT_SHIFT_JIS),
        "gbk" | "cp936" | "gbk/cp936" => Some("GBK/CP936"),
        "gb18030" => Some("GB18030"),
        "euc-kr" | "cp949" | "euc-kr/cp949" => Some("EUC-KR/CP949"),
        "euc-jp" => Some("EUC-JP"),
        "iso-2022-jp" | "iso2022-jp" => Some("ISO-2022-JP"),
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

fn is_strict_shift_jis(encoding_override: Option<&str>) -> bool {
    encoding_override
        .and_then(canonical_encoding_name)
        .is_some_and(|name| name == STRICT_SHIFT_JIS)
}

fn decode_strict_shift_jis(bytes: &[u8]) -> String {
    let mut output = String::new();
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte <= 0x7f {
            output.push(char::from(byte));
            index += 1;
            continue;
        }
        if (0xa1..=0xdf).contains(&byte) {
            output.push(
                char::from_u32(0xff61 + u32::from(byte - 0xa1)).expect("half-width katakana"),
            );
            index += 1;
            continue;
        }

        let is_pair = is_shift_jis_lead(byte)
            && bytes
                .get(index + 1)
                .is_some_and(|trail| is_shift_jis_trail(*trail));
        if !is_pair {
            output.push('\u{fffd}');
            index += 1;
            continue;
        }

        let pair = &bytes[index..index + 2];
        let (decoded, had_errors) = SHIFT_JIS.decode_without_bom_handling(pair);
        let mut characters = decoded.chars();
        let character = match (characters.next(), characters.next()) {
            (Some(character), None) => Some(character),
            _ => None,
        };
        if !had_errors
            && character
                .and_then(encode_strict_shift_jis_character)
                .is_some_and(|encoded| encoded.as_slice() == pair)
        {
            output.push(character.expect("checked strict Shift_JIS character"));
        } else {
            output.push('\u{fffd}');
        }
        index += 2;
    }
    output
}

fn encode_strict_shift_jis(text: &str) -> Option<Vec<u8>> {
    let mut output = Vec::new();
    for character in text.chars() {
        output.extend(encode_strict_shift_jis_character(character)?);
    }
    Some(output)
}

fn encode_strict_shift_jis_character(character: char) -> Option<Vec<u8>> {
    if character <= '\u{7f}' {
        return Some(vec![character as u8]);
    }
    if ('\u{ff61}'..='\u{ff9f}').contains(&character) {
        return Some(vec![(0xa1 + (character as u32 - 0xff61)) as u8]);
    }

    let character = character.to_string();
    let (euc, _, euc_errors) = EUC_JP.encode(&character);
    if euc_errors || euc.len() != 2 || !euc.iter().all(|byte| (0xa1..=0xfe).contains(byte)) {
        return None;
    }
    let expected = shift_jis_pair_from_euc(&euc)?;
    let (shift_jis, _, shift_jis_errors) = SHIFT_JIS.encode(&character);
    if shift_jis_errors || shift_jis.as_ref() != expected.as_slice() {
        return None;
    }
    Some(shift_jis.into_owned())
}

fn shift_jis_pair_from_euc(euc: &[u8]) -> Option<[u8; 2]> {
    if euc.len() != 2 || euc[0] >= 0xf9 {
        return None;
    }
    let row = euc[0].checked_sub(0xa1)?;
    let cell = euc[1].checked_sub(0xa1)?;
    let mut lead = row / 2 + 0x81;
    if lead >= 0xa0 {
        lead += 0x40;
    }
    let trail = if row % 2 == 0 {
        let mut trail = cell + 0x40;
        if trail >= 0x7f {
            trail += 1;
        }
        trail
    } else {
        cell + 0x9f
    };
    Some([lead, trail])
}

fn is_shift_jis_lead(byte: u8) -> bool {
    (0x81..=0x9f).contains(&byte) || (0xe0..=0xfc).contains(&byte)
}

fn is_shift_jis_trail(byte: u8) -> bool {
    (0x40..=0x7e).contains(&byte) || (0x80..=0xfc).contains(&byte)
}

fn encoding(language_driver: u8, encoding_override: Option<&str>) -> Option<&'static Encoding> {
    if let Some(encoding_override) = encoding_override {
        return Some(match canonical_encoding_name(encoding_override)? {
            "Big5/CP950" => BIG5,
            "EUC-KR/CP949" => EUC_KR,
            "EUC-JP" => EUC_JP,
            "GB18030" => GB18030,
            "GBK/CP936" => GBK,
            "ISO-2022-JP" => ISO_2022_JP,
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
