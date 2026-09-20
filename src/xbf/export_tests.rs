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
fn preserves_declared_dbf_nullability_without_a_current_null_value() {
    let mut bytes = vec![0; 101];
    bytes[0] = 0x30;
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
    bytes[96] = 0x0d;
    bytes[97] = 0x20;
    bytes[98] = b'A';
    bytes[100] = 0x1a;

    let dbf = crate::dbf::DbfTable::from_bytes(&bytes).unwrap();
    assert_eq!(dbf.active_record(1).unwrap().values["NAME"], "A");
    let xbf = super::from_dbf(&dbf).unwrap();

    assert!(xbf.fields[0].nullable);
}

#[test]
fn converts_representable_dbf_schema_constraints_to_xbf() {
    let path =
        std::env::temp_dir().join(format!("txbase-xbf-dbfschema-{}.dbf", std::process::id()));
    let schema_path = path.with_extension("txschema.json");
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(&schema_path);
    let bytes = include_str!("../../tests/fixtures/users.dbf.hex")
        .split_whitespace()
        .map(|byte| u8::from_str_radix(byte, 16).unwrap())
        .collect::<Vec<_>>();
    fs::write(&path, bytes).unwrap();
    fs::write(
        &schema_path,
        serde_json::to_vec(&json!({
            "format": "txbase-schema",
            "version": 1,
            "fields": {
                "ID": {"primary": true},
                "NAME": {"unique": true, "not_null": true}
            }
        }))
        .unwrap(),
    )
    .unwrap();

    let dbf = crate::dbf::DbfTable::from_path(&path).unwrap();
    let xbf = super::from_dbf(&dbf).unwrap();
    let id = xbf.fields.iter().find(|field| field.name == "ID").unwrap();
    let name = xbf
        .fields
        .iter()
        .find(|field| field.name == "NAME")
        .unwrap();
    assert!(id.primary_key && id.unique && !id.nullable);
    assert!(!name.primary_key && name.unique && !name.nullable);

    fs::remove_file(path).unwrap();
    fs::remove_file(schema_path).unwrap();
}

#[test]
fn rejects_dbf_schema_constraints_without_an_xbf_v1_representation() {
    let path = std::env::temp_dir().join(format!(
        "txbase-xbf-dbfschema-unsupported-{}.dbf",
        std::process::id()
    ));
    let schema_path = path.with_extension("txschema.json");
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(&schema_path);
    let bytes = include_str!("../../tests/fixtures/users.dbf.hex")
        .split_whitespace()
        .map(|byte| u8::from_str_radix(byte, 16).unwrap())
        .collect::<Vec<_>>();
    fs::write(&path, bytes).unwrap();
    fs::write(
        &schema_path,
        serde_json::to_vec(&json!({
            "format": "txbase-schema",
            "version": 1,
            "checks": [{"AGE": {"$gte": 0}}]
        }))
        .unwrap(),
    )
    .unwrap();

    let dbf = crate::dbf::DbfTable::from_path(&path).unwrap();
    let error = super::from_dbf(&dbf).unwrap_err();
    assert!(error.to_string().contains("not representable in XBF v1"));

    fs::remove_file(path).unwrap();
    fs::remove_file(schema_path).unwrap();
}

#[test]
fn exports_a_representable_xbf_table_to_dbf() {
    let table = XbfTable {
        generation: 3,
        fields: vec![
            XbfField {
                name: "ID".into(),
                ty: XbfType::Signed64,
                nullable: true,
                primary_key: false,
                unique: false,
            },
            XbfField {
                name: "NAME".into(),
                ty: XbfType::String,
                nullable: true,
                primary_key: false,
                unique: false,
            },
            XbfField {
                name: "ACTIVE".into(),
                ty: XbfType::Boolean,
                nullable: true,
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
fn rejects_direct_export_of_non_nullable_fields() {
    let table = XbfTable {
        generation: 0,
        fields: vec![XbfField {
            name: "ID".into(),
            ty: XbfType::Signed64,
            nullable: false,
            primary_key: false,
            unique: false,
        }],
        records: vec![XbfRecord {
            deleted: false,
            values: vec![XbfValue::Signed64(1)],
        }],
    };

    let error = super::to_dbf(&table).unwrap_err();
    assert!(error.to_string().contains("not-null constraint"));
    assert!(super::to_dbf_with_schema(&table).is_ok());
}

#[test]
fn rejects_invalid_primary_key_schema_during_export() {
    let table = XbfTable {
        generation: 0,
        fields: vec![XbfField {
            name: "ID".into(),
            ty: XbfType::Signed64,
            nullable: true,
            primary_key: true,
            unique: true,
        }],
        records: vec![XbfRecord {
            deleted: false,
            values: vec![XbfValue::Signed64(1)],
        }],
    };

    let error = super::to_dbf_with_schema(&table).unwrap_err();
    assert!(error.to_string().contains("cannot be nullable"));
    assert!(!super::dbf_export_report(&table).representable);
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
                nullable: true,
                primary_key: false,
                unique: false,
            },
            XbfField {
                name: "UUID".into(),
                ty: XbfType::Uuid,
                nullable: true,
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

#[test]
fn report_keeps_all_record_shape_issues() {
    let table = XbfTable {
        generation: 0,
        fields: vec![XbfField {
            name: "ID".into(),
            ty: XbfType::Signed64,
            nullable: true,
            primary_key: false,
            unique: false,
        }],
        records: vec![
            XbfRecord {
                deleted: false,
                values: Vec::new(),
            },
            XbfRecord {
                deleted: false,
                values: vec![XbfValue::Signed64(1), XbfValue::Signed64(2)],
            },
        ],
    };

    let report = super::dbf_export_report(&table);

    assert!(!report.representable);
    assert_eq!(
        report
            .issues
            .iter()
            .filter_map(|issue| issue.record)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
}

#[test]
fn report_catches_constraint_violations() {
    let mut table = constraint_table();
    table.records.push(XbfRecord {
        deleted: false,
        values: vec![XbfValue::Signed64(1), XbfValue::String("Bob".into())],
    });

    let report = super::dbf_export_report(&table);

    assert!(!report.representable);
    assert!(
        report
            .issues
            .iter()
            .any(|issue| issue.message.contains("duplicate values"))
    );
}
