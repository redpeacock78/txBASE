use super::*;

fn decode_hex_fixture(input: &str) -> Vec<u8> {
    input
        .split_whitespace()
        .map(|token| u8::from_str_radix(token, 16).unwrap())
        .collect()
}

fn fixture() -> Vec<u8> {
    decode_hex_fixture(include_str!("../../tests/fixtures/users.dbf.hex"))
}

fn assert_external_cjk_fixture(input: &str, language_driver: u8, expected: &str) {
    let bytes = decode_hex_fixture(input);
    assert_eq!(bytes[29], language_driver);
    let mut table = DbfTable::from_bytes(&bytes).unwrap();
    assert_eq!(table.active_record(1).unwrap().values["NAME"], expected);
    table
        .patch_record(
            1,
            serde_json::json!({"NAME": expected})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();
    let reloaded = DbfTable::from_bytes(&table.to_bytes()).unwrap();
    assert_eq!(reloaded.active_record(1).unwrap().values["NAME"], expected);
}

#[test]
fn reads_and_writes_pinned_cjk_driver_fixtures() {
    assert_external_cjk_fixture(
        include_str!("../../tests/fixtures/cjk-cp932.dbf.hex"),
        0x7b,
        "日本",
    );
    assert_external_cjk_fixture(
        include_str!("../../tests/fixtures/cjk-gbk.dbf.hex"),
        0x7a,
        "中文",
    );
    assert_external_cjk_fixture(
        include_str!("../../tests/fixtures/cjk-euc-kr.dbf.hex"),
        0x79,
        "한국",
    );
    assert_external_cjk_fixture(
        include_str!("../../tests/fixtures/cjk-big5.dbf.hex"),
        0x78,
        "中文",
    );
}

#[test]
fn decodes_and_encodes_declared_cjk_drivers() {
    for (language_driver, value) in [
        (0x7b, "日本"),
        (0x7a, "中文"),
        (0x79, "한국"),
        (0x78, "中文"),
    ] {
        let mut bytes = fixture();
        bytes[29] = language_driver;
        let mut table = DbfTable::from_bytes(&bytes).unwrap();
        assert!(table.schema_json()["encoding"].is_string());
        assert_eq!(
            table.schema_json()["encoding_metadata"]["declared"],
            table.schema_json()["encoding"]
        );
        assert_eq!(
            table.schema_json()["encoding_metadata"]["effective"],
            table.schema_json()["encoding"]
        );
        assert_eq!(
            table.schema_json()["encoding_metadata"]["source"],
            "language-driver"
        );
        table
            .patch_record(
                1,
                serde_json::json!({"NAME": value})
                    .as_object()
                    .unwrap()
                    .clone(),
            )
            .unwrap();
        assert_eq!(
            DbfTable::from_bytes(&table.to_bytes())
                .unwrap()
                .active_record(1)
                .unwrap()
                .values["NAME"],
            value
        );
    }
}

#[test]
fn explicit_euc_jp_and_gb18030_overrides_round_trip() {
    for (canonical, value) in [("EUC-JP", "日本"), ("GB18030", "中文")] {
        let mut table = DbfTable::from_bytes_with_encoding(&fixture(), Some(canonical)).unwrap();
        assert_eq!(table.schema_json()["encoding_override"], canonical);
        assert_eq!(
            table.schema_json()["encoding_metadata"],
            serde_json::json!({
                "declared": null,
                "effective": canonical,
                "source": "explicit-override",
            })
        );
        table
            .patch_record(
                1,
                serde_json::json!({"NAME": value})
                    .as_object()
                    .unwrap()
                    .clone(),
            )
            .unwrap();
        let reloaded =
            DbfTable::from_bytes_with_encoding(&table.to_bytes(), Some(canonical)).unwrap();
        assert_eq!(reloaded.active_record(1).unwrap().values["NAME"], value);
    }
}

#[test]
fn reads_pinned_explicit_cjk_codec_bytes_from_a_dbf_record() {
    let source = DbfTable::from_bytes(&fixture()).unwrap();
    let name_field = source
        .fields
        .iter()
        .find(|field| field.name == "NAME")
        .unwrap();
    let record_start = usize::from(source.header.header_length);
    let name_start = record_start + name_field.offset;
    let name_end = name_start + usize::from(name_field.length);
    let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "../../tests/fixtures/cjk-explicit-codecs.json"
    ))
    .unwrap();

    for case in cases {
        let encoding = case["encoding"].as_str().unwrap();
        let expected = case["value"].as_str().unwrap();
        let encoded = decode_hex_fixture(case["bytes"].as_str().unwrap());
        assert!(encoded.len() <= name_field.length as usize);

        let mut bytes = fixture();
        bytes[29] = 0;
        bytes[name_start..name_end].fill(b' ');
        bytes[name_start..name_start + encoded.len()].copy_from_slice(&encoded);

        let mut table = DbfTable::from_bytes_with_encoding(&bytes, Some(encoding)).unwrap();
        assert_eq!(table.active_record(1).unwrap().values["NAME"], expected);
        table
            .patch_record(
                1,
                serde_json::json!({"NAME": expected})
                    .as_object()
                    .unwrap()
                    .clone(),
            )
            .unwrap();
        assert_eq!(
            &table.to_bytes()[name_start..name_start + encoded.len()],
            encoded.as_slice()
        );
    }
}

#[test]
fn explicit_iso_2022_jp_override_round_trips_jis_text() {
    let mut table = DbfTable::from_bytes_with_encoding(&fixture(), Some("ISO-2022-JP")).unwrap();
    assert_eq!(table.schema_json()["encoding_override"], "ISO-2022-JP");
    assert_eq!(
        table.schema_json()["encoding_metadata"],
        serde_json::json!({
            "declared": null,
            "effective": "ISO-2022-JP",
            "source": "explicit-override",
        })
    );
    table
        .patch_record(
            1,
            serde_json::json!({"NAME": "日本"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();

    let reloaded =
        DbfTable::from_bytes_with_encoding(&table.to_bytes(), Some("ISO-2022-JP")).unwrap();
    assert_eq!(reloaded.active_record(1).unwrap().values["NAME"], "日本");
}

#[test]
fn iso_2022_jp_decodes_a_pinned_jis_fixture_and_replaces_truncated_sequences() {
    assert_eq!(
        text_with_encoding(
            &[0x1b, 0x24, 0x42, 0x46, 0x7c, 0x4b, 0x5c, 0x1b, 0x28, 0x42],
            0,
            Some("ISO-2022-JP"),
        ),
        "日本"
    );
    assert_eq!(
        text_with_encoding(&[0x1b, 0x24, 0x42, 0x46], 0, Some("ISO-2022-JP")),
        "�"
    );
}

#[test]
fn strict_shift_jis_override_round_trips_jis_text() {
    let mut table = DbfTable::from_bytes_with_encoding(&fixture(), Some("Shift_JIS")).unwrap();
    assert_eq!(table.schema_json()["encoding_override"], "Shift_JIS");
    assert_eq!(
        table.schema_json()["encoding_metadata"],
        serde_json::json!({
            "declared": null,
            "effective": "Shift_JIS",
            "source": "explicit-override",
        })
    );
    table
        .patch_record(
            1,
            serde_json::json!({"NAME": "日本"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();

    let reloaded =
        DbfTable::from_bytes_with_encoding(&table.to_bytes(), Some("Shift_JIS")).unwrap();
    assert_eq!(reloaded.active_record(1).unwrap().values["NAME"], "日本");
}

#[test]
fn strict_shift_jis_rejects_cp932_extensions() {
    let mut table = DbfTable::from_bytes_with_encoding(&fixture(), Some("shift-jis")).unwrap();
    let error = table
        .patch_record(
            1,
            serde_json::json!({"NAME": "ⅰ"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap_err();
    assert!(error.to_string().contains("outside Shift_JIS"));

    assert_eq!(
        text_with_encoding(&[0xfa, 0x40], 0, Some("Shift_JIS")),
        "\u{fffd}"
    );
}

#[test]
fn rejects_unrepresentable_or_overwide_cjk_values() {
    let mut bytes = fixture();
    bytes[29] = 0x7b;
    let mut table = DbfTable::from_bytes(&bytes).unwrap();

    let error = table
        .patch_record(
            1,
            serde_json::json!({"NAME": "😀"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap_err();
    assert!(error.to_string().contains("outside Windows-31J/CP932"));

    let error = table
        .patch_record(
            1,
            serde_json::json!({"NAME": "日本語文字列"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap_err();
    assert!(error.to_string().contains("exceeds field width"));
}

#[test]
fn malformed_declared_cjk_bytes_decode_as_replacement() {
    assert_eq!(text(&[0x82], 0x7b), "\u{fffd}");
}
