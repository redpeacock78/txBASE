use serde_json::{Map, Value};

pub(super) fn apply_merge_patch(
    current: Map<String, Value>,
    patch: Map<String, Value>,
) -> Map<String, Value> {
    let known_fields = current.keys().cloned().collect::<Vec<_>>();
    let unknown_null_fields = patch
        .iter()
        .filter(|(key, value)| value.is_null() && !current.contains_key(*key))
        .map(|(key, _)| key.clone())
        .collect::<Vec<_>>();
    let mut target = Value::Object(current);
    merge_patch_value(&mut target, Value::Object(patch));
    let mut target = match target {
        Value::Object(object) => object,
        Value::Array(_) | Value::Bool(_) | Value::Null | Value::Number(_) | Value::String(_) => {
            unreachable!("root merge patch remains an object")
        }
    };
    for key in known_fields {
        target.entry(key).or_insert(Value::Null);
    }
    for key in unknown_null_fields {
        target.entry(key).or_insert(Value::Null);
    }
    target
}

pub(super) fn materialize_removed_fields(
    current: Map<String, Value>,
    mut patched: Map<String, Value>,
) -> Map<String, Value> {
    for key in current.keys() {
        patched.entry(key.clone()).or_insert(Value::Null);
    }
    patched
}

fn merge_patch_value(target: &mut Value, patch: Value) {
    let Value::Object(patch) = patch else {
        *target = patch;
        return;
    };
    if !target.is_object() {
        *target = Value::Object(Map::new());
    }
    let target = target.as_object_mut().expect("target is an object");
    for (key, value) in patch {
        if value.is_null() {
            target.remove(&key);
        } else {
            merge_patch_value(target.entry(key).or_insert(Value::Null), value);
        }
    }
}
