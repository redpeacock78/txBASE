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
fn builds_and_loads_an_external_scalar_index() {
    let path = temporary_dbf();
    let index = IndexFile::build(&path, vec![IndexDefinition::for_field("NAME")]).unwrap();
    index.save(&path).unwrap();

    let loaded = IndexFile::load(&path).unwrap();
    assert_eq!(loaded.index_names(), vec!["NAME"]);
    assert_eq!(loaded.lookup_eq("NAME", &json!("Alice")).unwrap(), vec![1]);
    assert!(loaded.lookup_eq("NAME", &json!("Bob")).unwrap().is_empty());
    assert_eq!(loaded.schema_json()["indexes"][0]["entry_count"], 1);

    remove_table_files(&path);
}

#[test]
fn mutation_makes_the_sidecar_stale_until_rebuild() {
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

    assert!(matches!(
        IndexFile::load(&path),
        Err(IndexError::Stale { .. })
    ));

    IndexFile::rebuild(&path).unwrap();
    assert_eq!(
        IndexFile::load(&path)
            .unwrap()
            .lookup_eq("NAME", &json!("Caroline"))
            .unwrap(),
        vec![1]
    );

    remove_table_files(&path);
}

#[test]
fn logical_delete_is_removed_by_rebuild() {
    let path = temporary_dbf();
    IndexFile::build(&path, vec![IndexDefinition::for_field("ID")])
        .unwrap()
        .save(&path)
        .unwrap();

    let mut table = DbfTable::from_path(&path).unwrap();
    table.delete_record(1).unwrap();
    table.save_with_wal(&path).unwrap();
    assert!(matches!(
        IndexFile::load(&path),
        Err(IndexError::Stale { .. })
    ));

    IndexFile::rebuild(&path).unwrap();
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
fn non_scalar_fields_and_unknown_fields_are_rejected() {
    let path = temporary_dbf();
    let error = IndexFile::build(&path, vec![IndexDefinition::for_field("MISSING")]).unwrap_err();
    assert!(error.to_string().contains("not a user field"));

    let error = IndexFile::build(&path, vec![IndexDefinition::for_field("NAME")])
        .unwrap()
        .lookup_eq("NAME", &json!(["not", "scalar"]))
        .unwrap_err();
    assert!(error.to_string().contains("scalar"));

    remove_table_files(&path);
}
