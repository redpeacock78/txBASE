use super::join::{execute, parse};
use super::join_tests::{
    catalog_with_many_indexed_posts, catalog_with_many_indexed_posts_and_comments,
};
use crate::catalog::Catalog;
use std::fs;

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
