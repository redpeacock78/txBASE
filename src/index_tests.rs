use super::*;
use serde_json::{Map, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_INDEX_ID: AtomicUsize = AtomicUsize::new(0);

fn fixture() -> Vec<u8> {
    include_str!("../tests/fixtures/users.dbf.hex")
        .split_whitespace()
        .map(|token| u8::from_str_radix(token, 16).unwrap())
        .collect()
}

fn temporary_dbf() -> PathBuf {
    let id = NEXT_INDEX_ID.fetch_add(1, Ordering::Relaxed);
    let path =
        std::env::temp_dir().join(format!("txbase-index-test-{}-{id}.dbf", std::process::id()));
    remove_table_files(&path);
    fs::write(&path, fixture()).unwrap();
    path
}

fn remove_table_files(path: &Path) {
    for candidate in [
        path.to_path_buf(),
        sidecar_path(path),
        path.with_extension("txbase.wal"),
        path.with_extension("txbase.lock"),
    ] {
        let _ = fs::remove_file(candidate);
    }
}

#[test]
fn index_definition_wire_shape_validates_fields_and_directions() {
    let single = IndexDefinition::named("by_name", "NAME");
    assert_eq!(single.name(), "by_name");
    assert_eq!(single.field(), "NAME");
    assert_eq!(
        serde_json::to_value(&single).unwrap(),
        json!({"name": "by_name", "field": "NAME"})
    );
    assert_eq!(
        serde_json::from_value::<IndexDefinition>(serde_json::to_value(&single).unwrap())
            .unwrap()
            .fields(),
        &["NAME".to_owned()]
    );

    let compound = IndexDefinition::named_fields_with_directions(
        "by_name_age",
        vec!["NAME".into(), "AGE".into()],
        vec![1, -1],
    );
    assert_eq!(
        serde_json::to_value(&compound).unwrap(),
        json!({
            "name": "by_name_age",
            "fields": ["NAME", "AGE"],
            "directions": [1, -1]
        })
    );

    for (document, message) in [
        (
            json!({"name": "bad", "field": "NAME", "fields": ["AGE"]}),
            "both field and fields",
        ),
        (
            json!({"name": "bad", "fields": ["NAME", "AGE"], "directions": [1]}),
            "directions must match field count",
        ),
        (
            json!({"name": "bad", "field": "NAME", "directions": [0]}),
            "directions must be 1 or -1",
        ),
    ] {
        let error = serde_json::from_value::<IndexDefinition>(document).unwrap_err();
        assert!(error.to_string().contains(message));
    }
}

#[test]
fn builds_and_loads_an_external_scalar_index() {
    let path = temporary_dbf();
    let index = IndexFile::build(&path, vec![IndexDefinition::for_field("NAME")]).unwrap();
    index.save(&path).unwrap();

    let loaded = IndexFile::load(&path).unwrap();
    assert_eq!(loaded.index_names(), vec!["NAME"]);
    assert_eq!(loaded.lookup_eq("NAME", &json!("Alice")).unwrap(), vec![1]);
    assert!(loaded.lookup_eq("NAME", &json!("Bob")).unwrap().is_empty());
    assert_eq!(loaded.schema_json()["statistics"]["active_record_count"], 1);
    assert_eq!(loaded.equality_selectivity_estimate("NAME"), Some(1));
    assert_eq!(loaded.equality_fanout_estimate(&["NAME"]), Some(1));
    assert_eq!(loaded.index_traversal_cost("NAME"), Some(0));
    assert_eq!(loaded.schema_json()["indexes"][0]["entry_count"], 1);
    assert_eq!(
        loaded.schema_json()["indexes"][0]["histogram_bucket_count"],
        1
    );

    remove_table_files(&path);
}

#[test]
fn builds_and_loads_a_compound_ordered_index() {
    let path = temporary_dbf();
    let mut bytes = fixture();
    bytes[179] = b' ';
    bytes[183..193].copy_from_slice(b"Alice     ");
    fs::write(&path, bytes).unwrap();

    IndexFile::build(
        &path,
        vec![IndexDefinition::named_fields(
            "by_name_age",
            vec!["NAME".into(), "AGE".into()],
        )],
    )
    .unwrap()
    .save(&path)
    .unwrap();

    let loaded = IndexFile::load(&path).unwrap();
    assert_eq!(
        loaded.schema_json()["indexes"][0]["fields"],
        json!(["NAME", "AGE"])
    );
    assert_eq!(loaded.schema_json()["statistics"]["active_record_count"], 2);
    assert_eq!(
        loaded.schema_json()["indexes"][0]["histogram_bucket_count"],
        0
    );
    let (name, fields, directions, records) = loaded
        .lookup_ordered_for_fields(&["NAME", "AGE"], &[1, 1], &Map::new())
        .unwrap()
        .unwrap();
    assert_eq!(name, "by_name_age");
    assert_eq!(fields, vec!["NAME", "AGE"]);
    assert_eq!(directions, vec![1, 1]);
    assert_eq!(records.len(), 2);

    remove_table_files(&path);
}

#[test]
fn builds_a_mixed_direction_compound_index() {
    let path = temporary_dbf();
    let mut bytes = fixture();
    bytes[179] = b' ';
    fs::write(&path, bytes).unwrap();

    IndexFile::build(
        &path,
        vec![IndexDefinition::named_fields_with_directions(
            "by_name_age",
            vec!["NAME".into(), "AGE".into()],
            vec![1, -1],
        )],
    )
    .unwrap()
    .save(&path)
    .unwrap();

    let loaded = IndexFile::load(&path).unwrap();
    assert_eq!(
        loaded.schema_json()["indexes"][0]["directions"],
        json!([1, -1])
    );
    let (_, _, directions, records) = loaded
        .lookup_ordered_for_fields(&["NAME", "AGE"], &[1, -1], &Map::new())
        .unwrap()
        .unwrap();
    assert_eq!(directions, vec![1, -1]);
    assert_eq!(records.len(), 2);

    remove_table_files(&path);
}

#[test]
fn mutation_refreshes_the_sidecar_after_save() {
    let path = temporary_dbf();
    IndexFile::build(&path, vec![IndexDefinition::for_field("NAME")])
        .unwrap()
        .save(&path)
        .unwrap();

    let mut table = DbfTable::from_path(&path).unwrap();
    let mut patch = Map::new();
    patch.insert("NAME".into(), json!("Caroline"));
    table.patch_record(1, patch).unwrap();
    table.save_with_wal(&path).unwrap();

    let index = IndexFile::load(&path).unwrap();
    assert!(index.lookup_eq("NAME", &json!("Alice")).unwrap().is_empty());
    assert_eq!(
        index.lookup_eq("NAME", &json!("Caroline")).unwrap(),
        vec![1]
    );

    remove_table_files(&path);
}

#[test]
fn logical_delete_is_removed_after_save() {
    let path = temporary_dbf();
    IndexFile::build(&path, vec![IndexDefinition::for_field("ID")])
        .unwrap()
        .save(&path)
        .unwrap();

    let mut table = DbfTable::from_path(&path).unwrap();
    table.delete_record(1).unwrap();
    table.save_with_wal(&path).unwrap();

    assert!(
        IndexFile::load(&path)
            .unwrap()
            .lookup_eq("ID", &json!(1))
            .unwrap()
            .is_empty()
    );

    remove_table_files(&path);
}

#[test]
fn direct_dbf_change_leaves_the_sidecar_stale_until_rebuild() {
    let path = temporary_dbf();
    IndexFile::build(&path, vec![IndexDefinition::for_field("NAME")])
        .unwrap()
        .save(&path)
        .unwrap();

    let mut bytes = fs::read(&path).unwrap();
    let record_start = usize::from(u16::from_le_bytes([bytes[8], bytes[9]]));
    bytes[record_start + 4..record_start + 14].copy_from_slice(b"Zoe       ");
    fs::write(&path, bytes).unwrap();

    assert!(matches!(
        IndexFile::load(&path),
        Err(IndexError::Stale { .. })
    ));

    IndexFile::rebuild(&path).unwrap();
    assert_eq!(
        IndexFile::load(&path)
            .unwrap()
            .lookup_eq("NAME", &json!("Zoe"))
            .unwrap(),
        vec![1]
    );

    remove_table_files(&path);
}

#[test]
fn rejects_index_entries_that_do_not_match_current_records() {
    let path = temporary_dbf();
    IndexFile::build(&path, vec![IndexDefinition::for_field("NAME")])
        .unwrap()
        .save(&path)
        .unwrap();

    let sidecar = sidecar_path(&path);
    let mut document: Value = serde_json::from_slice(&fs::read(&sidecar).unwrap()).unwrap();
    document["indexes"][0]["entries"][0]["records"] = json!([2]);
    fs::write(&sidecar, serde_json::to_vec_pretty(&document).unwrap()).unwrap();

    assert!(matches!(
        IndexFile::load(&path),
        Err(IndexError::Invalid(message))
            if message.contains("index entries do not match the current DBF records")
    ));

    remove_table_files(&path);
}

#[test]
fn rebuild_migrates_a_v1_sidecar() {
    let path = temporary_dbf();
    let sidecar = sidecar_path(&path);
    IndexFile::build(&path, vec![IndexDefinition::for_field("NAME")])
        .unwrap()
        .save(&path)
        .unwrap();

    let mut document: Value = serde_json::from_slice(&fs::read(&sidecar).unwrap()).unwrap();
    document["version"] = json!(1);
    fs::write(&sidecar, serde_json::to_vec_pretty(&document).unwrap()).unwrap();

    assert!(matches!(
        IndexFile::load(&path),
        Err(IndexError::Invalid(message)) if message.contains("unsupported version")
    ));

    let rebuilt = IndexFile::rebuild(&path).unwrap();
    assert_eq!(rebuilt.schema_json()["version"], json!(3));
    assert!(IndexFile::load(&path).is_ok());

    remove_table_files(&path);
}

#[test]
fn non_scalar_fields_and_unknown_fields_are_rejected() {
    let path = temporary_dbf();
    let error = IndexFile::build(&path, vec![IndexDefinition::for_field("MISSING")]).unwrap_err();
    assert!(error.to_string().contains("not a user field"));

    let error = IndexFile::build(&path, vec![IndexDefinition::for_field("NAME")])
        .unwrap()
        .lookup_eq("NAME", &json!(["not", "scalar"]))
        .unwrap_err();
    assert!(error.to_string().contains("scalar"));

    let error = IndexFile::build(
        &path,
        vec![IndexDefinition::named_fields(
            "duplicate",
            vec!["NAME".into(), "NAME".into()],
        )],
    )
    .unwrap_err();
    assert!(error.to_string().contains("repeats field"));

    remove_table_files(&path);
}
