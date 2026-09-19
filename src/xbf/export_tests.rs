use super::{XbfField, XbfRecord, XbfTable, XbfType, XbfValue};
use serde_json::json;
use std::fs;

fn constraint_table() -> XbfTable {
    XbfTable {
        generation: 4,
        fields: vec![
            XbfField {
                name: "ID".into(),
                ty: XbfType::Signed64,
                nullable: false,
                primary_key: true,
                unique: true,
            },
            XbfField {
                name: "NAME".into(),
                ty: XbfType::String,
                nullable: false,
                primary_key: false,
                unique: true,
            },
        ],
        records: vec![XbfRecord {
            deleted: false,
            values: vec![XbfValue::Signed64(1), XbfValue::String("Alice".into())],
        }],
    }
}

#[test]
fn converts_a_dbf_fixture_to_xbf_without_dropping_records() {
    let bytes = include_str!("../../tests/fixtures/users.dbf.hex")
        .split_whitespace()
        .map(|byte| u8::from_str_radix(byte, 16).unwrap())
        .collect::<Vec<_>>();
    let dbf = crate::dbf::DbfTable::from_bytes(&bytes).unwrap();

    let xbf = super::from_dbf(&dbf).unwrap();

    assert_eq!(xbf.generation, 0);
    assert_eq!(xbf.fields[0].name, "ID");
    assert_eq!(xbf.fields[0].ty, XbfType::Signed64);
    assert_eq!(xbf.fields[1].name, "NAME");
    assert_eq!(xbf.fields[1].ty, XbfType::String);
    assert_eq!(xbf.fields[2].name, "AGE");
    assert_eq!(xbf.fields[2].ty, XbfType::Signed64);
    assert_eq!(xbf.records.len(), dbf.records().len());
    assert_eq!(xbf.records[0].deleted, dbf.records()[0].deleted);
    assert!(matches!(xbf.records[0].values[1], XbfValue::String(_)));
}

#[test]
fn exports_a_representable_xbf_table_to_dbf() {
    let table = XbfTable {
        generation: 3,
        fields: vec![
            XbfField {
                name: "ID".into(),
                ty: XbfType::Signed64,
                nullable: false,
                primary_key: false,
                unique: false,
            },
            XbfField {
                name: "NAME".into(),
                ty: XbfType::String,
                nullable: false,
                primary_key: false,
                unique: false,
            },
            XbfField {
                name: "ACTIVE".into(),
                ty: XbfType::Boolean,
                nullable: false,
                primary_key: false,
                unique: false,
            },
        ],
        records: vec![
            XbfRecord {
                deleted: false,
                values: vec![
                    XbfValue::Signed64(1),
                    XbfValue::String("Alice".into()),
                    XbfValue::Boolean(true),
                ],
            },
            XbfRecord {
                deleted: true,
                values: vec![
                    XbfValue::Signed64(2),
                    XbfValue::String("Bob".into()),
                    XbfValue::Boolean(false),
                ],
            },
        ],
    };

    let dbf = super::to_dbf(&table).unwrap();
    let round_trip = crate::dbf::DbfTable::from_bytes(&dbf.to_bytes()).unwrap();

    assert_eq!(round_trip.records().len(), 2);
    assert_eq!(
        round_trip.records()[0].values["NAME"],
        serde_json::json!("Alice")
    );
    assert!(!round_trip.records()[0].deleted);
    assert!(round_trip.records()[1].deleted);
}

#[test]
fn exports_xbf_constraints_as_schema_metadata() {
    let table = constraint_table();

    assert!(super::to_dbf(&table).is_err());
    let (dbf, schema) = super::to_dbf_with_schema(&table).unwrap();
    assert_eq!(dbf.active_record(1).unwrap().values["NAME"], "Alice");
    assert_eq!(
        schema,
        json!({
            "format": "txbase-schema",
            "version": 1,
            "fields": {
                "ID": {"primary": true, "not_null": true},
                "NAME": {"unique": true, "not_null": true}
            }
        })
    );
}

#[test]
fn saves_xbf_dbf_and_schema_sidecar() {
    let path = std::env::temp_dir().join(format!(
        "txbase-xbf-export-schema-{}.dbf",
        std::process::id()
    ));
    let schema_path = path.with_extension("txschema.json");
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(&schema_path);

    super::save_dbf_with_schema(&constraint_table(), &path).unwrap();

    let dbf = crate::dbf::DbfTable::from_path(&path).unwrap();
    assert_eq!(
        dbf.schema_json()["schema_metadata"]["fields"]["ID"]["primary"],
        true
    );
    assert_eq!(
        dbf.schema_json()["schema_metadata"]["fields"]["NAME"]["unique"],
        true
    );

    fs::remove_file(path).unwrap();
    fs::remove_file(schema_path).unwrap();
}

#[test]
fn rejects_nonrepresentable_xbf_dbf_export_types() {
    let table = XbfTable {
        generation: 0,
        fields: vec![XbfField {
            name: "DOC".into(),
            ty: XbfType::Json,
            nullable: false,
            primary_key: false,
            unique: false,
        }],
        records: vec![XbfRecord {
            deleted: false,
            values: vec![XbfValue::Json(serde_json::json!({"ok": true}))],
        }],
    };

    assert!(super::to_dbf(&table).is_err());
}

#[test]
fn reports_all_nonrepresentable_xbf_fields_and_values() {
    let table = XbfTable {
        generation: 0,
        fields: vec![
            XbfField {
                name: "DOC".into(),
                ty: XbfType::Json,
                nullable: false,
                primary_key: false,
                unique: false,
            },
            XbfField {
                name: "UUID".into(),
                ty: XbfType::Uuid,
                nullable: false,
                primary_key: false,
                unique: false,
            },
        ],
        records: vec![XbfRecord {
            deleted: false,
            values: vec![XbfValue::Json(json!({"ok": true})), XbfValue::Uuid([1; 16])],
        }],
    };

    let report = super::dbf_export_report(&table);

    assert!(!report.representable);
    assert!(!report.requires_schema_sidecar);
    assert!(report.issues.iter().any(|issue| {
        issue.field.as_deref() == Some("DOC")
            && issue.message.contains("no lossless representation")
    }));
    assert!(report.issues.iter().any(|issue| {
        issue.field.as_deref() == Some("UUID")
            && issue.message.contains("no lossless representation")
    }));
}

#[test]
fn report_marks_constraint_sidecar_requirement() {
    let report = super::dbf_export_report(&constraint_table());

    assert!(report.representable);
    assert!(report.requires_schema_sidecar);
    assert!(report.issues.is_empty());
}
