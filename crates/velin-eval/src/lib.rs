//! Deterministic evaluation of Velin expressions.
//!
//! This crate has no host, I/O, or foreign-code capability. It evaluates an
//! [`Expr`] against a variable environment and returns a [`Value`] or an
//! [`EvalError`]. Variable storage is a plain map, keeping the evaluator
//! independent of any host runtime.

mod builtins;
mod eval;

pub use builtins::{
    invoke, invoke_measured, invoke_random, invoke_readonly_measured, invoke_stack_measured,
    invoke_stack_measured_with_metrics,
};
pub use eval::{
    EvalError, Variables, apply_binary, apply_boolean_not, apply_integer_binary,
    apply_integer_unary, apply_unary, evaluate, evaluate_with_rng, unassigned,
};
