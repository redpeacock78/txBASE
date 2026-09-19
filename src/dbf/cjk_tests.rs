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
