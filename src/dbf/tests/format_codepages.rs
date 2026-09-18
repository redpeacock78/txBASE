use super::super::*;
use super::fixture;

#[test]
fn decodes_and_encodes_windows_1252_character_fields() {
    let mut bytes = fixture();
    bytes[29] = 0x03;
    let record_start = usize::from(u16::from_le_bytes([bytes[8], bytes[9]]));
    let name_start = record_start + 4;
    bytes[name_start..name_start + 10].fill(b' ');
    bytes[name_start] = 0xe9;

    let mut table = DbfTable::from_bytes(&bytes).unwrap();
    assert_eq!(table.active_record(1).unwrap().values["NAME"], "é");

    table
        .patch_record(
            1,
            serde_json::json!({"NAME": "€"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();
    assert_eq!(table.to_bytes()[name_start], 0x80);
    assert_eq!(
        DbfTable::from_bytes(&table.to_bytes())
            .unwrap()
            .active_record(1)
            .unwrap()
            .values["NAME"],
        "€"
    );

    let error = table
        .patch_record(
            1,
            serde_json::json!({"NAME": "漢"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap_err();
    assert!(error.to_string().contains("outside Windows-1252"));
}

#[test]
fn decodes_and_encodes_windows_character_fields() {
    let record_start = usize::from(u16::from_le_bytes([fixture()[8], fixture()[9]]));
    let name_start = record_start + 4;
    for (language_driver, raw, value, code_page) in [
        (0xc8, 0x8a, "Š", "Windows-1250"),
        (0xc9, 0xdf, "Я", "Windows-1251"),
        (0xca, 0xdd, "İ", "Windows-1254"),
        (0xcb, 0xd9, "Ω", "Windows-1253"),
        (0x7d, 0xe9, "י", "Windows-1255"),
        (0x7e, 0xc7, "ا", "Windows-1256"),
    ] {
        let mut bytes = fixture();
        bytes[29] = language_driver;
        bytes[name_start..name_start + 10].fill(b' ');
        bytes[name_start] = raw;
        let mut table = DbfTable::from_bytes(&bytes).unwrap();
        assert_eq!(table.active_record(1).unwrap().values["NAME"], value);

        table
            .patch_record(
                1,
                serde_json::json!({"NAME": value})
                    .as_object()
                    .unwrap()
                    .clone(),
            )
            .unwrap();
        assert_eq!(table.to_bytes()[name_start], raw);
        assert_eq!(
            DbfTable::from_bytes(&table.to_bytes())
                .unwrap()
                .active_record(1)
                .unwrap()
                .values["NAME"],
            value
        );

        let error = table
            .patch_record(
                1,
                serde_json::json!({"NAME": "漢"})
                    .as_object()
                    .unwrap()
                    .clone(),
            )
            .unwrap_err();
        assert!(error.to_string().contains(&format!("outside {code_page}")));
    }
}

#[test]
fn decodes_and_encodes_oem_character_fields() {
    let mut bytes = fixture();
    let record_start = usize::from(u16::from_le_bytes([bytes[8], bytes[9]]));
    let name_start = record_start + 4;

    bytes[29] = 0x01;
    bytes[name_start..name_start + 10].fill(b' ');
    bytes[name_start] = 0x82;
    let mut table = DbfTable::from_bytes(&bytes).unwrap();
    assert_eq!(table.active_record(1).unwrap().values["NAME"], "é");
    table
        .patch_record(
            1,
            serde_json::json!({"NAME": "é"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();
    assert_eq!(table.to_bytes()[name_start], 0x82);

    bytes[29] = 0x02;
    bytes[name_start..name_start + 10].fill(b' ');
    bytes[name_start] = 0x9b;
    let mut table = DbfTable::from_bytes(&bytes).unwrap();
    assert_eq!(table.active_record(1).unwrap().values["NAME"], "ø");
    table
        .patch_record(
            1,
            serde_json::json!({"NAME": "ø"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();
    assert_eq!(table.to_bytes()[name_start], 0x9b);

    bytes[29] = 0x64;
    bytes[name_start..name_start + 10].fill(b' ');
    bytes[name_start] = 0x88;
    let mut table = DbfTable::from_bytes(&bytes).unwrap();
    assert_eq!(table.active_record(1).unwrap().values["NAME"], "ł");
    table
        .patch_record(
            1,
            serde_json::json!({"NAME": "ł"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();
    assert_eq!(table.to_bytes()[name_start], 0x88);

    bytes[29] = 0x65;
    bytes[name_start..name_start + 10].fill(b' ');
    bytes[name_start] = 0x9f;
    let mut table = DbfTable::from_bytes(&bytes).unwrap();
    assert_eq!(table.active_record(1).unwrap().values["NAME"], "Я");
    table
        .patch_record(
            1,
            serde_json::json!({"NAME": "Я"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();
    assert_eq!(table.to_bytes()[name_start], 0x9f);

    let error = table
        .patch_record(
            1,
            serde_json::json!({"NAME": "漢"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap_err();
    assert!(error.to_string().contains("outside CP866"));
}
