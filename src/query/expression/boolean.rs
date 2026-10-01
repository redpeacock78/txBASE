use super::{
    EvaluationContext, QueryError, ScalarExpression, compare_values, evaluate_scalar_in_context,
    parse_scalar_operand,
};
use serde_json::Value;

#[derive(Debug, Clone)]
pub(in crate::query) enum BooleanExpression {
    And(Vec<BooleanExpression>),
    Or(Vec<BooleanExpression>),
    Not(Box<BooleanExpression>),
    Compare {
        operator: ComparisonOperator,
        left: ScalarExpression,
        right: ScalarExpression,
    },
}

#[derive(Debug, Clone, Copy)]
pub(in crate::query) enum ComparisonOperator {
    Eq,
    Ne,
    Gt,
    Gte,
    Lt,
    Lte,
}

impl ComparisonOperator {
    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "$eq" => Self::Eq,
            "$ne" => Self::Ne,
            "$gt" => Self::Gt,
            "$gte" => Self::Gte,
            "$lt" => Self::Lt,
            "$lte" => Self::Lte,
            _ => return None,
        })
    }
}

pub(super) fn parse_boolean_expression(
    value: &Value,
    path: &str,
) -> Result<BooleanExpression, QueryError> {
    let expression = value
        .as_object()
        .ok_or_else(|| QueryError::Invalid(format!("{path} must be an object")))?;
    let Some((operator, operands)) = expression.iter().next() else {
        return Err(QueryError::Invalid(format!("{path} cannot be empty")));
    };
    if expression.len() != 1 {
        return Err(QueryError::Invalid(format!(
            "{path} supports one expression operator"
        )));
    }
    match operator.as_str() {
        "$and" | "$or" => {
            let expressions = operands.as_array().ok_or_else(|| {
                QueryError::Invalid(format!("{path}.{operator} must be an array"))
            })?;
            let expressions = expressions
                .iter()
                .enumerate()
                .map(|(index, expression)| {
                    parse_boolean_expression(expression, &format!("{path}.{operator}[{index}]"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            if operator == "$and" {
                Ok(BooleanExpression::And(expressions))
            } else {
                Ok(BooleanExpression::Or(expressions))
            }
        }
        "$not" => Ok(BooleanExpression::Not(Box::new(parse_boolean_expression(
            operands,
            &format!("{path}.$not"),
        )?))),
        _ => {
            let operator = ComparisonOperator::parse(operator).ok_or_else(|| {
                QueryError::Invalid(format!(
                    "{path} supports only boolean and comparison operators"
                ))
            })?;
            let values = operands.as_array().ok_or_else(|| {
                QueryError::Invalid(format!("{path}.{} must be an array", operator.as_str()))
            })?;
            let [left, right] = values.as_slice() else {
                return Err(QueryError::Invalid(format!(
                    "{path}.{} requires two operands",
                    operator.as_str()
                )));
            };
            Ok(BooleanExpression::Compare {
                operator,
                left: parse_scalar_operand(left, &format!("{path}.{}[0]", operator.as_str()))?,
                right: parse_scalar_operand(right, &format!("{path}.{}[1]", operator.as_str()))?,
            })
        }
    }
}

pub(super) fn evaluate_boolean(
    context: &mut EvaluationContext<'_>,
    expression: &BooleanExpression,
    path: &str,
) -> Result<bool, QueryError> {
    match expression {
        BooleanExpression::And(expressions) => {
            for (index, expression) in expressions.iter().enumerate() {
                if !evaluate_boolean(context, expression, &format!("{path}.$and[{index}]"))? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        BooleanExpression::Or(expressions) => {
            for (index, expression) in expressions.iter().enumerate() {
                if evaluate_boolean(context, expression, &format!("{path}.$or[{index}]"))? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        BooleanExpression::Not(expression) => Ok(!evaluate_boolean(
            context,
            expression,
            &format!("{path}.$not"),
        )?),
        BooleanExpression::Compare {
            operator,
            left,
            right,
        } => {
            let (Some(left), Some(right)) = (
                evaluate_scalar_in_context(
                    context,
                    left,
                    &format!("{path}.{}[0]", operator.as_str()),
                )?,
                evaluate_scalar_in_context(
                    context,
                    right,
                    &format!("{path}.{}[1]", operator.as_str()),
                )?,
            ) else {
                return Ok(false);
            };
            Ok(match operator {
                ComparisonOperator::Eq => left == right,
                ComparisonOperator::Ne => left != right,
                ComparisonOperator::Gt => {
                    compare_values(&left, &right).is_some_and(|ordering| ordering.is_gt())
                }
                ComparisonOperator::Gte => {
                    compare_values(&left, &right).is_some_and(|ordering| ordering.is_ge())
                }
                ComparisonOperator::Lt => {
                    compare_values(&left, &right).is_some_and(|ordering| ordering.is_lt())
                }
                ComparisonOperator::Lte => {
                    compare_values(&left, &right).is_some_and(|ordering| ordering.is_le())
                }
            })
        }
    }
}

impl ComparisonOperator {
    fn as_str(self) -> &'static str {
        match self {
            Self::Eq => "$eq",
            Self::Ne => "$ne",
            Self::Gt => "$gt",
            Self::Gte => "$gte",
            Self::Lt => "$lt",
            Self::Lte => "$lte",
        }
    }
}
