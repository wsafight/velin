//! The tree-walking expression evaluator.
//!
//! This is the reference implementation of Velin's value semantics. It
//! evaluates the shared expression AST directly and returns a local
//! [`EvalError`] carrying the source line and message. The bytecode VM is
//! differential-tested against this implementation.

use std::collections::BTreeMap;
use velin_syntax::{BinaryOp, Expr, MAX_DATA_TEXT_BYTES, StrPart, UnaryOp, Value};

/// The variable environment an expression is evaluated against.
///
/// Keys are variable names; values are the deterministic [`Value`]s currently
/// bound. A `BTreeMap` is used so iteration order (and therefore any derived
/// serialization) is stable across runs.
pub type Variables = BTreeMap<String, Value>;

/// An error raised while evaluating an expression.
///
/// It records the 1-based source `line` the failing expression came from and a
/// human-readable `message`. Hosts are free to wrap this in their own richer
/// diagnostic type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvalError {
    pub line: usize,
    pub message: String,
}

impl EvalError {
    #[must_use]
    pub fn new(line: usize, message: impl Into<String>) -> Self {
        Self {
            line,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for EvalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for EvalError {}

/// Evaluates `expression` against `variables`, returning its [`Value`].
///
/// `line` is the 1-based source line used for error reporting only; it does
/// not affect the result.
///
/// # Errors
/// Returns [`EvalError`] for unbound variables, type mismatches, integer
/// overflow, division by zero, and any built-in failure.
pub fn evaluate(expression: &Expr, variables: &Variables, line: usize) -> Result<Value, EvalError> {
    validate_result(
        evaluate_inner(expression, variables, &mut None, line)?,
        line,
    )
}

/// Evaluates `expression` while explicitly threading deterministic RNG state.
///
/// This is the tree-walking reference for expressions containing `random` or
/// `chance`. The caller owns the state, so replaying with the same initial value
/// produces the same values and final state.
///
/// # Errors
/// Returns the same errors as [`evaluate`], plus invalid random ranges or
/// percentages.
pub fn evaluate_with_rng(
    expression: &Expr,
    variables: &Variables,
    state: &mut i64,
    line: usize,
) -> Result<Value, EvalError> {
    validate_result(
        evaluate_inner(expression, variables, &mut Some(state), line)?,
        line,
    )
}

fn evaluate_inner(
    expression: &Expr,
    variables: &Variables,
    rng: &mut Option<&mut i64>,
    line: usize,
) -> Result<Value, EvalError> {
    match expression {
        Expr::Spanned { expression, .. } => evaluate_inner(expression, variables, rng, line),
        Expr::Invoke {
            function,
            arguments,
        } => {
            let mut values = Vec::with_capacity(arguments.len());
            for argument in arguments {
                values.push(evaluate_inner(argument, variables, rng, line)?);
            }
            if matches!(
                function,
                velin_syntax::Builtin::Random | velin_syntax::Builtin::Chance
            ) {
                let state = rng.as_deref_mut().ok_or_else(|| {
                    EvalError::new(line, "random functions require a threaded RNG state")
                })?;
                crate::builtins::invoke_random(*function, &values, state, line)
            } else {
                crate::builtins::invoke(*function, values, line)
            }
        }
        Expr::Value(value) => Ok(value.clone()),
        Expr::Variable(name) => variables
            .get(name)
            .cloned()
            .ok_or_else(|| unassigned(line, name)),
        Expr::Unary { op, value } => {
            let value = evaluate_inner(value, variables, rng, line)?;
            apply_unary(*op, value, line)
        }
        Expr::Binary { left, op, right } => {
            let left = evaluate_inner(left, variables, rng, line)?;
            if *op == BinaryOp::And && left == Value::Boolean(false) {
                return Ok(Value::Boolean(false));
            }
            if *op == BinaryOp::Or && left == Value::Boolean(true) {
                return Ok(Value::Boolean(true));
            }
            let right = evaluate_inner(right, variables, rng, line)?;
            apply_binary(left, *op, right, line)
        }
        Expr::Interpolate { parts } => {
            let mut text = String::new();
            for part in parts {
                match part {
                    StrPart::Literal(literal) => append_text(&mut text, literal, line)?,
                    StrPart::Hole(expr) => {
                        let value = evaluate_inner(expr, variables, rng, line)?;
                        let rendered = value
                            .try_to_display()
                            .map_err(|error| execution(line, error))?;
                        append_text(&mut text, &rendered, line)?;
                    }
                }
            }
            Ok(Value::String(text.into()))
        }
    }
}

/// The error produced when a variable is read on a path where it was never
/// assigned. Shared so the tree-walker and the bytecode VM report it
/// identically.
#[must_use]
pub fn unassigned(line: usize, name: &str) -> EvalError {
    EvalError::new(
        line,
        format!("variable `{name}` has not been assigned on this path"),
    )
}

/// Applies a unary operator to an already-evaluated operand.
///
/// # Errors
/// Returns [`EvalError`] on a type mismatch or integer overflow.
pub fn apply_unary(op: UnaryOp, value: Value, line: usize) -> Result<Value, EvalError> {
    match (op, value) {
        (UnaryOp::Negate, Value::Integer(value)) => value
            .checked_neg()
            .map(Value::Integer)
            .ok_or_else(|| execution(line, "integer overflow")),
        (UnaryOp::Not, Value::Boolean(value)) => Ok(Value::Boolean(!value)),
        (UnaryOp::Negate, value) => Err(type_error(line, "unary `-`", "integer", &value)),
        (UnaryOp::Not, value) => Err(type_error(line, "`not`", "boolean", &value)),
    }
}

/// Applies a binary operator to two already-evaluated operands.
///
/// This performs the non-short-circuiting combine; callers are responsible for
/// short-circuiting `and`/`or` before invoking it (both [`evaluate`] and the
/// bytecode VM do).
///
/// # Errors
/// Returns [`EvalError`] on a type mismatch, integer overflow, or division by
/// zero.
pub fn apply_binary(
    left: Value,
    op: BinaryOp,
    right: Value,
    line: usize,
) -> Result<Value, EvalError> {
    match op {
        BinaryOp::Add => match (left, right) {
            (Value::Integer(left), Value::Integer(right)) => left
                .checked_add(right)
                .map(Value::Integer)
                .ok_or_else(|| execution(line, "integer overflow")),
            (Value::String(mut left), Value::String(right)) => {
                let length = left
                    .len()
                    .checked_add(right.len())
                    .ok_or_else(|| execution(line, "data text exceeds 1 MiB"))?;
                if length > MAX_DATA_TEXT_BYTES {
                    return Err(execution(line, "data text exceeds 1 MiB"));
                }
                left.make_mut().push_str(&right);
                Ok(Value::String(left))
            }
            (left, right) => Err(binary_type_error(line, "`+`", &left, &right)),
        },
        BinaryOp::Subtract | BinaryOp::Multiply | BinaryOp::Divide => {
            let (Value::Integer(left), Value::Integer(right)) = (&left, &right) else {
                return Err(binary_type_error(line, "arithmetic", &left, &right));
            };
            let result = match op {
                BinaryOp::Subtract => left.checked_sub(*right),
                BinaryOp::Multiply => left.checked_mul(*right),
                BinaryOp::Divide if *right == 0 => return Err(execution(line, "division by zero")),
                BinaryOp::Divide => left.checked_div(*right),
                _ => unreachable!(),
            };
            result
                .map(Value::Integer)
                .ok_or_else(|| execution(line, "integer overflow"))
        }
        BinaryOp::Equal => Ok(Value::Boolean(left == right)),
        BinaryOp::NotEqual => Ok(Value::Boolean(left != right)),
        BinaryOp::Less | BinaryOp::LessEqual | BinaryOp::Greater | BinaryOp::GreaterEqual => {
            let ordering = match (&left, &right) {
                (Value::Integer(left), Value::Integer(right)) => left.cmp(right),
                (Value::String(left), Value::String(right)) => left.cmp(right),
                _ => return Err(binary_type_error(line, "comparison", &left, &right)),
            };
            let result = match op {
                BinaryOp::Less => ordering.is_lt(),
                BinaryOp::LessEqual => ordering.is_le(),
                BinaryOp::Greater => ordering.is_gt(),
                BinaryOp::GreaterEqual => ordering.is_ge(),
                _ => unreachable!(),
            };
            Ok(Value::Boolean(result))
        }
        BinaryOp::And | BinaryOp::Or => match (left, right) {
            (Value::Boolean(left), Value::Boolean(right)) => {
                Ok(Value::Boolean(if op == BinaryOp::And {
                    left && right
                } else {
                    left || right
                }))
            }
            (left, right) => Err(binary_type_error(line, "boolean operation", &left, &right)),
        },
    }
}

pub(crate) fn execution(line: usize, message: impl Into<String>) -> EvalError {
    EvalError::new(line, message)
}

fn validate_result(value: Value, line: usize) -> Result<Value, EvalError> {
    value
        .validate_data()
        .map_err(|error| execution(line, error))?;
    Ok(value)
}

fn append_text(output: &mut String, text: &str, line: usize) -> Result<(), EvalError> {
    let length = output
        .len()
        .checked_add(text.len())
        .ok_or_else(|| execution(line, "data text exceeds 1 MiB"))?;
    if length > MAX_DATA_TEXT_BYTES {
        return Err(execution(line, "data text exceeds 1 MiB"));
    }
    output.push_str(text);
    Ok(())
}

fn type_error(line: usize, operation: &str, expected: &str, found: &Value) -> EvalError {
    execution(
        line,
        format!(
            "{operation} expects {expected}, found {}",
            found.type_name()
        ),
    )
}

fn binary_type_error(line: usize, operation: &str, left: &Value, right: &Value) -> EvalError {
    execution(
        line,
        format!(
            "{operation} cannot combine {} and {}",
            left.type_name(),
            right.type_name()
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval(expression: &Expr) -> Result<Value, EvalError> {
        evaluate(expression, &Variables::new(), 1)
    }

    fn binary(left: Value, op: BinaryOp, right: Value) -> Expr {
        Expr::Binary {
            left: Box::new(Expr::Value(left)),
            op,
            right: Box::new(Expr::Value(right)),
        }
    }

    #[test]
    fn arithmetic_comparisons_and_string_concat_evaluate() {
        assert_eq!(
            eval(&binary(Value::Integer(1), BinaryOp::Add, Value::Integer(2))).unwrap(),
            Value::Integer(3)
        );
        assert_eq!(
            eval(&binary(
                Value::String("a".into()),
                BinaryOp::Add,
                Value::String("b".into())
            ))
            .unwrap(),
            Value::String("ab".into())
        );
        assert_eq!(
            eval(&binary(
                Value::Integer(3),
                BinaryOp::GreaterEqual,
                Value::Integer(3)
            ))
            .unwrap(),
            Value::Boolean(true)
        );
        assert!(
            eval(&binary(
                Value::Integer(1),
                BinaryOp::Divide,
                Value::Integer(0)
            ))
            .unwrap_err()
            .to_string()
            .contains("division by zero")
        );
        assert!(
            eval(&binary(
                Value::Integer(i64::MAX),
                BinaryOp::Add,
                Value::Integer(1)
            ))
            .unwrap_err()
            .to_string()
            .contains("overflow")
        );
    }

    #[test]
    fn boolean_ops_short_circuit_missing_variables() {
        let and = Expr::Binary {
            left: Box::new(Expr::Value(Value::Boolean(false))),
            op: BinaryOp::And,
            right: Box::new(Expr::Variable("missing".into())),
        };
        assert_eq!(eval(&and).unwrap(), Value::Boolean(false));
        let or = Expr::Binary {
            left: Box::new(Expr::Value(Value::Boolean(true))),
            op: BinaryOp::Or,
            right: Box::new(Expr::Variable("missing".into())),
        };
        assert_eq!(eval(&or).unwrap(), Value::Boolean(true));
        assert!(
            eval(&Expr::Unary {
                op: UnaryOp::Not,
                value: Box::new(Expr::Value(Value::Integer(1))),
            })
            .unwrap_err()
            .to_string()
            .contains("boolean")
        );
    }

    #[test]
    fn unbound_variable_reports_its_name() {
        let error = eval(&Expr::Variable("hp".into())).unwrap_err();
        assert!(error.message.contains("`hp`"));
    }

    #[test]
    fn interpolation_renders_holes_deterministically() {
        // "hp is [hp]!" with hp = 7  ->  "hp is 7!"
        let expr = Expr::Interpolate {
            parts: vec![
                StrPart::Literal("hp is ".into()),
                StrPart::Hole(Box::new(Expr::Variable("hp".into()))),
                StrPart::Literal("!".into()),
            ],
        };
        let vars = Variables::from([("hp".into(), Value::Integer(7))]);
        assert_eq!(
            evaluate(&expr, &vars, 1).unwrap(),
            Value::String("hp is 7!".into())
        );
    }

    #[test]
    fn interpolation_propagates_hole_errors() {
        let expr = Expr::Interpolate {
            parts: vec![StrPart::Hole(Box::new(Expr::Variable("missing".into())))],
        };
        assert!(eval(&expr).unwrap_err().message.contains("`missing`"));
    }

    #[test]
    fn string_growth_is_rejected_before_concatenation() {
        let half = "x".repeat(MAX_DATA_TEXT_BYTES / 2 + 1);
        let error = apply_binary(
            Value::String(half.clone().into()),
            BinaryOp::Add,
            Value::String(half.into()),
            1,
        )
        .unwrap_err();
        assert!(error.message.contains("exceeds 1 MiB"));
    }

    #[test]
    fn unary_binary_and_rng_error_paths() {
        assert!(apply_unary(UnaryOp::Negate, Value::Boolean(true), 1).is_err());
        assert!(apply_unary(UnaryOp::Negate, Value::Integer(i64::MIN), 1).is_err());
        assert_eq!(
            apply_unary(UnaryOp::Not, Value::Boolean(true), 1).unwrap(),
            Value::Boolean(false)
        );
        assert!(
            apply_binary(
                Value::Integer(1),
                BinaryOp::Subtract,
                Value::Boolean(true),
                1
            )
            .is_err()
        );
        assert!(
            apply_binary(
                Value::Integer(i64::MIN),
                BinaryOp::Divide,
                Value::Integer(-1),
                1
            )
            .is_err()
        );
        assert!(
            apply_binary(
                Value::Integer(i64::MAX),
                BinaryOp::Multiply,
                Value::Integer(3),
                1
            )
            .is_err()
        );
        assert_eq!(
            apply_binary(Value::Integer(1), BinaryOp::NotEqual, Value::Integer(2), 1).unwrap(),
            Value::Boolean(true)
        );
        assert_eq!(
            apply_binary(
                Value::String("a".into()),
                BinaryOp::Less,
                Value::String("b".into()),
                1
            )
            .unwrap(),
            Value::Boolean(true)
        );
        assert_eq!(
            apply_binary(Value::Integer(3), BinaryOp::Greater, Value::Integer(1), 1).unwrap(),
            Value::Boolean(true)
        );
        assert!(apply_binary(Value::Integer(1), BinaryOp::Less, Value::Boolean(true), 1).is_err());
        assert_eq!(
            apply_binary(Value::Boolean(true), BinaryOp::And, Value::Boolean(true), 1).unwrap(),
            Value::Boolean(true)
        );
        assert_eq!(
            apply_binary(Value::Boolean(false), BinaryOp::Or, Value::Boolean(true), 1).unwrap(),
            Value::Boolean(true)
        );
        assert!(apply_binary(Value::Boolean(true), BinaryOp::And, Value::Integer(1), 1).is_err());

        let random = velin_parse::parse_expression("random(1, 2)", "t", 1, 1).unwrap();
        assert!(
            evaluate(&random, &Variables::new(), 1)
                .unwrap_err()
                .message
                .contains("RNG")
        );
        let mut state = 0;
        assert!(evaluate_with_rng(&random, &Variables::new(), &mut state, 1).is_ok());

        let oversized = Expr::Interpolate {
            parts: vec![
                StrPart::Literal("x".repeat(MAX_DATA_TEXT_BYTES / 2 + 1)),
                StrPart::Literal("y".repeat(MAX_DATA_TEXT_BYTES / 2 + 1)),
            ],
        };
        assert!(
            eval(&oversized)
                .unwrap_err()
                .message
                .contains("exceeds 1 MiB")
        );
    }
}
