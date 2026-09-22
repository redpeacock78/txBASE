use super::*;
use std::fs;

#[test]
fn discovers_direct_dbf_tables_and_ignores_other_files() {
    let root = temporary_catalog();
    fs::write(root.join("users.dbf"), fixture()).unwrap();
    fs::write(root.join("posts.DBF"), fixture()).unwrap();
    fs::write(root.join("users.dbt"), b"memo").unwrap();
    fs::create_dir(root.join("nested")).unwrap();
    fs::write(root.join("nested").join("ignored.dbf"), fixture()).unwrap();
    fs::write(root.join("nested").join("ignored.dbf"), fixture()).unwrap();

    let catalog = Catalog::from_path(&root).unwrap();

    assert_eq!(catalog.table_names(), vec!["posts", "users"]);
    assert_eq!(
        catalog.table_path("users").unwrap(),
        root.join("users.dbf").as_path()
    );
    assert_eq!(catalog.open_table("users").unwrap().active_json().len(), 1);

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn schema_and_verify_cover_every_discovered_table() {
    let root = temporary_catalog();
    fs::write(root.join("users.dbf"), fixture()).unwrap();
    fs::write(root.join("posts.dbf"), fixture()).unwrap();

    let catalog = Catalog::from_path(&root).unwrap();
    catalog.verify().unwrap();
    let schema = catalog.schema_json().unwrap();

    assert_eq!(schema["format"], "txbase-catalog");
    assert_eq!(schema["tables"].as_array().unwrap().len(), 2);
    assert_eq!(schema["tables"][0]["name"], "posts");
    assert_eq!(schema["tables"][1]["schema"]["format"], "dbf");

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn representation_tag_is_stable_and_changes_with_catalog_schema() {
    let schema = json!({
        "format": "txbase-catalog",
        "transaction_id": null,
        "tables": []
    });
    let same_schema = json!({
        "format": "txbase-catalog",
        "transaction_id": null,
        "tables": []
    });
    let changed_schema = json!({
        "format": "txbase-catalog",
        "transaction_id": 1,
        "tables": []
    });

    let tag = super::representation_tag(&schema);
    assert_eq!(tag, super::representation_tag(&same_schema));
    assert_ne!(tag, super::representation_tag(&changed_schema));
    assert!(tag.starts_with("\"txbase-catalog-"));
    assert!(tag.ends_with('"'));
}
