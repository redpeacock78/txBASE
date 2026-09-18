use super::*;

fn fixture() -> Vec<u8> {
    include_str!("../../tests/fixtures/users.dbf.hex")
        .split_whitespace()
        .map(|token| u8::from_str_radix(token, 16).unwrap())
        .collect()
}

fn active_model(model: &[Option<Map<String, Value>>]) -> Vec<Value> {
    model
        .iter()
        .filter_map(|values| values.clone().map(Value::Object))
        .collect()
}

fn assert_matches_model(table: &DbfTable, model: &[Option<Map<String, Value>>]) {
    assert_eq!(table.active_json(), active_model(model));
}

#[test]
fn generated_mutation_sequence_matches_reference_model() {
    let mut table = DbfTable::from_bytes(&fixture()).unwrap();
    let mut model = table
        .records()
        .iter()
        .map(|record| (!record.deleted).then(|| record.values.clone()))
        .collect::<Vec<_>>();

    for step in 0..32 {
        let active_numbers = model
            .iter()
            .enumerate()
            .filter_map(|(index, values)| values.as_ref().map(|_| index + 1))
            .collect::<Vec<_>>();
        let number = active_numbers[step % active_numbers.len()];

        match step % 5 {
            0 => {
                let values = serde_json::json!({"NAME": format!("N{step}")})
                    .as_object()
                    .unwrap()
                    .clone();
                table.patch_record(number, values.clone()).unwrap();
                model[number - 1].as_mut().unwrap().extend(values);
            }
            1 => {
                let values = serde_json::json!({"AGE": step + 10})
                    .as_object()
                    .unwrap()
                    .clone();
                table.patch_record(number, values.clone()).unwrap();
                model[number - 1].as_mut().unwrap().extend(values);
            }
            2 => {
                let values = serde_json::json!({
                    "ID": 3,
                    "NAME": format!("I{step}"),
                    "AGE": step + 20,
                    "ACTIVE": step % 2 == 0
                })
                .as_object()
                .unwrap()
                .clone();
                let inserted = table.insert_record(values.clone()).unwrap();
                assert_eq!(inserted, model.len() + 1);
                model.push(Some(values));
            }
            3 => {
                let id = model[number - 1].as_ref().unwrap()["ID"].clone();
                let values = serde_json::json!({
                    "ID": id,
                    "NAME": format!("R{step}"),
                    "AGE": step + 30,
                    "ACTIVE": step % 2 != 0
                })
                .as_object()
                .unwrap()
                .clone();
                table.replace_record(number, values.clone()).unwrap();
                model[number - 1] = Some(values);
            }
            4 if active_numbers.len() > 1 => {
                table.delete_record(number).unwrap();
                model[number - 1] = None;
            }
            4 => {
                let values = serde_json::json!({"AGE": step + 40})
                    .as_object()
                    .unwrap()
                    .clone();
                table.patch_record(number, values.clone()).unwrap();
                model[number - 1].as_mut().unwrap().extend(values);
            }
            _ => unreachable!(),
        }

        assert_matches_model(&table, &model);
    }

    let reloaded = DbfTable::from_bytes(&table.to_bytes()).unwrap();
    assert_matches_model(&reloaded, &model);
}
