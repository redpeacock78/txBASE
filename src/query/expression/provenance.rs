use super::{
    ArrayExpression, BooleanExpression, ExpressionReference, NumericExpression, ScalarExpression,
    string::StringExpression,
};
use std::collections::BTreeMap;

pub(in crate::query) fn uses_only_group_key_fields(expression: &ScalarExpression) -> bool {
    scalar_uses_only_group_key_fields(expression, &BTreeMap::new())
}

fn scalar_uses_only_group_key_fields(
    expression: &ScalarExpression,
    variables: &BTreeMap<String, bool>,
) -> bool {
    match expression {
        ScalarExpression::Reference(reference) => {
            reference_uses_only_group_key_fields(reference, variables)
        }
        ScalarExpression::Literal(_) => true,
        ScalarExpression::Array(expressions) | ScalarExpression::Concat(expressions) => expressions
            .iter()
            .all(|expression| scalar_uses_only_group_key_fields(expression, variables)),
        ScalarExpression::Numeric(expression) => {
            numeric_uses_only_group_key_fields(expression, variables)
        }
        ScalarExpression::Cond {
            condition,
            then_expression,
            else_expression,
        } => {
            boolean_uses_only_group_key_fields(condition, variables)
                && scalar_uses_only_group_key_fields(then_expression, variables)
                && scalar_uses_only_group_key_fields(else_expression, variables)
        }
        ScalarExpression::IfNull(first, fallback) => {
            scalar_uses_only_group_key_fields(first, variables)
                && scalar_uses_only_group_key_fields(fallback, variables)
        }
        ScalarExpression::ToLower(expression) | ScalarExpression::ToUpper(expression) => {
            scalar_uses_only_group_key_fields(expression, variables)
        }
        ScalarExpression::String(expression) => {
            string_uses_only_group_key_fields(expression, variables)
        }
        ScalarExpression::ArrayOperation(expression) => {
            array_uses_only_group_key_fields(expression, variables)
        }
        ScalarExpression::Let {
            bindings,
            in_expression,
        } => {
            let mut scoped_variables = variables.clone();
            let bindings_are_valid = bindings.iter().all(|(name, expression)| {
                let valid = scalar_uses_only_group_key_fields(expression, variables);
                scoped_variables.insert(name.clone(), valid);
                valid
            });
            bindings_are_valid
                && scalar_uses_only_group_key_fields(in_expression, &scoped_variables)
        }
    }
}

fn string_uses_only_group_key_fields(
    expression: &StringExpression,
    variables: &BTreeMap<String, bool>,
) -> bool {
    let valid = |expression| scalar_uses_only_group_key_fields(expression, variables);
    match expression {
        StringExpression::Length(input) => valid(input),
        StringExpression::Substring {
            input,
            start,
            count,
        } => valid(input) && valid(start) && valid(count),
        StringExpression::Split { input, delimiter } => valid(input) && valid(delimiter),
        StringExpression::IndexOf {
            input,
            search,
            start,
            end,
        } => {
            valid(input)
                && valid(search)
                && start.as_ref().is_none_or(&valid)
                && end.as_ref().is_none_or(&valid)
        }
        StringExpression::Replace {
            input,
            find,
            replacement,
            ..
        } => valid(input) && valid(find) && valid(replacement),
    }
}

fn array_uses_only_group_key_fields(
    expression: &ArrayExpression,
    variables: &BTreeMap<String, bool>,
) -> bool {
    match expression {
        ArrayExpression::Map {
            input,
            variable,
            array_index_variable,
            in_expression,
        } => {
            let input_is_group_key = scalar_uses_only_group_key_fields(input, variables);
            let mut scoped_variables = variables.clone();
            scoped_variables.insert(variable.clone(), input_is_group_key);
            scoped_variables.insert(
                array_index_variable.as_deref().unwrap_or("IDX").to_owned(),
                input_is_group_key,
            );
            input_is_group_key
                && scalar_uses_only_group_key_fields(in_expression, &scoped_variables)
        }
        ArrayExpression::Filter {
            input,
            variable,
            array_index_variable,
            condition,
            limit,
        } => {
            let input_is_group_key = scalar_uses_only_group_key_fields(input, variables);
            let mut scoped_variables = variables.clone();
            scoped_variables.insert(variable.clone(), input_is_group_key);
            scoped_variables.insert(
                array_index_variable.as_deref().unwrap_or("IDX").to_owned(),
                input_is_group_key,
            );
            input_is_group_key
                && boolean_uses_only_group_key_fields(condition, &scoped_variables)
                && limit
                    .as_ref()
                    .is_none_or(|limit| scalar_uses_only_group_key_fields(limit, variables))
        }
        ArrayExpression::Reduce {
            input,
            initial_value,
            variable,
            value_variable,
            array_index_variable,
            in_expression,
        } => {
            let input_is_group_key = scalar_uses_only_group_key_fields(input, variables);
            let initial_is_group_key = scalar_uses_only_group_key_fields(initial_value, variables);
            let mut scoped_variables = variables.clone();
            scoped_variables.insert(variable.clone(), input_is_group_key);
            scoped_variables.insert(value_variable.clone(), initial_is_group_key);
            scoped_variables.insert(
                array_index_variable.as_deref().unwrap_or("IDX").to_owned(),
                input_is_group_key,
            );
            input_is_group_key
                && initial_is_group_key
                && scalar_uses_only_group_key_fields(in_expression, &scoped_variables)
        }
    }
}

fn boolean_uses_only_group_key_fields(
    expression: &BooleanExpression,
    variables: &BTreeMap<String, bool>,
) -> bool {
    match expression {
        BooleanExpression::And(expressions) | BooleanExpression::Or(expressions) => expressions
            .iter()
            .all(|expression| boolean_uses_only_group_key_fields(expression, variables)),
        BooleanExpression::Not(expression) => {
            boolean_uses_only_group_key_fields(expression, variables)
        }
        BooleanExpression::Compare { left, right, .. } => {
            scalar_uses_only_group_key_fields(left, variables)
                && scalar_uses_only_group_key_fields(right, variables)
        }
    }
}

fn reference_uses_only_group_key_fields(
    reference: &ExpressionReference,
    variables: &BTreeMap<String, bool>,
) -> bool {
    match reference {
        ExpressionReference::Field(field) => is_group_key_field(field),
        ExpressionReference::Variable(variable) => {
            if matches!(variable.name.as_str(), "ROOT" | "CURRENT") {
                false
            } else {
                variables.get(&variable.name).copied().unwrap_or(false)
            }
        }
    }
}

fn numeric_uses_only_group_key_fields(
    expression: &NumericExpression,
    variables: &BTreeMap<String, bool>,
) -> bool {
    match expression {
        NumericExpression::Reference(reference) => {
            reference_uses_only_group_key_fields(reference, variables)
        }
        NumericExpression::Literal(_) => true,
        NumericExpression::Absolute(expression)
        | NumericExpression::Ceiling(expression)
        | NumericExpression::Floor(expression) => {
            numeric_uses_only_group_key_fields(expression, variables)
        }
        NumericExpression::Binary { left, right, .. } => {
            numeric_uses_only_group_key_fields(left, variables)
                && numeric_uses_only_group_key_fields(right, variables)
        }
    }
}

fn is_group_key_field(field: &str) -> bool {
    field == "_id"
        || field
            .strip_prefix("_id.")
            .is_some_and(|path| !path.is_empty())
}
