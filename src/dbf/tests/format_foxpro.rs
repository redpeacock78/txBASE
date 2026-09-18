use super::super::*;
use super::foxpro_variable_fixture;

#[test]
fn round_trips_visual_foxpro_variable_fields() {
    let mut table = DbfTable::from_bytes(&foxpro_variable_fixture()).unwrap();

    assert_eq!(
        table.active_json(),
        vec![serde_json::json!({"NAME": "abc", "_PAYLOAD": "00ff"})]
    );
    assert!(
        !table.active_json()[0]
            .as_object()
            .unwrap()
            .contains_key("_NullFlags")
    );

    table
        .patch_record(
            1,
            serde_json::json!({"NAME": "xy", "_PAYLOAD": "a1b2"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();
    let bytes = table.to_bytes();
    assert_eq!(&bytes[130..135], b"xy  \x02");
    assert_eq!(&bytes[135..140], &[0xa1, 0xb2, 0, 0, 2]);
    assert_eq!(bytes[140], 0x05);

    table
        .patch_record(
            1,
            serde_json::json!({"NAME": null, "_PAYLOAD": null})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();
    assert_eq!(
        table.active_json(),
        vec![serde_json::json!({"NAME": null, "_PAYLOAD": null})]
    );
    assert_eq!(&table.to_bytes()[130..140], &[0; 10]);
    assert_eq!(table.to_bytes()[140], 0x0f);

    assert_eq!(
        table
            .insert_record(
                serde_json::json!({"NAME": "new", "_PAYLOAD": "cafe"})
                    .as_object()
                    .unwrap()
                    .clone(),
            )
            .unwrap(),
        2
    );
    let second = 129 + 12;
    let bytes = table.to_bytes();
    assert_eq!(&bytes[second + 1..second + 6], b"new \x03");
    assert_eq!(&bytes[second + 6..second + 11], &[0xca, 0xfe, 0, 0, 2]);
    assert_eq!(bytes[second + 11], 0x05);
}

#[test]
fn reads_visual_foxpro_nullable_fixed_fields() {
    for version in [0x30, 0x31] {
        let mut bytes = vec![0; 101];
        bytes[0] = version;
        bytes[4..8].copy_from_slice(&1u32.to_le_bytes());
        bytes[8..10].copy_from_slice(&97u16.to_le_bytes());
        bytes[10..12].copy_from_slice(&3u16.to_le_bytes());
        bytes[32..36].copy_from_slice(b"NAME");
        bytes[43] = b'C';
        bytes[48] = 1;
        bytes[50] = 0x02;
        bytes[64..74].copy_from_slice(b"_NullFlags");
        bytes[80] = 1;
        bytes[82] = 0x01;
        bytes[96] = FIELD_TERMINATOR;
        bytes[97] = ACTIVE_RECORD;
        bytes[98] = b'A';
        bytes[99] = 0x01;
        bytes[100] = EOF_MARKER;

        assert_eq!(
            DbfTable::from_bytes(&bytes).unwrap().active_json(),
            vec![serde_json::json!({"NAME": null})]
        );
    }
}

#[test]
fn round_trips_visual_foxpro_binary_character_fields() {
    let mut bytes = vec![0; 71];
    bytes[0] = 0x32;
    bytes[4..8].copy_from_slice(&1u32.to_le_bytes());
    bytes[8..10].copy_from_slice(&65u16.to_le_bytes());
    bytes[10..12].copy_from_slice(&5u16.to_le_bytes());
    bytes[32..35].copy_from_slice(b"RAW");
    bytes[43] = b'C';
    bytes[48] = 4;
    bytes[50] = 0x04;
    bytes[64] = FIELD_TERMINATOR;
    bytes[65] = ACTIVE_RECORD;
    bytes[66..70].copy_from_slice(&[0, 0xff, 0, 0]);
    bytes[70] = EOF_MARKER;

    let mut table = DbfTable::from_bytes(&bytes).unwrap();
    assert_eq!(table.active_json()[0]["RAW"], "00ff0000");
    table
        .patch_record(
            1,
            serde_json::json!({"RAW": "a1b2"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();
    assert_eq!(&table.to_bytes()[66..70], &[0xa1, 0xb2, 0, 0]);
}

#[test]
fn assigns_level7_auto_increment_values_on_insert() {
    let mut bytes = vec![0; 123];
    bytes[0] = 0x04;
    bytes[4..8].copy_from_slice(&1u32.to_le_bytes());
    bytes[8..10].copy_from_slice(&117u16.to_le_bytes());
    bytes[10..12].copy_from_slice(&5u16.to_le_bytes());
    bytes[68..72].copy_from_slice(b"AUTO");
    bytes[100] = b'+';
    bytes[101] = 4;
    bytes[108..112].copy_from_slice(&7u32.to_le_bytes());
    bytes[116] = FIELD_TERMINATOR;
    bytes[117] = ACTIVE_RECORD;
    bytes[122] = EOF_MARKER;

    let mut table = DbfTable::from_bytes(&bytes).unwrap();
    assert_eq!(table.insert_record(Map::new()).unwrap(), 2);
    assert_eq!(table.active_record(2).unwrap().values["AUTO"], 7);
    assert_eq!(
        u32::from_le_bytes(table.to_bytes()[108..112].try_into().unwrap()),
        8
    );

    let before = table.to_bytes();
    let error = table
        .insert_record(serde_json::json!({"AUTO": 99}).as_object().unwrap().clone())
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("auto-increment field AUTO is read-only")
    );
    assert_eq!(table.to_bytes(), before);

    table.replace_record(2, Map::new()).unwrap();
    assert_eq!(table.active_record(2).unwrap().values["AUTO"], 7);
    let error = table
        .patch_record(
            2,
            serde_json::json!({"AUTO": 8}).as_object().unwrap().clone(),
        )
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("auto-increment field AUTO is read-only")
    );
    assert_eq!(table.active_record(2).unwrap().values["AUTO"], 7);
}

#[test]
fn assigns_visual_foxpro_auto_increment_values_on_insert() {
    let mut bytes = vec![0; 71];
    bytes[0] = 0x31;
    bytes[4..8].copy_from_slice(&1u32.to_le_bytes());
    bytes[8..10].copy_from_slice(&65u16.to_le_bytes());
    bytes[10..12].copy_from_slice(&5u16.to_le_bytes());
    bytes[32..36].copy_from_slice(b"AUTO");
    bytes[43] = b'I';
    bytes[48] = 4;
    bytes[50] = 0x0c;
    bytes[51..55].copy_from_slice(&7i32.to_le_bytes());
    bytes[55] = 3;
    bytes[64] = FIELD_TERMINATOR;
    bytes[65] = ACTIVE_RECORD;
    bytes[66..70].copy_from_slice(&4i32.to_le_bytes());
    bytes[70] = EOF_MARKER;

    let mut table = DbfTable::from_bytes(&bytes).unwrap();
    assert_eq!(table.active_record(1).unwrap().values["AUTO"], 4);
    assert_eq!(table.insert_record(Map::new()).unwrap(), 2);
    assert_eq!(table.active_record(2).unwrap().values["AUTO"], 10);
    assert_eq!(
        i32::from_le_bytes(table.to_bytes()[51..55].try_into().unwrap()),
        10
    );

    let before = table.to_bytes();
    let error = table
        .insert_record(serde_json::json!({"AUTO": 99}).as_object().unwrap().clone())
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("auto-increment field AUTO is read-only")
    );
    assert_eq!(table.to_bytes(), before);
}
