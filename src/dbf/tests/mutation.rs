use super::super::*;
use super::fixture;

#[test]
fn mutates_records_and_round_trips_to_dbf() {
    let mut table = DbfTable::from_bytes(&fixture()).unwrap();
    let inserted = table
        .insert_record(
            serde_json::json!({
                "ID": 3,
                "NAME": "Carol",
                "AGE": 42,
                "ACTIVE": false
            })
            .as_object()
            .unwrap()
            .clone(),
        )
        .unwrap();
    assert_eq!(inserted, 3);

    table
        .patch_record(
            1,
            serde_json::json!({"NAME": "Alicia"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();
    table
        .replace_record(
            3,
            serde_json::json!({
                "ID": 3,
                "NAME": "Carol",
                "AGE": 43,
                "ACTIVE": true
            })
            .as_object()
            .unwrap()
            .clone(),
        )
        .unwrap();
    table.delete_record(1).unwrap();

    let round_trip = DbfTable::from_bytes(&table.to_bytes()).unwrap();
    assert_eq!(round_trip.header.record_count, 3);
    assert!(round_trip.records()[0].deleted);
    assert_eq!(round_trip.active_record(3).unwrap().values["NAME"], "Carol");
    assert_eq!(round_trip.active_record(3).unwrap().values["AGE"], 43);
}

#[test]
fn rejects_unknown_mutation_fields_without_changing_table() {
    let mut table = DbfTable::from_bytes(&fixture()).unwrap();
    let before = table.to_bytes();
    let error = table
        .patch_record(
            1,
            serde_json::json!({"UNKNOWN": true})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap_err();

    assert!(error.to_string().contains("unknown field UNKNOWN"));
    assert_eq!(table.to_bytes(), before);
}

#[test]
fn rejects_nonempty_sidecar_mutations_without_sidecar() {
    let mut bytes = fixture();
    bytes[64 + 11] = b'M';
    let mut table = DbfTable::from_bytes(&bytes).unwrap();
    let before = table.to_bytes();

    let error = table
        .patch_record(
            1,
            serde_json::json!({"NAME": "new memo"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("memo sidecar is missing for non-empty field NAME")
    );
    assert_eq!(table.to_bytes(), before);

    let error = table
        .insert_record(
            serde_json::json!({
                "ID": 3,
                "NAME": "new memo",
                "AGE": 42,
                "ACTIVE": false
            })
            .as_object()
            .unwrap()
            .clone(),
        )
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("memo sidecar is missing for non-empty field NAME")
    );
    assert_eq!(table.to_bytes(), before);
}

#[test]
fn preserves_unresolved_sidecar_pointers_on_other_mutations() {
    let mut bytes = fixture();
    bytes[64 + 11] = b'M';
    let record_start = usize::from(u16::from_le_bytes([bytes[8], bytes[9]]));
    let name_start = record_start + 4;
    bytes[name_start..name_start + 10].fill(0xff);

    let mut table = DbfTable::from_bytes(&bytes).unwrap();
    table
        .patch_record(
            1,
            serde_json::json!({"AGE": 30}).as_object().unwrap().clone(),
        )
        .unwrap();

    assert_eq!(
        &table.to_bytes()[name_start..name_start + 10],
        &bytes[name_start..name_start + 10]
    );
    assert_eq!(table.active_record(1).unwrap().values["AGE"], 30);
}

#[test]
fn preserves_opaque_fields_on_other_mutations() {
    let mut bytes = fixture();
    bytes[64 + 11] = b'Z';
    let record_start = usize::from(u16::from_le_bytes([bytes[8], bytes[9]]));
    let field_start = record_start + 4;
    let raw = bytes[field_start..field_start + 10].to_vec();
    let mut table = DbfTable::from_bytes(&bytes).unwrap();

    table
        .patch_record(
            1,
            serde_json::json!({"AGE": 30}).as_object().unwrap().clone(),
        )
        .unwrap();

    assert_eq!(
        &table.to_bytes()[field_start..field_start + 10],
        raw.as_slice()
    );
    assert_eq!(table.active_record(1).unwrap().values["AGE"], 30);
}

#[test]
fn applies_update_operators_without_ambiguous_writes() {
    let mut table = DbfTable::from_bytes(&fixture()).unwrap();
    table
        .patch_record(
            1,
            serde_json::json!({
                "$set": {"NAME": "Alicia"},
                "$inc": {"AGE": 1},
                "$unset": {"ACTIVE": true}
            })
            .as_object()
            .unwrap()
            .clone(),
        )
        .unwrap();
    assert_eq!(table.active_record(1).unwrap().values["NAME"], "Alicia");
    assert_eq!(table.active_record(1).unwrap().values["AGE"], 30);
    assert_eq!(
        table.active_record(1).unwrap().values["ACTIVE"],
        Value::Null
    );

    let before = table.to_bytes();
    let error = table
        .patch_record(
            1,
            serde_json::json!({"$set": {"AGE": 31}, "$inc": {"AGE": 1}})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap_err();
    assert!(error.to_string().contains("multiple update operators"));
    assert_eq!(table.to_bytes(), before);

    let error = table
        .patch_record(
            1,
            serde_json::json!({"$unknown": {"AGE": 31}})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap_err();
    assert!(error.to_string().contains("unsupported update operator"));
}
