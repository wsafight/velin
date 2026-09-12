//! Lowering: statement AST → executable [`CompiledScript`].
//!
//! This is the bridge between the surface syntax and the existing bytecode
//! pipeline. It walks the flat [`Stmt`] list and drives a
//! [`velin_compile::ProgramBuilder`], so every value expression is compiled by
//! the *same* code paths the hand-built embedder API uses — lowering adds only
//! control flow, host interning, and label resolution.
//!
//! Control flow is emitted with placeholder jump targets that are back-patched
//! once the block that follows is known. Labels are resolved in a final pass so
//! a `jump` may target a label defined later in the file; a `jump` to a label
//! that never appears is a [`LowerError`].
//!
//! `default` values are evaluated at compile time (against no variables), so a
//! script's initial state is a plain [`velin_syntax::Value`] the embedder seeds
//! before running.

use crate::ast::Stmt;
use crate::host::{CompiledScript, HostCheckSite, HostTable};
use crate::limits::MAX_STATEMENT_DEPTH;
use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::sync::Arc;
use velin_check::{TypeCheckKind, TypeCheckSite};
use velin_compile::{
    InitialFrame, InitialValue, Op, Pc, ProgramBuilder, UpdateOp, ValidatedProgram,
};
use velin_eval::{Variables, evaluate};
use velin_syntax::{Diagnostic, Expr, Value};

/// A lowering failure with a source line.
///
/// Distinct from a parse error: the statements were syntactically valid, but
/// something only knowable while assembling the program went wrong — a `jump`
/// to an undefined label, a duplicate `label`, or a `default` whose value is
/// not a compile-time constant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LowerError {
    pub line: usize,
    pub message: String,
}

impl LowerError {
    fn new(line: usize, message: impl Into<String>) -> Self {
        Self {
            line,
            message: message.into(),
        }
    }

    /// Renders this error as a structured [`Diagnostic`] attributed to `file`.
    #[must_use]
    pub fn into_diagnostic(self, file: &str) -> Diagnostic {
        Diagnostic::new(file, self.line, 1, self.message)
    }
}

/// Lowers a statement list into a runnable [`CompiledScript`].
///
/// # Errors
/// Returns a [`LowerError`] for an undefined or duplicate label, or a `default`
/// value that is not a compile-time constant.
pub fn lower(statements: Vec<Stmt>) -> Result<CompiledScript, LowerError> {
    let mut lowerer = Lowerer::default();
    lowerer.block(statements, 0)?;
    lowerer.finish()
}

/// Mutable state threaded through the recursive lowering walk.
#[derive(Default)]
struct Lowerer {
    builder: ProgramBuilder,
    hosts: HostTable,
    defaults: BTreeMap<String, Value>,
    initial_values: Vec<InitialValue>,
    type_sites: Vec<TypeCheckSite>,
    host_sites: Vec<HostCheckSite>,
    labels: BTreeMap<String, Pc>,
    /// `Op::Jump`s emitted before their target label's `Pc` was known, to
    /// back-patch once every label has been placed.
    pending: Vec<PendingJump>,
}

/// A forward (or backward) `jump` awaiting label resolution.
struct PendingJump {
    pc: Pc,
    label: String,
    line: usize,
}

impl Lowerer {
    /// Lowers a run of statements in order.
    fn block(&mut self, statements: Vec<Stmt>, depth: usize) -> Result<(), LowerError> {
        if depth > MAX_STATEMENT_DEPTH {
            return Err(LowerError::new(
                statement_line(statements.first()),
                format!("statement nesting exceeds {MAX_STATEMENT_DEPTH}"),
            ));
        }
        for statement in statements {
            self.statement(statement, depth)?;
        }
        Ok(())
    }

    fn statement(&mut self, statement: Stmt, depth: usize) -> Result<(), LowerError> {
        match statement {
            Stmt::Label { name, body, line } => {
                self.lower_label(name, line)?;
                self.block(body, depth + 1)
            }
            Stmt::Default { name, value, line } => {
                if depth != 0 {
                    return Err(LowerError::new(
                        line,
                        "`default` declarations are only allowed at the top level",
                    ));
                }
                self.lower_default(name, value, line)
            }
            Stmt::Set { name, value, line } => {
                self.lower_set(&name, value, line);
                Ok(())
            }
            Stmt::Perform {
                command,
                arguments,
                bind,
                line,
            } => {
                self.lower_perform(&command, arguments, bind.as_deref(), line);
                Ok(())
            }
            Stmt::If {
                branches,
                otherwise,
            } => self.lower_if(branches, otherwise, depth),
            Stmt::While { condition, body } => self.lower_while(condition, body, depth),
            Stmt::Jump { label, line } => {
                self.lower_jump(label, line);
                Ok(())
            }
        }
    }

    /// Records a label's target `Pc` (the next op to be emitted).
    fn lower_label(&mut self, name: String, line: usize) -> Result<(), LowerError> {
        if self.labels.contains_key(&name) {
            return Err(LowerError::new(line, format!("duplicate label `{name}`")));
        }
        self.labels.insert(name, self.builder.here());
        Ok(())
    }

    /// Evaluates a `default` value at compile time and records it as seed state.
    fn lower_default(&mut self, name: String, value: Expr, line: usize) -> Result<(), LowerError> {
        let entry = match self.defaults.entry(name) {
            Entry::Vacant(entry) => entry,
            Entry::Occupied(entry) => {
                return Err(LowerError::new(
                    line,
                    format!("duplicate default `{}`", entry.key()),
                ));
            }
        };
        let value = value.into_unspanned();
        let seed = match value {
            Expr::Value(value) => value,
            expression => evaluate(&expression, &Variables::new(), line).map_err(|error| {
                LowerError::new(
                    line,
                    format!(
                        "`default {}` must be a compile-time constant: {}",
                        entry.key(),
                        error.message
                    ),
                )
            })?,
        };
        // Reserve the slot so the embedder can seed it even if it is only read.
        let slot = self.builder.slot(entry.key());
        let initial = InitialValue::new(entry.key(), slot, seed.clone())
            .map_err(|error| LowerError::new(line, error.to_string()))?;
        entry.insert(seed);
        self.initial_values.push(initial);
        Ok(())
    }

    fn lower_set(&mut self, name: &str, value: Expr, line: usize) {
        let slot = self.builder.slot(name);
        let operation = self.lower_update(name, &value, line).map_or_else(
            || self.builder.set_op(slot, &value, line),
            |(operation, column)| Op::update(slot, operation, line, column),
        );
        let pc = self.builder.push(operation);
        self.type_sites.push(TypeCheckSite {
            pc: pc as usize,
            expression: value,
            kind: TypeCheckKind::Assignment,
        });
    }

    fn lower_update(
        &mut self,
        name: &str,
        expression: &Expr,
        line: usize,
    ) -> Option<(UpdateOp, usize)> {
        match expression.unspanned() {
            Expr::Binary {
                left,
                op: velin_syntax::BinaryOp::Add,
                right,
            } if matches!(right.unspanned(), Expr::Value(Value::Integer(_))) => {
                let column = target_column(left, name)?;
                let Expr::Value(Value::Integer(value)) = right.unspanned() else {
                    unreachable!("guard matched an integer literal")
                };
                Some((UpdateOp::AddInteger { value: *value }, column))
            }
            Expr::Binary {
                left,
                op: velin_syntax::BinaryOp::Add,
                right,
            } if definitely_string(right) => {
                let column = target_column(left, name)?;
                let rhs = self.builder.expr(right, line);
                Some((UpdateOp::Add { rhs }, column))
            }
            Expr::Invoke {
                function,
                arguments,
            } => {
                let source = arguments.first()?;
                let column = target_column(source, name)?;
                match (function, arguments.as_slice()) {
                    (velin_syntax::Builtin::Push, [_, value]) => {
                        let value = self.builder.expr(value, line);
                        Some((UpdateOp::Push { value }, column))
                    }
                    (velin_syntax::Builtin::Put, [_, key, value]) => {
                        let key = self.builder.expr(key, line);
                        let value = self.builder.expr(value, line);
                        Some((UpdateOp::Put { key, value }, column))
                    }
                    (velin_syntax::Builtin::Remove, [_, key]) => {
                        let key = self.builder.expr(key, line);
                        Some((UpdateOp::Remove { key }, column))
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    fn lower_perform(
        &mut self,
        command: &str,
        arguments: Vec<Expr>,
        bind: Option<&str>,
        line: usize,
    ) {
        let host_id = self.hosts.intern(command);
        let args: Vec<_> = arguments
            .iter()
            .map(|argument| self.builder.expr(argument, line))
            .collect();
        let bind = bind.map(|name| self.builder.slot(name));
        let pc = self.builder.push(Op::host(host_id, args, bind, line));
        self.host_sites.push(HostCheckSite {
            host_id,
            arguments: arguments.len(),
            bind: bind.is_some(),
            line,
        });
        self.type_sites
            .extend(arguments.into_iter().map(|expression| TypeCheckSite {
                pc: pc as usize,
                expression,
                kind: TypeCheckKind::Expression,
            }));
    }

    /// Lowers an `if`/`elif`/`else` chain to conditional jumps.
    fn lower_if(
        &mut self,
        branches: Vec<crate::ast::Branch>,
        otherwise: Option<Vec<Stmt>>,
        depth: usize,
    ) -> Result<(), LowerError> {
        // Each branch's guard jumps past its body to the next arm; each body
        // ends with a jump to the shared exit, back-patched at the end.
        let mut exits = Vec::new();
        for branch in branches {
            let condition = branch.condition;
            let guard = self
                .builder
                .jump_if_false_op(&condition.expr, condition.line, Pc::MAX);
            let skip = self.builder.push(guard);
            self.type_sites.push(TypeCheckSite {
                pc: skip as usize,
                expression: condition.expr,
                kind: TypeCheckKind::Condition,
            });
            self.block(branch.body, depth + 1)?;
            exits.push(self.builder.push(Op::Jump(Pc::MAX)));
            let next = self.builder.here();
            self.builder.patch_condition_target(skip, next);
        }
        if let Some(body) = otherwise {
            self.block(body, depth + 1)?;
        }
        let end = self.builder.here();
        for exit in exits {
            self.builder.patch(exit, Op::Jump(end));
        }
        Ok(())
    }

    /// Lowers a `while` loop: guard at the top, back-edge at the bottom.
    fn lower_while(
        &mut self,
        condition: crate::ast::Condition,
        body: Vec<Stmt>,
        depth: usize,
    ) -> Result<(), LowerError> {
        let start = self.builder.here();
        let guard = self
            .builder
            .jump_if_false_op(&condition.expr, condition.line, Pc::MAX);
        let exit = self.builder.push(guard);
        self.type_sites.push(TypeCheckSite {
            pc: exit as usize,
            expression: condition.expr,
            kind: TypeCheckKind::Condition,
        });
        self.block(body, depth + 1)?;
        self.builder.push(Op::Jump(start));
        let after = self.builder.here();
        self.builder.patch_condition_target(exit, after);
        Ok(())
    }

    /// Emits a placeholder jump, deferring label resolution to [`Self::finish`].
    fn lower_jump(&mut self, label: String, line: usize) {
        let pc = self.builder.push(Op::Jump(Pc::MAX));
        self.pending.push(PendingJump { pc, label, line });
    }

    /// Resolves pending jumps and assembles the [`CompiledScript`].
    fn finish(mut self) -> Result<CompiledScript, LowerError> {
        for jump in &self.pending {
            let target = self.labels.get(&jump.label).copied().ok_or_else(|| {
                LowerError::new(
                    jump.line,
                    format!("jump to undefined label `{}`", jump.label),
                )
            })?;
            self.builder.patch(jump.pc, Op::Jump(target));
        }
        let program = Arc::new(self.builder.build());
        let validated_program = ValidatedProgram::new(program.clone()).map_err(|error| {
            LowerError::new(1, format!("compiled program failed validation: {error}"))
        })?;
        let initial_frame = InitialFrame::from_initial_values(&program.slots, self.initial_values)
            .map_err(|error| LowerError::new(1, error.to_string()))?;
        let initial_types = initial_frame
            .values()
            .iter()
            .map(|value| {
                value
                    .as_ref()
                    .map_or(velin_check::Type::Unknown, Into::into)
            })
            .collect();
        Ok(CompiledScript {
            program,
            validated_program,
            hosts: self.hosts.into_names(),
            labels: self.labels,
            defaults: self.defaults,
            initial_frame,
            initial_types,
            type_sites: self.type_sites,
            host_sites: self.host_sites,
        })
    }
}

fn target_column(expression: &Expr, target: &str) -> Option<usize> {
    let column = expression.span().map_or(1, |span| span.column);
    match expression.unspanned() {
        Expr::Variable(name) if name == target => Some(column),
        _ => None,
    }
}

fn definitely_string(expression: &Expr) -> bool {
    match expression.unspanned() {
        Expr::Value(Value::String(_)) | Expr::Interpolate { .. } => true,
        Expr::Binary {
            left,
            op: velin_syntax::BinaryOp::Add,
            right,
        } => definitely_string(left) && definitely_string(right),
        _ => false,
    }
}

fn statement_line(statement: Option<&Stmt>) -> usize {
    match statement {
        Some(
            Stmt::Label { line, .. }
            | Stmt::Default { line, .. }
            | Stmt::Set { line, .. }
            | Stmt::Perform { line, .. }
            | Stmt::Jump { line, .. },
        ) => *line,
        Some(Stmt::If { branches, .. }) => {
            branches.first().map_or(0, |branch| branch.condition.line)
        }
        Some(Stmt::While { condition, .. }) => condition.line,
        None => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    fn compile(source: &str) -> CompiledScript {
        lower(parse(source).expect("parse")).expect("lower")
    }

    #[test]
    fn default_is_evaluated_at_compile_time() {
        let script = compile("default hp = 20 + 10\n");
        assert_eq!(script.defaults.get("hp"), Some(&Value::Integer(30)));
    }

    #[test]
    fn changing_public_defaults_invalidates_cached_check_types() {
        let mut script = compile("default hp = 3\nset hp = hp + 1\n");
        assert!(script.check("test.velin").is_empty());

        script
            .defaults
            .insert("hp".into(), Value::String("three".into()));
        assert!(script.initial_frame().is_none());
        assert!(
            script
                .check("test.velin")
                .iter()
                .any(|diagnostic| diagnostic.message.contains("`+` cannot combine"))
        );
    }

    #[test]
    fn non_constant_default_is_an_error() {
        let error = lower(parse("default hp = other + 1\n").unwrap()).unwrap_err();
        assert!(error.message.contains("compile-time constant"));
    }

    #[test]
    fn defaults_must_be_unique_and_top_level() {
        let duplicate = lower(parse("default hp = 1\ndefault hp = 2\n").unwrap()).unwrap_err();
        assert!(duplicate.message.contains("duplicate default"));

        let nested = lower(parse("if true:\n    default hp = 1\n").unwrap()).unwrap_err();
        assert!(nested.message.contains("top level"));
    }

    #[test]
    fn perform_interns_host_commands_in_first_seen_order() {
        let script = compile("perform show(\"a\")\nperform say(\"b\")\nperform show(\"c\")\n");
        assert_eq!(script.hosts, vec!["show".to_string(), "say".to_string()]);
    }

    #[test]
    fn bound_perform_stores_its_destination_on_the_host_op() {
        let script = compile("choice = perform ask(\"go?\")\n");
        assert!(matches!(
            script.program.ops[0],
            Op::Host(ref host) if host.host_id == 0 && host.bind.is_some()
        ));
    }

    #[test]
    fn ownership_updates_are_emitted_only_for_the_assignment_target() {
        let script = compile(
            "default items = list()\n\
             set items = push(items, 1)\n\
             set items = put(items, 0, 2)\n\
             set items = remove(items, 0)\n\
             default text = \"a\"\n\
             set text = text + \"b\"\n\
             set count = count + 1\n\
             set other = push(items, 3)\n",
        );
        assert!(matches!(
            script.program.ops[0],
            Op::Update {
                operation: UpdateOp::Push { .. },
                ..
            }
        ));
        assert!(matches!(
            script.program.ops[1],
            Op::Update {
                operation: UpdateOp::Put { .. },
                ..
            }
        ));
        assert!(matches!(
            script.program.ops[2],
            Op::Update {
                operation: UpdateOp::Remove { .. },
                ..
            }
        ));
        assert!(matches!(
            script.program.ops[3],
            Op::Update {
                operation: UpdateOp::Add { .. },
                ..
            }
        ));
        assert!(matches!(
            script.program.ops[4],
            Op::Update {
                operation: UpdateOp::AddInteger { value: 1 },
                ..
            }
        ));
        assert!(matches!(script.program.ops[5], Op::Set { .. }));
    }

    #[test]
    fn labels_resolve_forward_and_backward_jumps() {
        let script = compile("jump ahead\nlabel ahead:\njump ahead\n");
        // Both jumps target the same recorded label Pc.
        let target = script.labels["ahead"];
        assert!(matches!(script.program.ops[0], Op::Jump(t) if t == target));
    }

    #[test]
    fn jump_to_undefined_label_is_an_error() {
        let error = lower(parse("jump nowhere\n").unwrap()).unwrap_err();
        assert!(error.message.contains("undefined label `nowhere`"));
    }

    #[test]
    fn duplicate_label_is_an_error() {
        let error = lower(parse("label a:\nlabel a:\n").unwrap()).unwrap_err();
        assert!(error.message.contains("duplicate label `a`"));
    }

    #[test]
    fn if_else_lowers_to_conditional_jumps_and_a_shared_exit() {
        let src = "if hp > 0:\n\
                   \x20\x20\x20\x20set alive = true\n\
                   else:\n\
                   \x20\x20\x20\x20set alive = false\n";
        let script = compile(src);
        assert!(matches!(
            script.program.ops[0],
            Op::JumpIfIntegerCompare { .. }
        ));
        assert!(matches!(script.program.ops[1], Op::SetConst { .. }));
    }
}
