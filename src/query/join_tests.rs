use super::join::{JoinError, execute, parse};
use crate::catalog::Catalog;
use crate::dbf::DbfTable;
use serde_json::json;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_JOIN_ID: AtomicUsize = AtomicUsize::new(0);

fn fixture() -> Vec<u8> {
    include_str!("../../tests/fixtures/users.dbf.hex")
        .split_whitespace()
        .map(|token| u8::from_str_radix(token, 16).unwrap())
        .collect()
}

fn temporary_catalog() -> PathBuf {
    let id = NEXT_JOIN_ID.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!("txbase-join-test-{}-{id}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir(&root).unwrap();
    root
}

fn catalog_with_posts() -> PathBuf {
    let root = temporary_catalog();
    fs::write(root.join("users.dbf"), fixture()).unwrap();
    let mut posts = DbfTable::from_bytes(&fixture()).unwrap();
    posts
        .insert_record(
            json!({"ID": 1, "NAME": "Post", "AGE": 1, "ACTIVE": true})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();
    posts
        .insert_record(
            json!({"ID": 2, "NAME": "Other", "AGE": 2, "ACTIVE": true})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();
    posts.save_with_wal(root.join("posts.dbf")).unwrap();
    root
}

#[test]
fn left_join_accepts_attachment_shape_and_projects_namespaced_fields() {
    let root = catalog_with_posts();
    let catalog = Catalog::from_path(&root).unwrap();
    let request = parse(
        br#"{
          "from": "users",
          "join": {
            "type": "left",
            "table": "posts",
            "on": {
              "users.ID": {"$eq": {"$field": "posts.ID"}}
            }
          },
          "filter": {"users.ACTIVE": true},
          "projection": {"users.NAME": 1, "posts.NAME": 1}
        }"#,
    )
    .unwrap();

    let rows = execute(&catalog, &request).unwrap();

    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["users.NAME"], "Alice");
    assert_eq!(rows[0]["posts.NAME"], "Alice");
    assert_eq!(rows[1]["posts.NAME"], "Post");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn left_join_keeps_unmatched_left_record_and_inner_join_drops_it() {
    let root = catalog_with_posts();
    let catalog = Catalog::from_path(&root).unwrap();
    let request = parse(
        br#"{
          "from": "users",
          "join": {
            "type": "left",
            "table": "posts",
            "on": {
              "users.AGE": {"$eq": {"$field": "posts.ID"}}
            }
          },
          "projection": {"users.NAME": 1, "posts.NAME": 1}
        }"#,
    )
    .unwrap();

    let rows = execute(&catalog, &request).unwrap();
    assert_eq!(rows, vec![json!({"users.NAME": "Alice"})]);

    let mut inner = request;
    inner.join.kind = super::join::JoinType::Inner;
    assert!(execute(&catalog, &inner).unwrap().is_empty());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn right_join_keeps_unmatched_right_record() {
    let root = catalog_with_posts();
    let catalog = Catalog::from_path(&root).unwrap();
    let request = parse(
        br#"{
          "from": "users",
          "join": {
            "type": "right",
            "table": "posts",
            "on": {
              "users.ID": {"$eq": {"$field": "posts.ID"}}
            }
          },
          "projection": {"users.NAME": 1, "posts.NAME": 1}
        }"#,
    )
    .unwrap();

    let rows = execute(&catalog, &request).unwrap();
    let other = json!("Other");
    assert!(
        rows.iter().any(|row| {
            row.get("posts.NAME") == Some(&other) && row.get("users.NAME").is_none()
        })
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn semi_and_anti_join_emit_only_matching_or_unmatched_left_rows() {
    let root = catalog_with_posts();
    let catalog = Catalog::from_path(&root).unwrap();
    let mut semi = parse(
        br#"{
          "from": "users",
          "join": {
            "type": "semi",
            "table": "posts",
            "on": {
              "users.ID": {"$eq": {"$field": "posts.ID"}}
            }
          },
          "projection": {"users.NAME": 1, "posts.NAME": 1}
        }"#,
    )
    .unwrap();

    let rows = execute(&catalog, &semi).unwrap();
    assert_eq!(rows, vec![json!({"users.NAME": "Alice"})]);

    semi.join.kind = super::join::JoinType::Anti;
    let rows = execute(&catalog, &semi).unwrap();
    assert!(rows.is_empty());

    semi.join.on.insert(
        String::from("users.AGE"),
        super::join::JoinCondition {
            equality: super::join::JoinField {
                field: String::from("posts.ID"),
            },
        },
    );
    let rows = execute(&catalog, &semi).unwrap();
    assert_eq!(rows, vec![json!({"users.NAME": "Alice"})]);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cross_join_emits_bounded_cartesian_rows() {
    let root = catalog_with_posts();
    let catalog = Catalog::from_path(&root).unwrap();
    let request = parse(
        br#"{
          "from": "users",
          "join": {
            "type": "cross",
            "table": "posts",
            "on": {}
          },
          "projection": {"users.NAME": 1, "posts.NAME": 1}
        }"#,
    )
    .unwrap();

    let rows = execute(&catalog, &request).unwrap();
    assert_eq!(rows.len(), 3);
    assert!(
        rows.iter()
            .all(|row| row.get("users.NAME") == Some(&json!("Alice")))
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cross_join_rejects_equality_conditions() {
    let error = parse(
        br#"{
          "from": "users",
          "join": {
            "type": "cross",
            "table": "posts",
            "on": {"users.ID": {"$eq": {"$field": "posts.ID"}}}
          }
        }"#,
    )
    .unwrap_err();
    assert!(matches!(error, JoinError::Invalid(message) if message.contains("do not accept")));
}

#[test]
fn join_accepts_multiple_equality_conditions() {
    let request = parse(
        br#"{
          "from": "users",
          "join": {
            "type": "inner",
            "table": "posts",
            "on": {
              "users.ID": {"$eq": {"$field": "posts.ID"}},
              "users.AGE": {"$eq": {"$field": "posts.AGE"}}
            }
          }
        }"#,
    )
    .unwrap();
    let root = catalog_with_posts();
    let catalog = Catalog::from_path(&root).unwrap();
    let rows = execute(&catalog, &request).unwrap();

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["users.NAME"], "Alice");
    assert_eq!(rows[0]["posts.NAME"], "Alice");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn join_rejects_an_empty_condition_set() {
    let error = parse(
        br#"{
          "from": "users",
          "join": {
            "type": "inner",
            "table": "posts",
            "on": {}
          }
        }"#,
    )
    .unwrap_err();
    assert!(matches!(error, JoinError::Invalid(message) if message.contains("at least one")));
}
