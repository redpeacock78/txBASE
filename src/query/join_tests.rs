use super::join::{JoinError, execute, parse};
use crate::catalog::Catalog;
use crate::dbf::DbfTable;
use crate::index::{IndexDefinition, IndexFile};
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

fn catalog_with_many_indexed_posts() -> PathBuf {
    let root = temporary_catalog();
    let users_path = root.join("users.dbf");
    fs::write(&users_path, fixture()).unwrap();
    IndexFile::build(
        &users_path,
        vec![
            IndexDefinition::named("by_id", "ID"),
            IndexDefinition::named_fields(
                "by_age_id",
                vec![String::from("AGE"), String::from("ID")],
            ),
        ],
    )
    .unwrap()
    .save(&users_path)
    .unwrap();
    let mut posts = DbfTable::from_bytes(&fixture()).unwrap();
    for id in 1..=80 {
        posts
            .insert_record(
                json!({"ID": id, "NAME": "Indexed", "AGE": id, "ACTIVE": true})
                    .as_object()
                    .unwrap()
                    .clone(),
            )
            .unwrap();
    }
    let posts_path = root.join("posts.dbf");
    posts.save_with_wal(&posts_path).unwrap();
    IndexFile::build(
        &posts_path,
        vec![
            IndexDefinition::named("by_id", "ID"),
            IndexDefinition::named_fields(
                "by_age_id",
                vec![String::from("AGE"), String::from("ID")],
            ),
        ],
    )
    .unwrap()
    .save(&posts_path)
    .unwrap();
    root
}

fn catalog_with_many_indexed_posts_and_comments() -> PathBuf {
    let root = catalog_with_many_indexed_posts();
    let mut comments = DbfTable::from_bytes(&fixture()).unwrap();
    for _ in 0..80 {
        comments
            .insert_record(
                json!({"ID": 1, "NAME": "Comment", "AGE": 1, "ACTIVE": true})
                    .as_object()
                    .unwrap()
                    .clone(),
            )
            .unwrap();
    }
    comments
        .insert_record(
            json!({"ID": 99, "NAME": "Orphan", "AGE": 99, "ACTIVE": true})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();
    let comments_path = root.join("comments.dbf");
    comments.save_with_wal(&comments_path).unwrap();
    IndexFile::build(
        &comments_path,
        vec![
            IndexDefinition::named("by_id", "ID"),
            IndexDefinition::named_fields(
                "by_age_id",
                vec![String::from("AGE"), String::from("ID")],
            ),
        ],
    )
    .unwrap()
    .save(&comments_path)
    .unwrap();
    root
}

fn catalog_with_posts_and_comments() -> PathBuf {
    let root = catalog_with_posts();
    let mut comments = DbfTable::from_bytes(&fixture()).unwrap();
    comments.save_with_wal(root.join("comments.dbf")).unwrap();
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
fn large_single_key_join_uses_fresh_ordered_indexes() {
    let root = catalog_with_many_indexed_posts();
    let catalog = Catalog::from_path(&root).unwrap();
    let request = parse(
        br#"{
          "from": "users",
          "join": {
            "type": "inner",
            "table": "posts",
            "on": {
              "users.ID": {"$eq": {"$field": "posts.ID"}}
            }
          },
          "projection": {"users.ID": 1, "posts.ID": 1}
        }"#,
    )
    .unwrap();

    let rows = execute(&catalog, &request).unwrap();

    assert!(!rows.is_empty());
    assert!(rows.iter().all(|row| row["users.ID"] == row["posts.ID"]));

    let mut right_request = request.clone();
    right_request.join.kind = super::join::JoinType::Right;
    let right_rows = execute(&catalog, &right_request).unwrap();
    assert!(right_rows.iter().any(|row| {
        row.get("users.ID")
            .zip(row.get("posts.ID"))
            .is_some_and(|(left, right)| left == right)
    }));
    assert!(right_rows.iter().any(|row| row.get("users.ID").is_none()));

    let mut left_request = request.clone();
    left_request.join.kind = super::join::JoinType::Left;
    let left_rows = execute(&catalog, &left_request).unwrap();
    assert!(!left_rows.is_empty());
    assert!(left_rows.iter().all(|row| row.get("users.ID").is_some()));

    let mut semi_request = request.clone();
    semi_request.join.kind = super::join::JoinType::Semi;
    let semi_rows = execute(&catalog, &semi_request).unwrap();
    assert!(semi_rows.iter().all(|row| row.get("posts.ID").is_none()));

    let mut anti_request = request;
    anti_request.join.kind = super::join::JoinType::Anti;
    let anti_rows = execute(&catalog, &anti_request).unwrap();
    assert!(anti_rows.len() <= left_rows.len());

    let compound_request = parse(
        br#"{
          "from": "users",
          "join": {
            "type": "inner",
            "table": "posts",
            "on": {
              "users.ID": {"$eq": {"$field": "posts.ID"}},
              "users.AGE": {"$eq": {"$field": "posts.AGE"}}
            }
          },
          "projection": {"users.ID": 1, "posts.ID": 1, "users.AGE": 1, "posts.AGE": 1}
        }"#,
    )
    .unwrap();
    let compound_rows = execute(&catalog, &compound_request).unwrap();
    assert!(!compound_rows.is_empty());
    assert!(
        compound_rows.iter().all(|row| {
            row["users.ID"] == row["posts.ID"] && row["users.AGE"] == row["posts.AGE"]
        })
    );

    let mut compound_right_request = compound_request;
    compound_right_request.join.kind = super::join::JoinType::Right;
    let compound_right_rows = execute(&catalog, &compound_right_request).unwrap();
    assert!(compound_right_rows.iter().any(|row| {
        row.get("users.ID")
            .zip(row.get("posts.ID"))
            .is_some_and(|(left, right)| left == right)
    }));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn chained_single_key_join_uses_a_fresh_foreign_index() {
    let root = catalog_with_many_indexed_posts_and_comments();
    let catalog = Catalog::from_path(&root).unwrap();
    let request = parse(
        br#"{
          "from": "users",
          "join": {
            "type": "cross",
            "table": "posts",
            "on": {}
          },
          "joins": [
            {
              "type": "inner",
              "table": "comments",
              "on": {
                "posts.ID": {"$eq": {"$field": "comments.ID"}}
              }
            }
          ],
          "projection": {"posts.ID": 1, "comments.ID": 1}
        }"#,
    )
    .unwrap();

    let rows = execute(&catalog, &request).unwrap();

    assert!(rows.len() > 64);
    assert!(rows.iter().all(|row| row["posts.ID"] == row["comments.ID"]));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn chained_right_single_key_join_uses_a_fresh_foreign_index() {
    let root = catalog_with_many_indexed_posts_and_comments();
    let catalog = Catalog::from_path(&root).unwrap();
    let request = parse(
        br#"{
          "from": "users",
          "join": {
            "type": "cross",
            "table": "posts",
            "on": {}
          },
          "joins": [
            {
              "type": "right",
              "table": "comments",
              "on": {
                "posts.ID": {"$eq": {"$field": "comments.ID"}}
              }
            }
          ],
          "projection": {"posts.ID": 1, "comments.ID": 1}
        }"#,
    )
    .unwrap();

    let rows = execute(&catalog, &request).unwrap();

    assert!(rows.len() >= 80);
    assert!(rows.iter().all(|row| row.get("comments.ID").is_some()));
    assert!(rows.iter().any(|row| row.get("posts.ID").is_none()));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn chained_compound_join_uses_a_fresh_foreign_index() {
    let root = catalog_with_many_indexed_posts_and_comments();
    let catalog = Catalog::from_path(&root).unwrap();
    let request = parse(
        br#"{
          "from": "users",
          "join": {
            "type": "cross",
            "table": "posts",
            "on": {}
          },
          "joins": [
            {
              "type": "inner",
              "table": "comments",
              "on": {
                "posts.ID": {"$eq": {"$field": "comments.ID"}},
                "posts.AGE": {"$eq": {"$field": "comments.AGE"}}
              }
            }
          ],
          "projection": {"posts.ID": 1, "comments.ID": 1, "posts.AGE": 1, "comments.AGE": 1}
        }"#,
    )
    .unwrap();

    let rows = execute(&catalog, &request).unwrap();

    assert!(rows.len() > 64);
    assert!(rows.iter().all(|row| {
        row["posts.ID"] == row["comments.ID"] && row["posts.AGE"] == row["comments.AGE"]
    }));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn chained_right_compound_join_uses_a_fresh_foreign_index() {
    let root = catalog_with_many_indexed_posts_and_comments();
    let catalog = Catalog::from_path(&root).unwrap();
    let request = parse(
        br#"{
          "from": "users",
          "join": {
            "type": "cross",
            "table": "posts",
            "on": {}
          },
          "joins": [
            {
              "type": "right",
              "table": "comments",
              "on": {
                "posts.ID": {"$eq": {"$field": "comments.ID"}},
                "posts.AGE": {"$eq": {"$field": "comments.AGE"}}
              }
            }
          ],
          "projection": {"posts.ID": 1, "comments.ID": 1, "posts.AGE": 1, "comments.AGE": 1}
        }"#,
    )
    .unwrap();

    let rows = execute(&catalog, &request).unwrap();

    assert!(rows.len() >= 81);
    assert!(rows.iter().all(|row| row.get("comments.ID").is_some()));
    assert!(rows.iter().any(|row| row.get("posts.ID").is_none()));
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
fn chained_joins_reference_fields_from_prior_stages() {
    let root = catalog_with_posts_and_comments();
    let catalog = Catalog::from_path(&root).unwrap();
    let request = parse(
        br#"{
          "from": "users",
          "join": {
            "type": "inner",
            "table": "posts",
            "on": {
              "users.ID": {"$eq": {"$field": "posts.ID"}}
            }
          },
          "joins": [
            {
              "type": "left",
              "table": "comments",
              "on": {
                "posts.ID": {"$eq": {"$field": "comments.ID"}}
              }
            }
          ],
          "projection": {
            "users.NAME": 1,
            "posts.NAME": 1,
            "comments.NAME": 1
          }
        }"#,
    )
    .unwrap();

    let rows = execute(&catalog, &request).unwrap();

    assert_eq!(rows.len(), 2);
    assert!(
        rows.iter()
            .all(|row| row.get("users.NAME") == Some(&json!("Alice")))
    );
    assert!(
        rows.iter()
            .all(|row| row.get("comments.NAME") == Some(&json!("Alice")))
    );
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
