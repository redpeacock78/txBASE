use super::*;

fn fixture() -> Vec<u8> {
    include_str!("../../tests/fixtures/users.dbf.hex")
        .split_whitespace()
        .map(|token| u8::from_str_radix(token, 16).unwrap())
        .collect()
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
