use super::*;
use std::fs;

#[test]
fn verify_reports_a_malformed_table_by_name() {
    let root = temporary_catalog();
    fs::write(root.join("broken.dbf"), b"not a DBF").unwrap();

    let error = Catalog::from_path(&root).unwrap().verify().unwrap_err();

    assert!(error.to_string().contains("table broken"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn verify_reports_a_stale_index_sidecar_by_table_name() {
    let root = temporary_catalog();
    let path = root.join("users.dbf");
    fs::write(&path, fixture()).unwrap();
    crate::index::IndexFile::build(
        &path,
        vec![crate::index::IndexDefinition::for_field("NAME")],
    )
    .unwrap()
    .save(&path)
    .unwrap();
    let mut changed = DbfTable::from_path(&path).unwrap();
    changed
        .patch_record(1, json!({"NAME": "Changed"}).as_object().unwrap().clone())
        .unwrap();
    fs::write(&path, changed.to_bytes()).unwrap();

    let error = Catalog::from_path(&root).unwrap().verify().unwrap_err();
    assert!(error.to_string().contains("table users"));
    assert!(error.to_string().contains("index sidecar is invalid"));

    fs::remove_dir_all(root).unwrap();
}
