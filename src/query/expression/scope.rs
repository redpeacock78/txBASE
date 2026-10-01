use super::{ExpressionReference, QueryError};
use crate::query_path::{field_value, field_value_from_value};
use serde_json::{Map, Value};

pub(super) const MAX_ARRAY_EXPRESSION_ITERATIONS: usize = 100_000;

#[derive(Debug, Clone)]
pub(in crate::query) struct VariableReference {
    pub(super) name: String,
    pub(super) path: Option<String>,
}

pub(super) struct EvaluationContext<'a> {
    values: &'a Map<String, Value>,
    variables: Vec<(String, Option<Value>)>,
    array_iterations: usize,
}

impl<'a> EvaluationContext<'a> {
    pub(super) fn new(values: &'a Map<String, Value>) -> Self {
        Self {
            values,
            variables: Vec::new(),
            array_iterations: 0,
        }
    }

    pub(super) fn resolve(
        &self,
        reference: &ExpressionReference,
        path: &str,
    ) -> Result<Option<Value>, QueryError> {
        match reference {
            ExpressionReference::Field(field) => Ok(field_value(self.values, field)),
            ExpressionReference::Variable(variable) => {
                if matches!(variable.name.as_str(), "ROOT" | "CURRENT") {
                    return Ok(match &variable.path {
                        Some(path) => field_value(self.values, path),
                        None => Some(Value::Object(self.values.clone())),
                    });
                }
                let Some((_, value)) = self
                    .variables
                    .iter()
                    .rev()
                    .find(|(name, _)| name == &variable.name)
                else {
                    return Err(QueryError::Invalid(format!(
                        "{path} references unbound expression variable $${}",
                        variable.name
                    )));
                };
                Ok(match (value, &variable.path) {
                    (None, _) => None,
                    (Some(value), None) => Some(value.clone()),
                    (Some(value), Some(path)) => field_value_from_value(value, path),
                })
            }
        }
    }

    pub(super) fn with_variables<T>(
        &mut self,
        variables: Vec<(String, Option<Value>)>,
        evaluate: impl FnOnce(&mut Self) -> Result<T, QueryError>,
    ) -> Result<T, QueryError> {
        let original_len = self.variables.len();
        self.variables.extend(variables);
        let result = evaluate(self);
        self.variables.truncate(original_len);
        result
    }

    pub(super) fn visit_array_item(&mut self, path: &str) -> Result<(), QueryError> {
        if self.array_iterations >= MAX_ARRAY_EXPRESSION_ITERATIONS {
            return Err(QueryError::Invalid(format!(
                "{path} exceeds {MAX_ARRAY_EXPRESSION_ITERATIONS} array-expression iterations"
            )));
        }
        self.array_iterations += 1;
        Ok(())
    }
}

pub(super) fn parse_reference(value: &str, path: &str) -> Result<ExpressionReference, QueryError> {
    let Some(reference) = value.strip_prefix('$') else {
        return Err(QueryError::Invalid(format!(
            "{path} must be a field or variable reference"
        )));
    };
    if let Some(variable) = reference.strip_prefix('$') {
        let (name, field_path) = variable
            .split_once('.')
            .map_or((variable, None), |(name, path)| (name, Some(path)));
        if name.is_empty() || field_path.is_some_and(|path| path.is_empty()) {
            return Err(QueryError::Invalid(format!(
                "{path} has an invalid expression variable reference"
            )));
        }
        if !matches!(name, "ROOT" | "CURRENT" | "IDX") {
            validate_variable_name(name, path)?;
        }
        return Ok(ExpressionReference::Variable(VariableReference {
            name: name.to_owned(),
            path: field_path.map(str::to_owned),
        }));
    }
    if reference.is_empty() {
        return Err(QueryError::Invalid(format!(
            "{path} has an empty field reference"
        )));
    }
    Ok(ExpressionReference::Field(reference.to_owned()))
}

pub(super) fn validate_variable_name(name: &str, path: &str) -> Result<(), QueryError> {
    let mut characters = name.chars();
    let valid_start = characters
        .next()
        .is_some_and(|character| character.is_ascii_lowercase() || !character.is_ascii());
    let valid_rest = characters.all(|character| {
        character.is_ascii_alphanumeric() || character == '_' || !character.is_ascii()
    });
    if !valid_start || !valid_rest {
        let detail = if name
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_uppercase())
        {
            format!("uses unsupported system variable $${name}")
        } else {
            "has an invalid expression variable name".to_owned()
        };
        return Err(QueryError::Invalid(format!("{path} {detail}")));
    }
    Ok(())
}
