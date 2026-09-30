use crate::dbf::DbfRecord;
use serde_json::json;

#[test]
fn selects_n_values_from_scalar_and_array_expressions() {
    let records = [
        json!({"VALUE": 3, "NAME": "C"}),
        json!({"VALUE": 1, "NAME": "A"}),
        json!({"VALUE": 2, "NAME": "B"}),
        json!({"VALUE": 2, "NAME": "B"}),
        json!({"VALUE": null, "NAME": "null"}),
        json!({"NAME": "missing"}),
    ]
    .into_iter()
    .enumerate()
    .map(|(number, values)| DbfRecord {
        number,
        deleted: false,
        values: values.as_object().unwrap().clone(),
    })
    .collect::<Vec<_>>();
    let references = records.iter().collect::<Vec<_>>();
    let stages = vec![
        json!({
            "$group": {
                "_id": null,
                "minimum_pairs": {"$minN": {"input": ["$VALUE", "$NAME"], "n": 2}},
                "maximum_values": {"$maxN": {"input": "$VALUE", "n": 2}},
                "duplicates": {"$minN": {"input": "$VALUE", "n": 3}},
                "empty": {"$maxN": {"input": "$MISSING", "n": 1}},
                "first_values": {"$firstN": {"input": "$VALUE", "n": 2}},
                "last_pairs": {"$lastN": {"input": ["$VALUE", "$NAME"], "n": 2}}
            }
        })
        .as_object()
        .unwrap()
        .clone(),
    ];

    assert_eq!(
        crate::query::aggregation::execute(&references, &stages).unwrap(),
        vec![json!({
            "_id": null,
            "minimum_pairs": [[null, "missing"], [null, "null"]],
            "maximum_values": [3, 2],
            "duplicates": [1, 2, 2],
            "empty": [],
            "first_values": [3, 1],
            "last_pairs": [[null, "null"], [null, "missing"]]
        })]
    );
}

#[test]
fn evaluates_n_from_the_group_id_once_per_group() {
    let records = [
        json!({"GROUP": "wide", "VALUE": 3}),
        json!({"GROUP": "wide", "VALUE": 1}),
        json!({"GROUP": "wide", "VALUE": 2}),
        json!({"GROUP": "narrow", "VALUE": 7}),
        json!({"GROUP": "narrow", "VALUE": 8}),
    ]
    .into_iter()
    .enumerate()
    .map(|(number, values)| DbfRecord {
        number,
        deleted: false,
        values: values.as_object().unwrap().clone(),
    })
    .collect::<Vec<_>>();
    let references = records.iter().collect::<Vec<_>>();
    let stages = vec![
        json!({
            "$group": {
                "_id": "$GROUP",
                "selected": {
                    "$minN": {
                        "input": "$VALUE",
                        "n": {
                            "$cond": {
                                "if": {"$eq": ["$_id", "wide"]},
                                "then": 2,
                                "else": 1
                            }
                        }
                    }
                },
                "first_values": {
                    "$firstN": {
                        "input": "$VALUE",
                        "n": {
                            "$cond": {
                                "if": {"$eq": ["$_id", "wide"]},
                                "then": 2,
                                "else": 1
                            }
                        }
                    }
                },
                "last_values": {
                    "$lastN": {
                        "input": "$VALUE",
                        "n": {
                            "$cond": {
                                "if": {"$eq": ["$_id", "wide"]},
                                "then": 2,
                                "else": 1
                            }
                        }
                    }
                }
            }
        })
        .as_object()
        .unwrap()
        .clone(),
    ];

    assert_eq!(
        crate::query::aggregation::execute(&references, &stages).unwrap(),
        vec![
            json!({"_id": "narrow", "selected": [7], "first_values": [7], "last_values": [8]}),
            json!({"_id": "wide", "selected": [1, 2], "first_values": [3, 1], "last_values": [1, 2]})
        ]
    );
}

#[test]
fn orders_n_values_across_json_types_and_nested_values() {
    let records = [
        json!({"VALUE": false}),
        json!({"VALUE": [1]}),
        json!({"VALUE": {}}),
        json!({"VALUE": "one"}),
        json!({"VALUE": 1}),
    ]
    .into_iter()
    .enumerate()
    .map(|(number, values)| DbfRecord {
        number,
        deleted: false,
        values: values.as_object().unwrap().clone(),
    })
    .collect::<Vec<_>>();
    let references = records.iter().collect::<Vec<_>>();
    let stages = vec![
        json!({
            "$group": {
                "_id": null,
                "minimum": {"$minN": {"input": "$VALUE", "n": 5}},
                "maximum": {"$maxN": {"input": "$VALUE", "n": 5}}
            }
        })
        .as_object()
        .unwrap()
        .clone(),
    ];

    assert_eq!(
        crate::query::aggregation::execute(&references, &stages).unwrap(),
        vec![json!({
            "_id": null,
            "minimum": [1, "one", {}, [1], false],
            "maximum": [false, [1], {}, "one", 1]
        })]
    );
}

#[test]
fn preserves_input_order_for_equal_numeric_values() {
    let records = [json!({"VALUE": 1}), json!({"VALUE": 1.0})]
        .into_iter()
        .enumerate()
        .map(|(number, values)| DbfRecord {
            number,
            deleted: false,
            values: values.as_object().unwrap().clone(),
        })
        .collect::<Vec<_>>();
    let references = records.iter().collect::<Vec<_>>();
    let stages = vec![
        json!({
            "$group": {
                "_id": null,
                "values": {"$minN": {"input": "$VALUE", "n": 2}}
            }
        })
        .as_object()
        .unwrap()
        .clone(),
    ];

    let result = crate::query::aggregation::execute(&references, &stages).unwrap();
    let values = result[0]["values"].as_array().unwrap();
    assert_eq!(values[0].to_string(), "1");
    assert_eq!(values[1].to_string(), "1.0");
}

#[test]
fn rejects_invalid_n_accumulator_shapes_and_limits() {
    let record = DbfRecord {
        number: 1,
        deleted: false,
        values: serde_json::Map::new(),
    };
    let records = [&record];
    let invalid_stages = [
        json!({"$group": {"_id": null, "values": {"$minN": {"input": "$VALUE"}}}}),
        json!({"$group": {"_id": null, "values": {"$minN": {"input": "$VALUE", "n": 0}}}}),
        json!({"$group": {"_id": null, "values": {"$maxN": {"input": "$VALUE", "n": 1.5}}}}),
        json!({"$group": {"_id": null, "values": {"$minN": {"input": "$VALUE", "n": 10001}}}}),
        json!({"$group": {"_id": null, "values": {"$maxN": {"input": "$VALUE", "n": "$VALUE"}}}}),
        json!({"$group": {"_id": null, "values": {"$firstN": {"input": "$VALUE"}}}}),
        json!({"$group": {"_id": null, "values": {"$firstN": {"input": "$VALUE", "n": 1, "unused": true}}}}),
        json!({"$group": {"_id": null, "values": {"$lastN": {"input": "$VALUE", "n": 0}}}}),
        json!({"$group": {"_id": null, "values": {"$firstN": {"input": "$VALUE", "n": 10001}}}}),
        json!({"$group": {"_id": null, "values": {"$lastN": {"input": "$VALUE", "n": "$VALUE"}}}}),
    ];

    for stage in invalid_stages {
        let stages = vec![stage.as_object().unwrap().clone()];
        assert!(crate::query::aggregation::execute(&records, &stages).is_err());
    }
}

#[test]
fn keeps_n_accumulators_bounded_when_input_exceeds_the_value_limit() {
    let records = (0..=crate::query::aggregation::MAX_COLLECTED_VALUES)
        .map(|number| DbfRecord {
            number,
            deleted: false,
            values: json!({"VALUE": number}).as_object().unwrap().clone(),
        })
        .collect::<Vec<_>>();
    let references = records.iter().collect::<Vec<_>>();
    let stages = vec![
        json!({
            "$group": {
                "_id": null,
                "minimum": {"$minN": {"input": "$VALUE", "n": 1}},
                "first": {"$firstN": {"input": "$VALUE", "n": 1}},
                "last": {"$lastN": {"input": "$VALUE", "n": 1}}
            }
        })
        .as_object()
        .unwrap()
        .clone(),
    ];

    assert_eq!(
        crate::query::aggregation::execute(&references, &stages).unwrap(),
        vec![json!({"_id": null, "minimum": [0], "first": [0], "last": [10000]})]
    );
}

#[test]
fn groups_first_and_last_values_in_physical_order() {
    let first = DbfRecord {
        number: 1,
        deleted: false,
        values: json!({"VALUE": "first"}).as_object().unwrap().clone(),
    };
    let second = DbfRecord {
        number: 2,
        deleted: false,
        values: json!({"VALUE": "last"}).as_object().unwrap().clone(),
    };
    let records = [&first, &second];
    let stages = vec![
        json!({
            "$group": {
                "_id": null,
                "first_value": {"$first": "$VALUE"},
                "last_value": {"$last": "$VALUE"},
                "missing_value": {"$first": "$MISSING"},
                "first_values": {"$firstN": {"input": "$VALUE", "n": 2}},
                "last_values": {"$lastN": {"input": "$VALUE", "n": 2}}
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
            "first_value": "first",
            "last_value": "last",
            "missing_value": null,
            "first_values": ["first", "last"],
            "last_values": ["first", "last"]
        })]
    );
}

#[test]
fn first_n_and_last_n_keep_null_and_missing_inputs_in_order() {
    let records = [
        json!({"VALUE": "first"}),
        json!({"VALUE": null}),
        json!({}),
        json!({"VALUE": "last"}),
    ]
    .into_iter()
    .enumerate()
    .map(|(number, values)| DbfRecord {
        number,
        deleted: false,
        values: values.as_object().unwrap().clone(),
    })
    .collect::<Vec<_>>();
    let stages = vec![
        json!({
            "$group": {
                "_id": null,
                "first_values": {"$firstN": {"input": "$VALUE", "n": 3}},
                "last_values": {"$lastN": {"input": "$VALUE", "n": 3}}
            }
        })
        .as_object()
        .unwrap()
        .clone(),
    ];

    assert_eq!(
        crate::query::aggregation::execute(&records.iter().collect::<Vec<_>>(), &stages).unwrap(),
        vec![json!({
            "_id": null,
            "first_values": ["first", null, null],
            "last_values": [null, null, "last"]
        })]
    );
}

#[test]
fn first_n_and_last_n_follow_prior_input_sort_order() {
    let records = [
        json!({"VALUE": 30}),
        json!({"VALUE": 10}),
        json!({"VALUE": 20}),
    ]
    .into_iter()
    .enumerate()
    .map(|(number, values)| DbfRecord {
        number,
        deleted: false,
        values: values.as_object().unwrap().clone(),
    })
    .collect::<Vec<_>>();
    let stages = vec![
        json!({"$sort": {"VALUE": 1}}).as_object().unwrap().clone(),
        json!({
            "$group": {
                "_id": null,
                "first_values": {"$firstN": {"input": "$VALUE", "n": 2}},
                "last_values": {"$lastN": {"input": "$VALUE", "n": 2}}
            }
        })
        .as_object()
        .unwrap()
        .clone(),
    ];

    assert_eq!(
        crate::query::aggregation::execute(&records.iter().collect::<Vec<_>>(), &stages).unwrap(),
        vec![json!({"_id": null, "first_values": [10, 20], "last_values": [20, 30]})]
    );
}
