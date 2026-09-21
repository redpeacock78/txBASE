use super::table_with_two_active_records;
use serde_json::json;

#[test]
fn aggregation_restores_physical_order_after_index_candidates() {
    let table = table_with_two_active_records();
    let request = crate::query::parse(
        br#"{
            "aggregate": [{"$group": {
                "_id": null,
                "first_value": {"$first": "$NAME"},
                "last_value": {"$last": "$NAME"}
            }}]
        }"#,
    )
    .unwrap();

    let page =
        crate::query::execute_query_with_records(&table, &request, Some(vec![2, 1]), 0).unwrap();

    assert_eq!(
        page.records,
        vec![json!({"_id": null, "first_value": "Alice", "last_value": "Bob"})]
    );
}

#[test]
fn missing_group_fields_share_the_null_group() {
    let table = table_with_two_active_records();
    let request = crate::query::parse(
        br#"{"aggregate":[{"$group":{"_id":"$MISSING","count":{"$count":{}}}}]}"#,
    )
    .unwrap();

    assert_eq!(
        crate::query::execute_query(&table, &request).unwrap(),
        vec![json!({"_id": null, "count": 2})]
    );
}

#[test]
fn applies_match_stages_before_grouping() {
    let table = table_with_two_active_records();
    let request = crate::query::parse(
        br#"{
            "aggregate": [
                {"$match": {"AGE": {"$gte": 20}}},
                {"$match": {"ACTIVE": true}},
                {"$group": {
                    "_id": null,
                    "count": {"$count": {}},
                    "total_age": {"$sum": "$AGE"}
                }}
            ]
        }"#,
    )
    .unwrap();

    assert_eq!(
        crate::query::execute_query(&table, &request).unwrap(),
        vec![json!({"_id": null, "count": 1, "total_age": 29})]
    );
}

#[test]
fn filters_group_output_before_projection_and_sorting() {
    let table = table_with_two_active_records();
    let request = crate::query::parse(
        br#"{
            "aggregate": [
                {"$group": {
                    "_id": "$AGE",
                    "count": {"$count": {}}
                }},
                {"$match": {"_id": {"$gte": 20}}},
                {"$project": {"_id": 1}},
                {"$sort": {"_id": -1}}
            ]
        }"#,
    )
    .unwrap();

    assert_eq!(
        crate::query::execute_query(&table, &request).unwrap(),
        vec![json!({"_id": 29})]
    );
}

#[test]
fn sorts_group_output_after_grouping() {
    let table = table_with_two_active_records();
    let request = crate::query::parse(
        br#"{
            "aggregate": [
                {"$group": {
                    "_id": "$AGE",
                    "count": {"$count": {}}
                }},
                {"$sort": {"_id": -1}}
            ]
        }"#,
    )
    .unwrap();

    assert_eq!(
        crate::query::execute_query(&table, &request).unwrap(),
        vec![
            json!({"_id": 29, "count": 1}),
            json!({"_id": 7, "count": 1})
        ]
    );
}

#[test]
fn limits_sorted_group_output() {
    let table = table_with_two_active_records();
    let request = crate::query::parse(
        br#"{
            "aggregate": [
                {"$group": {
                    "_id": "$AGE",
                    "count": {"$count": {}}
                }},
                {"$sort": {"_id": -1}},
                {"$limit": 1}
            ]
        }"#,
    )
    .unwrap();

    assert_eq!(
        crate::query::execute_query(&table, &request).unwrap(),
        vec![json!({"_id": 29, "count": 1})]
    );
}

#[test]
fn projects_group_output_before_sorting() {
    let table = table_with_two_active_records();
    let request = crate::query::parse(
        br#"{
            "aggregate": [
                {"$group": {
                    "_id": "$AGE",
                    "count": {"$count": {}}
                }},
                {"$project": {"_id": 1}},
                {"$sort": {"_id": -1}}
            ]
        }"#,
    )
    .unwrap();

    assert_eq!(
        crate::query::execute_query(&table, &request).unwrap(),
        vec![json!({"_id": 29}), json!({"_id": 7})]
    );
}

#[test]
fn rejects_unsupported_aggregation_combinations() {
    for body in [
        br#"{"aggregate":[]}"#.as_slice(),
        br#"{"sort":{"AGE":1},"aggregate":[{"$group":{"_id":null}}]}"#.as_slice(),
        br#"{"aggregate":[{"$count":{}}]}"#.as_slice(),
        br#"{"aggregate":[{"$count":""}]}"#.as_slice(),
        br#"{"aggregate":[{"$count":"total.value"}]}"#.as_slice(),
        br#"{"aggregate":[{"$count":"total"},{"$match":{}}]}"#.as_slice(),
        br#"{"aggregate":[{"$count":"total"},{"$group":{"_id":null}}]}"#.as_slice(),
        br#"{"aggregate":[{"$distinct":"ACTIVE"}]}"#.as_slice(),
        br#"{"aggregate":[{"$distinct":"$ACTIVE"},{"$limit":1}]}"#.as_slice(),
        br#"{"aggregate":[{"$distinct":"$ACTIVE"},{"$distinct":"$AGE"}]}"#.as_slice(),
        br#"{"aggregate":[{"$group":{"_id":null,"total":{"$sum":"AGE"}}}]}"#.as_slice(),
        br#"{"aggregate":[{"$group":{"_id":null,"total":{"$sum":true}}}]}"#.as_slice(),
        br#"{"aggregate":[{"$group":{"_id":null,"average":{"$avg":"AGE"}}}]}"#.as_slice(),
        br#"{"aggregate":[{"$group":{"_id":null}},{"$project":{"_id":1}},{"$match":{}}]}"#
            .as_slice(),
        br#"{"aggregate":[{"$sort":{"_id":1}},{"$group":{"_id":null}}]}"#.as_slice(),
        br#"{"aggregate":[{"$group":{"_id":null}},{"$sort":{}}]}"#.as_slice(),
        br#"{"aggregate":[{"$group":{"_id":null}},{"$sort":{"_id":2}}]}"#.as_slice(),
        br#"{"aggregate":[{"$group":{"_id":null}},{"$sort":{"_id":1}},{"$sort":{"_id":1}}]}"#
            .as_slice(),
        br#"{"aggregate":[{"$limit":1},{"$group":{"_id":null}}]}"#.as_slice(),
        br#"{"aggregate":[{"$group":{"_id":null}},{"$limit":-1}]}"#.as_slice(),
        br#"{"aggregate":[{"$group":{"_id":null}},{"$limit":1},{"$limit":1}]}"#.as_slice(),
        br#"{"aggregate":[{"$group":{"_id":null}},{"$limit":1},{"$sort":{"_id":1}}]}"#.as_slice(),
        br#"{"aggregate":[{"$project":{"_id":1}},{"$group":{"_id":null}}]}"#.as_slice(),
        br#"{"aggregate":[{"$group":{"_id":null}},{"$project":{"_id":1,"count":0}}]}"#.as_slice(),
        br#"{"aggregate":[{"$group":{"_id":null}},{"$sort":{"_id":1}},{"$project":{"_id":1}}]}"#
            .as_slice(),
        br#"{"aggregate":[{"$group":{"_id":null}},{"$project":{"_id":1}},{"$project":{"_id":1}}]}"#
            .as_slice(),
    ] {
        assert!(
            crate::query::parse(body).is_err(),
            "expected rejection for {body:?}"
        );
    }
}
