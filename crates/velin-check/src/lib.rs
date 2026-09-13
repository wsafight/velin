//! Conservative static checks for Velin.
//!
//! Two independent analyses, both designed to never reject a program the
//! runtime would accept:
//!
//! * [`infer`] — expression type inference over a coarse [`Type`] lattice,
//!   reporting only provable mismatches (`"a" + 1`, `not 3`, `get(list, "k")`).
//! * [`definite_assignment`] — a must-dataflow analysis over a compiled
//!   [`velin_bytecode::Program`] that proves every variable read is assigned on
//!   all paths, turning "unassigned on this path" runtime errors into
//!   compile-time diagnostics.
//!
//! Both feed the shared [`velin_syntax::Diagnostic`] type so a host reports
//! them through one channel with a stable exit-code contract.

mod cfg;
mod definite;
mod flow;
mod types;

pub use definite::{UnassignedUse, definite_assignment, definite_assignment_slots};
pub use flow::{
    HostSignature, HostSignatures, TypeCheckKind, TypeCheckSite, check_program_types,
    check_program_types_with_hosts, check_program_types_with_hosts_and_slot_types,
    check_program_types_with_slot_types,
};
pub use types::{Environment, Type, TypeError, infer};

use velin_syntax::{Diagnostic, Expr};

/// Type-checks a single expression, returning structured diagnostics anchored
/// at `(file, line, column)`.
#[must_use]
pub fn check_expression(
    expression: &Expr,
    env: &Environment,
    file: &str,
    line: usize,
    column: usize,
) -> Vec<Diagnostic> {
    let mut errors = Vec::new();
    infer(expression, env, &mut errors);
    errors
        .into_iter()
        .map(|error| {
            let (line, column) = error
                .span
                .map_or((line, column), |span| (span.line, span.column));
            Diagnostic::new(file, line, column, error.message)
        })
        .collect()
}

/// Type-checks an expression used as an `if`/`while` condition.
///
/// In addition to errors inside the expression, a known non-boolean result is
/// rejected. `Unknown` remains permissive because it may be boolean at runtime.
#[must_use]
pub fn check_condition(
    expression: &Expr,
    env: &Environment,
    file: &str,
    line: usize,
    column: usize,
) -> Vec<Diagnostic> {
    let mut errors = Vec::new();
    let inferred = infer(expression, env, &mut errors);
    if !matches!(inferred, Type::Boolean | Type::Unknown) {
        errors.push(TypeError {
            message: format!("condition expects boolean, found {}", inferred.name()),
            span: expression.span().cloned(),
        });
    }
    errors
        .into_iter()
        .map(|error| {
            let (line, column) = error
                .span
                .map_or((line, column), |span| (span.line, span.column));
            Diagnostic::new(file, line, column, error.message)
        })
        .collect()
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
