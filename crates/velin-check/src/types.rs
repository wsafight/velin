//! Conservative static type inference for Velin expressions.
//!
//! The language is dynamically evaluated, so this checker is a *safety net*,
//! not a gatekeeper: it only reports a diagnostic when an expression is
//! guaranteed to fail at runtime regardless of variable values. Anything it
//! cannot prove wrong is [`Type::Unknown`], which unifies with everything and
//! never produces a false positive.
//!
//! This catches the common authoring mistakes the runtime would otherwise only
//! surface on the unlucky code path: `"a" + 1`, `if <non-boolean>`,
//! `get(list, "key")`, arithmetic on strings, and so on.

use std::collections::BTreeMap;
use velin_syntax::{BinaryOp, Builtin, Expr, Span, UnaryOp, Value};

/// A coarse type lattice. `Unknown` is the top element: it is compatible with
/// every type and is produced whenever inference cannot be certain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Type {
    Integer,
    Boolean,
    String,
    List,
    Record,
    Unknown,
}

impl Type {
    /// Stable human-readable name used in diagnostics and host schemas.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Integer => "integer",
            Self::Boolean => "boolean",
            Self::String => "string",
            Self::List => "list",
            Self::Record => "record",
            Self::Unknown => "unknown",
        }
    }

    /// Whether this type could satisfy `expected` at runtime.
    ///
    /// `Unknown` is compatible with every type, preserving the checker's rule
    /// that uncertain values do not create false positives.
    #[must_use]
    pub fn could_be(self, expected: Type) -> bool {
        self == Type::Unknown || expected == Type::Unknown || self == expected
    }
}

impl From<&Value> for Type {
    fn from(value: &Value) -> Self {
        match value {
            Value::Integer(_) => Self::Integer,
            Value::Boolean(_) => Self::Boolean,
            Value::String(_) => Self::String,
            Value::List(_) => Self::List,
            Value::Record(_) => Self::Record,
        }
    }
}

/// A type error found in an expression, with the message the caller turns into
/// a structured diagnostic. `line` is inherited from the checking context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeError {
    pub message: String,
    pub span: Option<Span>,
}

impl TypeError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            span: None,
        }
    }
}

/// Known variable types, keyed by name. Absent or `Unknown`-typed variables are
/// treated permissively.
pub type Environment = BTreeMap<String, Type>;

/// Infers the type of `expression`, appending any provable type errors to
/// `errors`. Always returns a best-effort type (possibly [`Type::Unknown`]) so
/// checking can continue past an error without cascading false positives.
pub fn infer(expression: &Expr, env: &Environment, errors: &mut Vec<TypeError>) -> Type {
    infer_with(
        expression,
        &|name| env.get(name).copied().unwrap_or(Type::Unknown),
        errors,
    )
}

pub(crate) fn infer_with(
    expression: &Expr,
    variable_type: &impl Fn(&str) -> Type,
    errors: &mut Vec<TypeError>,
) -> Type {
    match expression {
        Expr::Spanned { span, expression } => {
            let first_error = errors.len();
            let inferred = infer_with(expression, variable_type, errors);
            for error in &mut errors[first_error..] {
                if error.span.is_none() {
                    error.span = Some(span.clone());
                }
            }
            inferred
        }
        Expr::Value(value) => Type::from(value),
        Expr::Variable(name) => variable_type(name),
        Expr::Unary { op, value } => {
            let inner = infer_with(value, variable_type, errors);
            match op {
                UnaryOp::Negate => {
                    expect(inner, Type::Integer, "unary `-`", errors);
                    Type::Integer
                }
                UnaryOp::Not => {
                    expect(inner, Type::Boolean, "`not`", errors);
                    Type::Boolean
                }
            }
        }
        Expr::Binary { left, op, right } => {
            let l = infer_with(left, variable_type, errors);
            let r = infer_with(right, variable_type, errors);
            infer_binary(*op, l, r, errors)
        }
        Expr::Invoke {
            function,
            arguments,
        } => {
            let argument_types: Vec<Type> = arguments
                .iter()
                .map(|arg| infer_with(arg, variable_type, errors))
                .collect();
            infer_builtin(*function, &argument_types, errors)
        }
        Expr::Interpolate { parts } => {
            // Every hole is checked recursively; any value renders to a string,
            // so the only thing to report is an error *inside* a hole. The
            // result is always a string.
            for part in parts {
                if let velin_syntax::StrPart::Hole(expr) = part {
                    infer_with(expr, variable_type, errors);
                }
            }
            Type::String
        }
    }
}

fn expect(actual: Type, expected: Type, operation: &str, errors: &mut Vec<TypeError>) {
    if !actual.could_be(expected) {
        errors.push(TypeError::new(format!(
            "{operation} expects {}, found {}",
            expected.name(),
            actual.name()
        )));
    }
}

fn infer_binary(op: BinaryOp, l: Type, r: Type, errors: &mut Vec<TypeError>) -> Type {
    match op {
        BinaryOp::Add => {
            // `+` is integer addition or string concatenation; both operands
            // must share one of those two types.
            if l.could_be(Type::Integer) && r.could_be(Type::Integer) {
                result_of(l, r, Type::Integer)
            } else if l.could_be(Type::String) && r.could_be(Type::String) {
                result_of(l, r, Type::String)
            } else {
                errors.push(TypeError::new(format!(
                    "`+` cannot combine {} and {}",
                    l.name(),
                    r.name()
                )));
                Type::Unknown
            }
        }
        BinaryOp::Subtract | BinaryOp::Multiply | BinaryOp::Divide => {
            expect(l, Type::Integer, "arithmetic", errors);
            expect(r, Type::Integer, "arithmetic", errors);
            Type::Integer
        }
        BinaryOp::Equal | BinaryOp::NotEqual => Type::Boolean,
        BinaryOp::Less | BinaryOp::LessEqual | BinaryOp::Greater | BinaryOp::GreaterEqual => {
            // Comparisons are defined on two integers or two strings.
            let both_int = l.could_be(Type::Integer) && r.could_be(Type::Integer);
            let both_str = l.could_be(Type::String) && r.could_be(Type::String);
            if !(both_int || both_str) {
                errors.push(TypeError::new(format!(
                    "comparison cannot combine {} and {}",
                    l.name(),
                    r.name()
                )));
            }
            Type::Boolean
        }
        BinaryOp::And | BinaryOp::Or => {
            expect(l, Type::Boolean, "boolean operation", errors);
            expect(r, Type::Boolean, "boolean operation", errors);
            Type::Boolean
        }
    }
}

/// Refines the result type of `+` when both operands are known to be the same
/// concrete type; falls back to the nominal type otherwise.
fn result_of(l: Type, r: Type, nominal: Type) -> Type {
    if l == nominal || r == nominal {
        nominal
    } else {
        Type::Unknown
    }
}

#[allow(clippy::match_same_arms)]
fn infer_builtin(function: Builtin, args: &[Type], errors: &mut Vec<TypeError>) -> Type {
    if !function.accepts(args.len()) {
        errors.push(TypeError::new(format!(
            "{} does not accept {} argument(s)",
            builtin_name(function),
            args.len()
        )));
    }
    match function {
        Builtin::List => Type::List,
        Builtin::Record => {
            // Record keys must be strings (every other argument).
            for key in args.iter().step_by(2) {
                expect(*key, Type::String, "record key", errors);
            }
            Type::Record
        }
        Builtin::Len => {
            if let Some(first) = args.first() {
                expect_container_or_string(*first, "len", errors);
            }
            Type::Integer
        }
        Builtin::Contains => {
            if let Some(first) = args.first() {
                expect_container_or_string(*first, "contains", errors);
            }
            Type::Boolean
        }
        Builtin::Get => {
            check_indexable(args, "get", errors);
            Type::Unknown // element type is not tracked
        }
        Builtin::Put => {
            check_indexable(args, "put", errors);
            // Result matches the collection type when known.
            args.first().copied().unwrap_or(Type::Unknown)
        }
        Builtin::Push => {
            if let Some(first) = args.first() {
                expect(*first, Type::List, "push", errors);
            }
            Type::List
        }
        Builtin::Remove => {
            check_indexable(args, "remove", errors);
            args.first().copied().unwrap_or(Type::Unknown)
        }
        Builtin::Random => {
            for argument in args {
                expect(*argument, Type::Integer, "random", errors);
            }
            Type::Integer
        }
        Builtin::Chance => {
            if let Some(percent) = args.first() {
                expect(*percent, Type::Integer, "chance", errors);
            }
            Type::Boolean
        }
    }
}

fn expect_container_or_string(actual: Type, operation: &str, errors: &mut Vec<TypeError>) {
    if !(actual.could_be(Type::List)
        || actual.could_be(Type::Record)
        || actual.could_be(Type::String))
    {
        errors.push(TypeError::new(format!(
            "{operation} expects a list, record or string, found {}",
            actual.name()
        )));
    }
}

/// Checks the `(collection, key, ...)` shape shared by get/put/remove: a list
/// indexed by an integer, or a record indexed by a string.
fn check_indexable(args: &[Type], operation: &str, errors: &mut Vec<TypeError>) {
    let (Some(collection), Some(key)) = (args.first().copied(), args.get(1).copied()) else {
        return;
    };
    let list_ok = collection.could_be(Type::List) && key.could_be(Type::Integer);
    let record_ok = collection.could_be(Type::Record) && key.could_be(Type::String);
    if !(list_ok || record_ok) {
        errors.push(TypeError::new(format!(
            "{operation} expects a list and integer index, or a record and string key, \
             found {} and {}",
            collection.name(),
            key.name()
        )));
    }
}

fn builtin_name(function: Builtin) -> &'static str {
    match function {
        Builtin::List => "list",
        Builtin::Record => "record",
        Builtin::Get => "get",
        Builtin::Put => "put",
        Builtin::Push => "push",
        Builtin::Remove => "remove",
        Builtin::Len => "len",
        Builtin::Contains => "contains",
        Builtin::Random => "random",
        Builtin::Chance => "chance",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use velin_parse::parse_expression;

    fn errors_for(source: &str, env: &Environment) -> Vec<String> {
        let expr = parse_expression(source, "t", 1, 1).unwrap();
        let mut errors = Vec::new();
        infer(&expr, env, &mut errors);
        errors.into_iter().map(|error| error.message).collect()
    }

    #[test]
    fn provable_mismatches_are_reported() {
        let env = Environment::new();
        assert!(!errors_for("\"a\" + 1", &env).is_empty());
        assert!(!errors_for("not 3", &env).is_empty());
        assert!(!errors_for("1 - \"x\"", &env).is_empty());
        assert!(!errors_for("get(list(1), \"key\")", &env).is_empty());
        assert!(!errors_for("push(record(\"a\", 1), 2)", &env).is_empty());
    }

    #[test]
    fn valid_expressions_have_no_errors() {
        let env = Environment::from([("hp".into(), Type::Integer)]);
        assert!(errors_for("hp + 1", &env).is_empty());
        assert!(errors_for("\"a\" + \"b\"", &env).is_empty());
        assert!(errors_for("len(list(1, 2))", &env).is_empty());
        assert!(errors_for("get(list(1, 2), 0)", &env).is_empty());
        assert!(errors_for("contains(\"abc\", \"b\")", &env).is_empty());
        assert!(errors_for("random(1, 6)", &env).is_empty());
        assert!(errors_for("chance(50)", &env).is_empty());
        assert!(!errors_for("random(\"low\", 6)", &env).is_empty());
        assert!(!errors_for("chance(false)", &env).is_empty());
    }

    #[test]
    fn unknown_variables_never_false_positive() {
        let env = Environment::new(); // everything is Unknown
        assert!(errors_for("mystery + 1", &env).is_empty());
        assert!(errors_for("if_flag and other", &env).is_empty());
        assert!(errors_for("get(mystery, key)", &env).is_empty());
    }

    #[test]
    fn every_type_and_builtin_path_is_reported() {
        use std::collections::BTreeMap;
        use std::sync::Arc;
        use velin_syntax::{Expr, Value};

        assert_eq!(Type::Unknown.name(), "unknown");
        assert_eq!(Type::List.name(), "list");
        assert_eq!(Type::Record.name(), "record");
        assert_eq!(Type::from(&Value::List(Arc::new(Vec::new()))), Type::List);
        assert_eq!(
            Type::from(&Value::Record(Arc::new(BTreeMap::new()))),
            Type::Record
        );

        let env = Environment::new();
        assert!(!errors_for("- \"x\"", &env).is_empty());
        assert!(!errors_for("true < 1", &env).is_empty());
        assert!(!errors_for("\"a\" >= 3", &env).is_empty());
        assert!(!errors_for("len(1)", &env).is_empty());
        assert!(!errors_for("contains(1, 2)", &env).is_empty());
        assert!(!errors_for("record(1, 2)", &env).is_empty());
        assert!(!errors_for("put(list(1), \"k\", 2)", &env).is_empty());
        assert!(!errors_for("remove(list(1), \"k\")", &env).is_empty());
        assert!(errors_for("put(list(1), 0, 2)", &env).is_empty());
        assert!(errors_for("remove(record(\"a\", 1), \"a\")", &env).is_empty());
        assert!(errors_for("len(\"ab\")", &env).is_empty());
        assert!(errors_for("len(record(\"a\", 1))", &env).is_empty());
        assert!(errors_for("contains(record(\"a\", 1), \"a\")", &env).is_empty());
        assert!(
            errors_for("\"hello [1 - true]\"", &env)
                .iter()
                .any(|message| message.contains("arithmetic"))
        );
        assert!(errors_for("\"[hp]\"", &env).is_empty());

        let mut errors = Vec::new();
        let too_many_args = Expr::Invoke {
            function: velin_syntax::Builtin::Len,
            arguments: vec![
                Expr::Value(Value::Integer(1)),
                Expr::Value(Value::Integer(2)),
            ],
        };
        infer(&too_many_args, &env, &mut errors);
        assert!(
            errors
                .iter()
                .any(|error| error.message.contains("does not accept"))
        );

        let mut errors = Vec::new();
        let empty_get = Expr::Invoke {
            function: velin_syntax::Builtin::Get,
            arguments: Vec::new(),
        };
        assert_eq!(infer(&empty_get, &env, &mut errors), Type::Unknown);

        let mut errors = Vec::new();
        let empty_put = Expr::Invoke {
            function: velin_syntax::Builtin::Put,
            arguments: Vec::new(),
        };
        assert_eq!(infer(&empty_put, &env, &mut errors), Type::Unknown);

        let mut errors = Vec::new();
        let empty_remove = Expr::Invoke {
            function: velin_syntax::Builtin::Remove,
            arguments: Vec::new(),
        };
        assert_eq!(infer(&empty_remove, &env, &mut errors), Type::Unknown);
    }
}
