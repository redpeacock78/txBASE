use serde_json::{Map, Value};

pub(super) const MAX_OPERATIONS: usize = 100;

pub(super) fn apply(
    current: Map<String, Value>,
    document: &Value,
) -> Result<Map<String, Value>, String> {
    let operations = document
        .as_array()
        .ok_or_else(|| "JSON Patch body must be an array".to_owned())?;
    if operations.len() > MAX_OPERATIONS {
        return Err(format!(
            "JSON Patch operation count exceeds {MAX_OPERATIONS}"
        ));
    }

    let mut root = Value::Object(current);
    for (index, operation) in operations.iter().enumerate() {
        apply_operation(&mut root, operation)
            .map_err(|error| format!("JSON Patch operation {index}: {error}"))?;
    }
    match root {
        Value::Object(values) => Ok(values),
        _ => Err("JSON Patch must leave the record root as an object".into()),
    }
}

fn apply_operation(root: &mut Value, operation: &Value) -> Result<(), String> {
    let operation = operation
        .as_object()
        .ok_or_else(|| "operation must be an object".to_owned())?;
    let op = operation
        .get("op")
        .and_then(Value::as_str)
        .ok_or_else(|| "operation requires a string op".to_owned())?;
    let path = operation
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| "operation requires a string path".to_owned())?;
    let path = decode_pointer(path)?;

    match op {
        "add" => add_value(root, &path, required_value(operation, "add")?.clone()),
        "remove" => remove_value(root, &path),
        "replace" => replace_value(root, &path, required_value(operation, "replace")?.clone()),
        "test" => {
            let expected = required_value(operation, "test")?;
            let actual = pointer_value(root, &path)?;
            if actual == expected {
                Ok(())
            } else {
                Err("test operation failed".into())
            }
        }
        "copy" => {
            let from = operation
                .get("from")
                .and_then(Value::as_str)
                .ok_or_else(|| "copy operation requires a string from".to_owned())?;
            let value = pointer_value(root, &decode_pointer(from)?)?.clone();
            add_value(root, &path, value)
        }
        "move" => {
            let from = operation
                .get("from")
                .and_then(Value::as_str)
                .ok_or_else(|| "move operation requires a string from".to_owned())?;
            let from = decode_pointer(from)?;
            if path != from && path.starts_with(&from) {
                return Err("move destination cannot be inside its source".into());
            }
            if path == from {
                return Ok(());
            }
            let value = pointer_value(root, &from)?.clone();
            remove_value(root, &from)?;
            add_value(root, &path, value)
        }
        _ => Err(format!("unsupported operation {op}")),
    }
}

fn required_value<'a>(operation: &'a Map<String, Value>, op: &str) -> Result<&'a Value, String> {
    operation
        .get("value")
        .ok_or_else(|| format!("{op} operation requires a value"))
}

fn decode_pointer(path: &str) -> Result<Vec<String>, String> {
    if path.is_empty() {
        return Ok(Vec::new());
    }
    let Some(path) = path.strip_prefix('/') else {
        return Err("JSON Pointer must be empty or start with '/'".into());
    };
    path.split('/').map(decode_token).collect()
}

fn decode_token(token: &str) -> Result<String, String> {
    let mut decoded = String::with_capacity(token.len());
    let mut characters = token.chars();
    while let Some(character) = characters.next() {
        if character != '~' {
            decoded.push(character);
            continue;
        }
        match characters.next() {
            Some('0') => decoded.push('~'),
            Some('1') => decoded.push('/'),
            Some(other) => return Err(format!("invalid JSON Pointer escape ~{other}")),
            None => return Err("incomplete JSON Pointer escape".into()),
        }
    }
    Ok(decoded)
}

fn pointer_value<'a>(root: &'a Value, path: &[String]) -> Result<&'a Value, String> {
    let mut current = root;
    for token in path {
        current = match current {
            Value::Object(values) => values
                .get(token)
                .ok_or_else(|| format!("path does not exist: {token}"))?,
            Value::Array(values) => values
                .get(array_index(token, values.len(), false)?)
                .ok_or_else(|| format!("array path does not exist: {token}"))?,
            _ => return Err("path traverses a scalar value".into()),
        };
    }
    Ok(current)
}

fn pointer_value_mut<'a>(root: &'a mut Value, path: &[String]) -> Result<&'a mut Value, String> {
    let mut current = root;
    for token in path {
        current = match current {
            Value::Object(values) => values
                .get_mut(token)
                .ok_or_else(|| format!("path does not exist: {token}"))?,
            Value::Array(values) => {
                let index = array_index(token, values.len(), false)?;
                values
                    .get_mut(index)
                    .ok_or_else(|| format!("array path does not exist: {token}"))?
            }
            _ => return Err("path traverses a scalar value".into()),
        };
    }
    Ok(current)
}

fn add_value(root: &mut Value, path: &[String], value: Value) -> Result<(), String> {
    let Some((last, parent_path)) = path.split_last() else {
        *root = value;
        return Ok(());
    };
    let parent = pointer_value_mut(root, parent_path)?;
    match parent {
        Value::Object(values) => {
            values.insert(last.clone(), value);
            Ok(())
        }
        Value::Array(values) => {
            let index = array_index(last, values.len(), true)?;
            values.insert(index, value);
            Ok(())
        }
        _ => Err("add path parent is not an object or array".into()),
    }
}

fn replace_value(root: &mut Value, path: &[String], value: Value) -> Result<(), String> {
    let Some((last, parent_path)) = path.split_last() else {
        *root = value;
        return Ok(());
    };
    let parent = pointer_value_mut(root, parent_path)?;
    match parent {
        Value::Object(values) => {
            if !values.contains_key(last) {
                return Err(format!("replace path does not exist: {last}"));
            }
            values.insert(last.clone(), value);
            Ok(())
        }
        Value::Array(values) => {
            let index = array_index(last, values.len(), false)?;
            values[index] = value;
            Ok(())
        }
        _ => Err("replace path parent is not an object or array".into()),
    }
}

fn remove_value(root: &mut Value, path: &[String]) -> Result<(), String> {
    let Some((last, parent_path)) = path.split_last() else {
        return Err("remove cannot delete the record root".into());
    };
    let parent = pointer_value_mut(root, parent_path)?;
    match parent {
        Value::Object(values) => values
            .remove(last)
            .map(|_| ())
            .ok_or_else(|| format!("remove path does not exist: {last}")),
        Value::Array(values) => {
            let index = array_index(last, values.len(), false)?;
            values.remove(index);
            Ok(())
        }
        _ => Err("remove path parent is not an object or array".into()),
    }
}

fn array_index(token: &str, length: usize, allow_end: bool) -> Result<usize, String> {
    if allow_end && token == "-" {
        return Ok(length);
    }
    if token.is_empty() || (token.len() > 1 && token.starts_with('0')) {
        return Err(format!("invalid array index: {token}"));
    }
    if !token.chars().all(|character| character.is_ascii_digit()) {
        return Err(format!("invalid array index: {token}"));
    }
    let index = token
        .parse::<usize>()
        .map_err(|_| format!("array index overflows usize: {token}"))?;
    let valid = if allow_end {
        index <= length
    } else {
        index < length
    };
    if !valid {
        return Err(format!("array index out of bounds: {token}"));
    }
    Ok(index)
}

#[cfg(test)]
mod tests {
    use super::apply;
    use serde_json::json;

    #[test]
    fn applies_all_json_patch_operations_and_pointer_escapes() {
        let current = json!({
            "NAME": "Alice",
            "NESTED": {"VALUES": [1, 2], "a/b": true}
        });
        let patch = json!([
            {"op": "replace", "path": "/NAME", "value": "Alicia"},
            {"op": "add", "path": "/NESTED/VALUES/-", "value": 3},
            {"op": "copy", "from": "/NESTED/VALUES/0", "path": "/NESTED/COPY"},
            {"op": "move", "from": "/NESTED/COPY", "path": "/NESTED/MOVED"},
            {"op": "test", "path": "/NESTED/a~1b", "value": true},
            {"op": "remove", "path": "/NESTED/VALUES/1"}
        ]);

        assert_eq!(
            apply(current.as_object().unwrap().clone(), &patch).unwrap(),
            json!({
                "NAME": "Alicia",
                "NESTED": {"VALUES": [1, 3], "a/b": true, "MOVED": 1}
            })
            .as_object()
            .unwrap()
            .clone()
        );
    }

    #[test]
    fn rejects_too_many_operations_without_partial_result() {
        let patch = (0..=super::MAX_OPERATIONS)
            .map(|_| json!({"op": "test", "path": "/NAME", "value": "Alice"}))
            .collect::<Vec<_>>();
        let error = apply(
            json!({"NAME": "Alice"}).as_object().unwrap().clone(),
            &json!(patch),
        )
        .unwrap_err();
        assert!(error.contains("operation count"));
    }

    #[test]
    fn rejects_out_of_bounds_array_indices() {
        let current = json!({"VALUES": [1, 2]});
        for (operation, path) in [
            ("add", "/VALUES/3"),
            ("replace", "/VALUES/2"),
            ("remove", "/VALUES/2"),
        ] {
            let patch = json!([{
                "op": operation,
                "path": path,
                "value": 3
            }]);
            let error = apply(current.as_object().unwrap().clone(), &patch).unwrap_err();
            assert!(error.contains("array index out of bounds"));
        }
    }
}
