use super::table_with_two_active_records;
use crate::dbf::DbfRecord;
use serde_json::json;

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
fn applies_input_stages_in_listed_order() {
    let first = DbfRecord {
        number: 1,
        deleted: false,
        values: json!({"AGE": 29}).as_object().unwrap().clone(),
    };
    let second = DbfRecord {
        number: 2,
        deleted: false,
        values: json!({"AGE": 7}).as_object().unwrap().clone(),
    };
    let third = DbfRecord {
        number: 3,
        deleted: false,
        values: json!({"AGE": 20}).as_object().unwrap().clone(),
    };
    let records = [&first, &second, &third];
    let stages = vec![
        json!({"$limit": 2}).as_object().unwrap().clone(),
        json!({"$sort": {"AGE": -1}}).as_object().unwrap().clone(),
        json!({"$skip": 1}).as_object().unwrap().clone(),
        json!({
            "$group": {
                "_id": null,
                "count": {"$count": {}},
                "first_age": {"$first": "$AGE"},
                "last_age": {"$last": "$AGE"}
            }
        })
        .as_object()
        .unwrap()
        .clone(),
    ];

    assert_eq!(
        crate::query::aggregation::execute(&records, &stages).unwrap(),
        vec![json!({"_id": null, "count": 1, "first_age": 7, "last_age": 7})]
    );
}

#[test]
fn projects_input_records_before_matching_and_grouping() {
    let first = DbfRecord {
        number: 1,
        deleted: false,
        values: json!({"COUNTRY": "JP", "AGE": 29, "SECRET": "hidden"})
            .as_object()
            .unwrap()
            .clone(),
    };
    let second = DbfRecord {
        number: 2,
        deleted: false,
        values: json!({"COUNTRY": "JP", "AGE": 7, "SECRET": "hidden"})
            .as_object()
            .unwrap()
            .clone(),
    };
    let records = [&first, &second];
    let stages = vec![
        json!({"$project": {"COUNTRY": 1, "AGE": 1}})
            .as_object()
            .unwrap()
            .clone(),
        json!({"$match": {"AGE": {"$gte": 20}}})
            .as_object()
            .unwrap()
            .clone(),
        json!({
            "$group": {
                "_id": "$COUNTRY",
                "total_age": {"$sum": "$AGE"},
                "secret": {"$first": "$SECRET"}
            }
        })
        .as_object()
        .unwrap()
        .clone(),
    ];

    assert_eq!(
        crate::query::aggregation::execute(&records, &stages).unwrap(),
        vec![json!({"_id": "JP", "total_age": 29, "secret": null})]
    );
}

#[test]
fn sets_computed_fields_before_matching_and_grouping() {
    let first = DbfRecord {
        number: 1,
        deleted: false,
        values: json!({"PRICE": 5, "QUANTITY": 3, "EXPLICIT_NULL": null})
            .as_object()
            .unwrap()
            .clone(),
    };
    let second = DbfRecord {
        number: 2,
        deleted: false,
        values: json!({"PRICE": 2, "QUANTITY": 4, "LABEL": "sale"})
            .as_object()
            .unwrap()
            .clone(),
    };
    let records = [&first, &second];
    let stages = vec![
        json!({
            "$set": {
                "TOTAL": {"$multiply": ["$PRICE", "$QUANTITY"]},
                "LABEL": {"$ifNull": ["$LABEL", "unknown"]},
                "EXPLICIT": {"$ifNull": ["$EXPLICIT_NULL", "fallback"]},
                "REFERENCE": "$PRICE",
                "DOLLAR": {"$literal": "$PRICE"},
                "SAME_STAGE": "$TOTAL"
            }
        })
        .as_object()
        .unwrap()
        .clone(),
        json!({"$match": {"TOTAL": {"$gte": 10}}})
            .as_object()
            .unwrap()
            .clone(),
        json!({
            "$group": {
                "_id": null,
                "total": {"$sum": "$TOTAL"},
                "label": {"$first": "$LABEL"},
                "explicit": {"$first": "$EXPLICIT"},
                "reference": {"$first": "$REFERENCE"},
                "dollar": {"$first": "$DOLLAR"},
                "same_stage": {"$first": "$SAME_STAGE"}
            }
        })
        .as_object()
        .unwrap()
        .clone(),
    ];

    assert_eq!(
        crate::query::aggregation::execute(&records, &stages).unwrap(),
        vec![json!({
            "_id": null,
            "total": 15,
            "label": "unknown",
            "explicit": "fallback",
            "reference": 5,
            "dollar": "$PRICE",
            "same_stage": null
        })]
    );
}

#[test]
fn sets_rounded_fields_and_nulls_non_numeric_values() {
    let positive = DbfRecord {
        number: 1,
        deleted: false,
        values: json!({"VALUE": 2.8}).as_object().unwrap().clone(),
    };
    let negative = DbfRecord {
        number: 2,
        deleted: false,
        values: json!({"VALUE": -2.8}).as_object().unwrap().clone(),
    };
    let non_numeric = DbfRecord {
        number: 3,
        deleted: false,
        values: json!({"VALUE": "ignored"}).as_object().unwrap().clone(),
    };
    let records = [&positive, &negative, &non_numeric];
    let stages = vec![
        json!({
            "$set": {
                "CEILING": {"$ceil": "$VALUE"},
                "FLOOR": {"$floor": "$VALUE"}
            }
        })
        .as_object()
        .unwrap()
        .clone(),
        json!({
            "$group": {
                "_id": null,
                "ceilings": {"$push": "$CEILING"},
                "floors": {"$push": "$FLOOR"}
            }
        })
        .as_object()
        .unwrap()
        .clone(),
    ];

    assert_eq!(
        crate::query::aggregation::execute(&records, &stages).unwrap(),
        vec![json!({
            "_id": null,
            "ceilings": [3, -2, null],
            "floors": [2, -3, null]
        })]
    );
}

#[test]
fn evaluates_conditional_expressions_in_input_fields_and_group_keys() {
    let adult = DbfRecord {
        number: 1,
        deleted: false,
        values: json!({"AGE": 29}).as_object().unwrap().clone(),
    };
    let minor = DbfRecord {
        number: 2,
        deleted: false,
        values: json!({"AGE": 7}).as_object().unwrap().clone(),
    };
    let records = [&adult, &minor];
    let stages = vec![
        json!({"$set": {
            "STATUS": {"$cond": {
                "if": {"$gte": ["$AGE", 18]},
                "then": {"$literal": "adult"},
                "else": {"$literal": "minor"}
            }}
        }})
        .as_object()
        .unwrap()
        .clone(),
        json!({"$group": {
            "_id": {"$cond": [
                {"$eq": ["$STATUS", "adult"]},
                "adult",
                "minor"
            ]},
            "status": {"$first": "$STATUS"},
            "count": {"$count": {}}
        }})
        .as_object()
        .unwrap()
        .clone(),
        json!({"$sort": {"_id": 1}}).as_object().unwrap().clone(),
    ];

    assert_eq!(
        crate::query::aggregation::execute(&records, &stages).unwrap(),
        vec![
            json!({"_id": "adult", "status": "adult", "count": 1}),
            json!({"_id": "minor", "status": "minor", "count": 1})
        ]
    );
}

#[test]
fn add_fields_alias_sets_input_fields() {
    let record = DbfRecord {
        number: 1,
        deleted: false,
        values: json!({"NAME": "Alice"}).as_object().unwrap().clone(),
    };
    let records = [&record];
    let stages = vec![
        json!({"$addFields": {"COPY": "$NAME"}})
            .as_object()
            .unwrap()
            .clone(),
        json!({"$group": {"_id": null, "name": {"$first": "$COPY"}}})
            .as_object()
            .unwrap()
            .clone(),
    ];

    assert_eq!(
        crate::query::aggregation::execute(&records, &stages).unwrap(),
        vec![json!({"_id": null, "name": "Alice"})]
    );
}

#[test]
fn sets_array_scalar_expressions() {
    let record = DbfRecord {
        number: 1,
        deleted: false,
        values: json!({"VALUE": 1}).as_object().unwrap().clone(),
    };
    let records = [&record];
    let stages = vec![
        json!({"$set": {"PAIR": ["$VALUE", "$MISSING"]}})
            .as_object()
            .unwrap()
            .clone(),
        json!({"$group": {"_id": null, "pair": {"$first": "$PAIR"}}})
            .as_object()
            .unwrap()
            .clone(),
    ];

    assert_eq!(
        crate::query::aggregation::execute(&records, &stages).unwrap(),
        vec![json!({"_id": null, "pair": [1, null]})]
    );
}

#[test]
fn sets_string_scalar_expressions_before_matching_and_grouping() {
    let first = DbfRecord {
        number: 1,
        deleted: false,
        values: json!({"FIRST": "Alice", "LAST": "Smith"})
            .as_object()
            .unwrap()
            .clone(),
    };
    let second = DbfRecord {
        number: 2,
        deleted: false,
        values: json!({"FIRST": "Bob"}).as_object().unwrap().clone(),
    };
    let records = [&first, &second];
    let stages = vec![
        json!({
            "$set": {
                "DISPLAY": {"$concat": [
                    {"$toUpper": "$FIRST"},
                    " ",
                    {"$ifNull": ["$LAST", {"$literal": "UNKNOWN"}]}
                ]},
                "FIRST_LENGTH": {"$strLenCP": "$FIRST"},
                "FIRST_PREFIX": {"$substrCP": ["$FIRST", 0, 2]},
                "FIRST_PARTS": {"$split": ["$FIRST", "i"]},
                "FIRST_INDEX": {"$indexOfCP": ["$FIRST", "i"]},
                "FIRST_ONCE": {"$replaceOne": {
                    "input": "$FIRST", "find": "i", "replacement": "!"
                }},
                "FIRST_ALL": {"$replaceAll": {
                    "input": "$FIRST", "find": "i", "replacement": "!"
                }}
            }
        })
        .as_object()
        .unwrap()
        .clone(),
        json!({"$group": {
            "_id": null,
            "display": {"$push": "$DISPLAY"},
            "length": {"$push": "$FIRST_LENGTH"},
            "prefix": {"$push": "$FIRST_PREFIX"},
            "parts": {"$push": "$FIRST_PARTS"},
            "index": {"$push": "$FIRST_INDEX"},
            "once": {"$push": "$FIRST_ONCE"},
            "all": {"$push": "$FIRST_ALL"}
        }})
        .as_object()
        .unwrap()
        .clone(),
    ];

    assert_eq!(
        crate::query::aggregation::execute(&records, &stages).unwrap(),
        vec![json!({
            "_id": null,
            "display": ["ALICE Smith", "BOB UNKNOWN"],
            "length": [5, 3],
            "prefix": ["Al", "Bo"],
            "parts": [["Al", "ce"], ["Bob"]],
            "index": [2, -1],
            "once": ["Al!ce", "Bob"],
            "all": ["Al!ce", "Bob"]
        })]
    );
}

#[test]
fn projects_input_fields_before_distinct() {
    let first = DbfRecord {
        number: 1,
        deleted: false,
        values: json!({"COUNTRY": "JP", "SECRET": "one"})
            .as_object()
            .unwrap()
            .clone(),
    };
    let second = DbfRecord {
        number: 2,
        deleted: false,
        values: json!({"COUNTRY": "US", "SECRET": "two"})
            .as_object()
            .unwrap()
            .clone(),
    };
    let records = [&first, &second];
    let stages = vec![
        json!({"$project": {"SECRET": 0}})
            .as_object()
            .unwrap()
            .clone(),
        json!({"$distinct": "$SECRET"}).as_object().unwrap().clone(),
    ];

    assert_eq!(
        crate::query::aggregation::execute(&records, &stages).unwrap(),
        vec![json!(null)]
    );
}

#[test]
fn unwinds_array_values_before_grouping() {
    let first = DbfRecord {
        number: 1,
        deleted: false,
        values: json!({"TAGS": ["a", "b"]}).as_object().unwrap().clone(),
    };
    let second = DbfRecord {
        number: 2,
        deleted: false,
        values: json!({"TAGS": ["b"]}).as_object().unwrap().clone(),
    };
    let empty = DbfRecord {
        number: 3,
        deleted: false,
        values: json!({"TAGS": []}).as_object().unwrap().clone(),
    };
    let null = DbfRecord {
        number: 4,
        deleted: false,
        values: json!({"TAGS": null}).as_object().unwrap().clone(),
    };
    let missing = DbfRecord {
        number: 5,
        deleted: false,
        values: json!({"NAME": "missing"}).as_object().unwrap().clone(),
    };
    let records = [&first, &second, &empty, &null, &missing];
    let stages = vec![
        json!({"$unwind": "$TAGS"}).as_object().unwrap().clone(),
        json!({
            "$group": {
                "_id": "$TAGS",
                "count": {"$count": {}}
            }
        })
        .as_object()
        .unwrap()
        .clone(),
    ];

    assert_eq!(
        crate::query::aggregation::execute(&records, &stages).unwrap(),
        vec![
            json!({"_id": "a", "count": 1}),
            json!({"_id": "b", "count": 2})
        ]
    );
}

#[test]
fn includes_unwind_array_indexes() {
    let record = DbfRecord {
        number: 1,
        deleted: false,
        values: json!({"TAGS": ["a", "b"]}).as_object().unwrap().clone(),
    };
    let records = [&record];
    let stages = vec![
        json!({
            "$unwind": {
                "path": "$TAGS",
                "includeArrayIndex": "TAG_INDEX"
            }
        })
        .as_object()
        .unwrap()
        .clone(),
        json!({
            "$group": {
                "_id": null,
                "tags": {"$push": "$TAGS"},
                "indexes": {"$push": "$TAG_INDEX"}
            }
        })
        .as_object()
        .unwrap()
        .clone(),
    ];

    assert_eq!(
        crate::query::aggregation::execute(&records, &stages).unwrap(),
        vec![json!({"_id": null, "tags": ["a", "b"], "indexes": [0, 1]})]
    );
}

#[test]
fn preserves_null_and_empty_unwind_inputs_with_null_indexes() {
    let array = DbfRecord {
        number: 1,
        deleted: false,
        values: json!({"TAGS": ["a", "b"]}).as_object().unwrap().clone(),
    };
    let empty = DbfRecord {
        number: 2,
        deleted: false,
        values: json!({"TAGS": []}).as_object().unwrap().clone(),
    };
    let null = DbfRecord {
        number: 3,
        deleted: false,
        values: json!({"TAGS": null}).as_object().unwrap().clone(),
    };
    let missing = DbfRecord {
        number: 4,
        deleted: false,
        values: json!({"NAME": "missing"}).as_object().unwrap().clone(),
    };
    let records = [&array, &empty, &null, &missing];
    let stages = vec![
        json!({
            "$unwind": {
                "path": "$TAGS",
                "includeArrayIndex": "TAG_INDEX",
                "preserveNullAndEmptyArrays": true
            }
        })
        .as_object()
        .unwrap()
        .clone(),
        json!({
            "$group": {
                "_id": null,
                "count": {"$count": {}},
                "tags": {"$push": "$TAGS"},
                "indexes": {"$push": "$TAG_INDEX"}
            }
        })
        .as_object()
        .unwrap()
        .clone(),
    ];

    assert_eq!(
        crate::query::aggregation::execute(&records, &stages).unwrap(),
        vec![json!({
            "_id": null,
            "count": 5,
            "tags": ["a", "b", null, null, null],
            "indexes": [0, 1, null, null, null]
        })]
    );
}

#[test]
fn preserves_unwind_order_for_array_accumulators() {
    let first = DbfRecord {
        number: 1,
        deleted: false,
        values: json!({"TAGS": ["a", "b"], "LABELS": ["x", "y"]})
            .as_object()
            .unwrap()
            .clone(),
    };
    let second = DbfRecord {
        number: 2,
        deleted: false,
        values: json!({"TAGS": ["b"], "LABELS": ["z"]})
            .as_object()
            .unwrap()
            .clone(),
    };
    let records = [&first, &second];
    let stages = vec![
        json!({"$unwind": "$TAGS"}).as_object().unwrap().clone(),
        json!({"$unwind": "$LABELS"}).as_object().unwrap().clone(),
        json!({
            "$group": {
                "_id": null,
                "count": {"$count": {}},
                "tags": {"$push": "$TAGS"}
            }
        })
        .as_object()
        .unwrap()
        .clone(),
    ];

    assert_eq!(
        crate::query::aggregation::execute(&records, &stages).unwrap(),
        vec![json!({"_id": null, "count": 5, "tags": ["a", "a", "b", "b", "b"]})]
    );
}

#[test]
fn filters_unwound_records_before_grouping() {
    let first = DbfRecord {
        number: 1,
        deleted: false,
        values: json!({"TAGS": ["a", "b"]}).as_object().unwrap().clone(),
    };
    let second = DbfRecord {
        number: 2,
        deleted: false,
        values: json!({"TAGS": ["b"]}).as_object().unwrap().clone(),
    };
    let records = [&first, &second];
    let stages = vec![
        json!({"$unwind": "$TAGS"}).as_object().unwrap().clone(),
        json!({"$match": {"TAGS": "b"}})
            .as_object()
            .unwrap()
            .clone(),
        json!({"$group": {"_id": null, "count": {"$count": {}}}})
            .as_object()
            .unwrap()
            .clone(),
    ];

    assert_eq!(
        crate::query::aggregation::execute(&records, &stages).unwrap(),
        vec![json!({"_id": null, "count": 2})]
    );
}

#[test]
fn rejects_non_array_unwind_values() {
    let record = DbfRecord {
        number: 1,
        deleted: false,
        values: json!({"TAGS": "not-an-array"}).as_object().unwrap().clone(),
    };
    let records = [&record];
    let stages = vec![
        json!({"$unwind": "$TAGS"}).as_object().unwrap().clone(),
        json!({"$count": "total"}).as_object().unwrap().clone(),
    ];

    let error = crate::query::aggregation::execute(&records, &stages).unwrap_err();
    assert!(error.to_string().contains("must be an array"));
}

#[test]
fn bounds_unwound_records() {
    let record = DbfRecord {
        number: 1,
        deleted: false,
        values: json!({
            "TAGS": (0..(crate::query::aggregation::MAX_UNWOUND_RECORDS / 2))
                .map(|value| json!(value))
                .collect::<Vec<_>>(),
            "LABELS": [0, 1]
        })
        .as_object()
        .unwrap()
        .clone(),
    };
    let records = [&record];
    let stages = vec![
        json!({"$unwind": "$TAGS"}).as_object().unwrap().clone(),
        json!({"$unwind": "$LABELS"}).as_object().unwrap().clone(),
        json!({"$count": "total"}).as_object().unwrap().clone(),
    ];

    let error = crate::query::aggregation::execute(&records, &stages).unwrap_err();
    assert!(error.to_string().contains("unwound record count"));
}
