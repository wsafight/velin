//! The statement AST produced by the parser and consumed by lowering.
//!
//! Every expression and condition embedded in a statement is a
//! [`velin_syntax::Expr`] parsed by `velin-parse`, so this module adds no new
//! value semantics — only the control-flow and binding scaffolding around
//! expressions. There is no host-domain concept here: a `perform`ed call names
//! a *host command* by string, which lowering interns to an opaque id.

use velin_syntax::Expr;

/// A boolean condition guarding an `if`/`elif`/`while`. Stored with its source
/// line so lowering can anchor runtime/check diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Condition {
    pub expr: Expr,
    pub line: usize,
}

/// One arm of an `if` chain: a guard plus the body it protects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Branch {
    pub condition: Condition,
    pub body: Vec<Stmt>,
}

/// A statement in the surface language.
///
/// The variants map one-to-one onto the lowering implementation:
/// `Label`/`Jump` become program-counter targets and `Op::Jump`; `Default`
/// seeds initial state; `Set` becomes `Op::Set`; `Perform` becomes `Op::Host`
/// with an optional bind destination; `If`/`While` become conditional jumps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stmt {
    /// `label name:` — a named jump target. The statements indented beneath it
    /// are its `body`; labels are positional markers, so the body is lowered
    /// inline (a `jump name` transfers to the first statement of the body).
    Label {
        name: String,
        body: Vec<Stmt>,
        line: usize,
    },
    /// `default name = expr` — a variable seeded before the program runs.
    Default {
        name: String,
        value: Expr,
        line: usize,
    },
    /// `set name = expr` — an assignment.
    Set {
        name: String,
        value: Expr,
        line: usize,
    },
    /// `perform cmd(args...)` or `name = perform cmd(args...)` — a host effect.
    ///
    /// `command` is the host command name (interned to a `host_id` when
    /// lowered). `arguments` are the effect's argument expressions. `bind`, if
    /// present, is the variable the host's resume value is stored into.
    Perform {
        command: String,
        arguments: Vec<Expr>,
        bind: Option<String>,
        line: usize,
    },
    /// `if cond:` / `elif cond:` / `else:` — an ordered chain of guarded
    /// branches with an optional final unguarded `else` body.
    If {
        branches: Vec<Branch>,
        otherwise: Option<Vec<Stmt>>,
    },
    /// `while cond:` — a loop whose body repeats while the condition holds.
    While {
        condition: Condition,
        body: Vec<Stmt>,
    },
    /// `jump label` — an unconditional transfer to a labelled statement.
    Jump { label: String, line: usize },
}
