use super::*;
use crate::dbf::DbfRecord;
use serde_json::Number;

fn table_with_two_active_records() -> DbfTable {
    let mut bytes = include_str!("../../tests/fixtures/users.dbf.hex")
        .split_whitespace()
        .map(|token| u8::from_str_radix(token, 16).unwrap())
        .collect::<Vec<_>>();
    bytes[179] = b' ';
    DbfTable::from_bytes(&bytes).unwrap()
}

#[test]
fn parses_query_shape() {
    let query = parse(
        br#"{
            "filter": {"age": {"$gte": 20}},
            "sort": {"age": 1},
            "projection": {"name": 1},
            "limit": 10
        }"#,
    )
    .unwrap();

    assert_eq!(query.sort["age"], 1);
    assert_eq!(query.limit, Some(10));
}

#[test]
fn rejects_a_query_document_over_the_shared_input_limit() {
    let body = vec![b' '; crate::MAX_JSON_INPUT_BYTES + 1];
    let error = parse(&body).unwrap_err();
    assert!(error.to_string().contains("query document exceeds"));
}

#[test]
fn rejects_invalid_sort_direction() {
    let error = parse(br#"{"sort":{"age":2}}"#).unwrap_err();
    assert!(error.to_string().contains("must be 1 or -1"));
}

#[test]
fn executes_filter_sort_projection_and_pagination() {
    let table = table_with_two_active_records();
    let request = parse(
        br#"{
            "filter": {"AGE": {"$gte": 7}},
            "sort": {"AGE": -1},
            "projection": {"NAME": 1, "AGE": 1},
            "skip": 1,
            "limit": 1
        }"#,
    )
    .unwrap();

    assert_eq!(
        execute_query(&table, &request).unwrap(),
        vec![serde_json::json!({
            "NAME": "Bob",
            "AGE": 7
        })]
    );
}

#[test]
fn supports_dotted_paths_for_nested_values() {
    let first_values = serde_json::json!({
        "PROFILE": {
            "CITY": "Tokyo",
            "TAGS": [{"NAME": "jp"}, {"NAME": "db"}]
        }
    })
    .as_object()
    .unwrap()
    .clone();
    let second_values = serde_json::json!({
        "PROFILE": {"CITY": "Osaka"}
    })
    .as_object()
    .unwrap()
    .clone();
    let first = DbfRecord {
        number: 1,
        deleted: false,
        values: first_values.clone(),
    };
    let second = DbfRecord {
        number: 2,
        deleted: false,
        values: second_values,
    };

    assert!(
        matches_filter(
            &first_values,
            serde_json::json!({
                "PROFILE.CITY": "Tokyo",
                "PROFILE.TAGS.NAME": {"$in": ["db"]}
            })
            .as_object()
            .unwrap()
        )
        .unwrap()
    );
    assert_eq!(
        compare_records(
            &first,
            &second,
            &IndexMap::from([(String::from("PROFILE.CITY"), 1)])
        ),
        Ordering::Greater
    );

    let projection = BTreeMap::from([
        (String::from("PROFILE.CITY"), 1),
        (String::from("PROFILE.TAGS.NAME"), 1),
    ]);
    assert_eq!(
        project(&first, &projection),
        serde_json::json!({
            "PROFILE": {
                "CITY": "Tokyo",
                "TAGS": {"NAME": ["jp", "db"]}
            }
        })
    );

    let exclusion = BTreeMap::from([(String::from("PROFILE.CITY"), 0)]);
    assert_eq!(
        project(&first, &exclusion),
        serde_json::json!({
            "PROFILE": {"TAGS": [{"NAME": "jp"}, {"NAME": "db"}]}
        })
    );

    let mut literal_values = first_values;
    literal_values.insert(
        String::from("PROFILE.CITY"),
        Value::String("literal".into()),
    );
    assert_eq!(
        field_value(&literal_values, "PROFILE.CITY"),
        Some(Value::String("literal".into()))
    );
}

#[test]
fn supports_explicit_array_indices_in_paths() {
    let values = serde_json::json!({
        "PROFILE": {
            "TAGS": [{"NAME": "jp"}, {"NAME": "db"}]
        }
    })
    .as_object()
    .unwrap()
    .clone();
    let other_values = serde_json::json!({
        "PROFILE": {
            "TAGS": [{"NAME": "aa"}]
        }
    })
    .as_object()
    .unwrap()
    .clone();
    let first = DbfRecord {
        number: 1,
        deleted: false,
        values: values.clone(),
    };
    let second = DbfRecord {
        number: 2,
        deleted: false,
        values: other_values,
    };

    assert_eq!(
        field_value(&values, "PROFILE.TAGS.0.NAME"),
        Some(Value::String("jp".into()))
    );
    assert_eq!(field_value(&values, "PROFILE.TAGS.2.NAME"), None);
    assert!(
        matches_filter(
            &values,
            serde_json::json!({"PROFILE.TAGS.1.NAME": "db"})
                .as_object()
                .unwrap()
        )
        .unwrap()
    );
    assert_eq!(
        compare_records(
            &first,
            &second,
            &IndexMap::from([(String::from("PROFILE.TAGS.1.NAME"), 1)])
        ),
        Ordering::Greater
    );

    let inclusion = BTreeMap::from([(String::from("PROFILE.TAGS.1.NAME"), 1)]);
    assert_eq!(
        project(&first, &inclusion),
        serde_json::json!({
            "PROFILE": {"TAGS": [null, {"NAME": "db"}]}
        })
    );

    let exclusion = BTreeMap::from([(String::from("PROFILE.TAGS.0.NAME"), 0)]);
    assert_eq!(
        project(&first, &exclusion),
        serde_json::json!({
            "PROFILE": {"TAGS": [{}, {"NAME": "db"}]}
        })
    );
}

#[test]
fn preserves_multi_key_sort_order() {
    let table = table_with_two_active_records();
    let request = parse(br#"{"sort":{"NAME":1,"AGE":1}}"#).unwrap();
    let records = execute_query(&table, &request).unwrap();

    assert_eq!(records[0]["NAME"], "Alice");
    assert_eq!(records[1]["NAME"], "Bob");
}

#[test]
fn supports_unicode_lowercase_collation_for_sort_keys() {
    let first = DbfRecord {
        number: 1,
        deleted: false,
        values: serde_json::json!({"NAME": "apple"})
            .as_object()
            .unwrap()
            .clone(),
    };
    let second = DbfRecord {
        number: 2,
        deleted: false,
        values: serde_json::json!({"NAME": "Banana"})
            .as_object()
            .unwrap()
            .clone(),
    };
    let sort = IndexMap::from([(String::from("NAME"), 1)]);

    assert_eq!(compare_records(&first, &second, &sort), Ordering::Greater);
    assert_eq!(
        compare_records_with_collation(&first, &second, &sort, Some(Collation::UnicodeLowercase)),
        Ordering::Less
    );
}

#[test]
fn supports_unicode_nfkc_lowercase_collation_for_cjk_compatibility_keys() {
    let fullwidth = Value::String("ＡＢＣ".into());
    let ascii = Value::String("abc".into());
    assert_eq!(
        super::ordering::compare_for_sort_with_collation(
            Some(&fullwidth),
            Some(&ascii),
            Some(Collation::UnicodeNfkcLowercase),
        ),
        Ordering::Equal
    );

    let halfwidth_katakana = Value::String("ｶﾀｶﾅ".into());
    let fullwidth_katakana = Value::String("カタカナ".into());
    assert_eq!(
        super::ordering::compare_for_sort_with_collation(
            Some(&halfwidth_katakana),
            Some(&fullwidth_katakana),
            Some(Collation::UnicodeNfkcLowercase),
        ),
        Ordering::Equal
    );
}

#[test]
fn validates_the_bounded_collation_contract() {
    let request = parse(br#"{"sort":{"NAME":1},"collation":"unicode-lowercase"}"#).unwrap();
    assert_eq!(request.collation, Some(Collation::UnicodeLowercase));
    let normalized_request =
        parse(br#"{"sort":{"NAME":1},"collation":"unicode-nfkc-lowercase"}"#).unwrap();
    assert_eq!(
        normalized_request.collation,
        Some(Collation::UnicodeNfkcLowercase)
    );
    assert!(parse(br#"{"collation":"unicode-lowercase"}"#).is_err());
    assert!(parse(br#"{"sort":{"NAME":1},"collation":"locale-aware"}"#).is_err());
}

#[test]
fn executes_logical_and_membership_predicates() {
    let table = table_with_two_active_records();
    let request = parse(br#"{"filter":{"$and":[{"AGE":{"$in":[29]}},{"ACTIVE":true}]}}"#).unwrap();

    assert_eq!(execute_query(&table, &request).unwrap().len(), 1);
}

#[test]
fn rejects_unknown_and_mixed_projection_operators() {
    assert!(parse(br#"{"filter":{"AGE":{"$regex":"2"}}}"#).is_err());
    assert!(parse(br#"{"projection":{"NAME":1,"AGE":0}}"#).is_err());
}

#[test]
fn rejects_unknown_query_fields() {
    assert!(parse(br#"{"filtre":{"AGE":29}}"#).is_err());
}

#[test]
fn compares_large_integer_values_exactly() {
    let maximum = Value::Number(Number::from(u64::MAX));
    let condition = serde_json::json!({"$gt": u64::MAX - 1});

    assert!(matches_condition(Some(&maximum), &condition).unwrap());
}
