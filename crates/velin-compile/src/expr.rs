//! Lowering from the `Expr` tree to a flat [`ExprChunk`].
//!
//! The compiler is a straightforward post-order walk: operands are emitted
//! before the op that consumes them, which is exactly the order a stack machine
//! needs. The only subtlety is `and`/`or`, which must short-circuit: they are
//! lowered to conditional jumps rather than a `Binary` op, so the right operand
//! is skipped (and never fails on an unassigned variable) when the result is
//! already determined by the left operand.

use crate::bytecode::{ExprChunk, ExprOp};
use crate::slots::SlotTable;
use velin_eval::{apply_binary, apply_unary, invoke};
use velin_syntax::{BinaryOp, Builtin, Expr, MAX_DATA_TEXT_BYTES, StrPart, Value};

/// Compiles `expression` into an [`ExprChunk`], interning any referenced
/// variables into `slots`.
///
/// `line` is recorded on the chunk for runtime error reporting.
#[must_use]
pub fn compile_expression(expression: &Expr, slots: &mut SlotTable, line: usize) -> ExprChunk {
    let mut chunk = ExprChunk::new(line);
    compile_expression_into(expression, slots, line, &mut chunk);
    chunk.compact();
    chunk
}

#[inline]
pub(crate) fn compile_expression_into(
    expression: &Expr,
    slots: &mut SlotTable,
    line: usize,
    chunk: &mut ExprChunk,
) {
    chunk.ops.clear();
    chunk.constants.clear();
    chunk.line = crate::compact_source_position(line);
    let plan = fold_plan(expression, line);
    lower(expression, chunk, slots, 1, &mut FoldCursor::new(&plan));
}

#[derive(Clone, Copy)]
struct FoldShape {
    nodes: usize,
    constant: bool,
    candidate: bool,
}

fn fold_shape(expression: &Expr) -> FoldShape {
    match expression {
        Expr::Spanned { expression, .. } => fold_shape(expression),
        Expr::Value(_) => FoldShape {
            nodes: 1,
            constant: true,
            candidate: false,
        },
        Expr::Variable(_) => FoldShape {
            nodes: 1,
            constant: false,
            candidate: false,
        },
        Expr::Unary { value, .. } => {
            let child = fold_shape(value);
            FoldShape {
                nodes: child.nodes + 1,
                constant: child.constant,
                candidate: child.candidate || child.constant,
            }
        }
        Expr::Binary { left, op, right } => {
            let left_shape = fold_shape(left);
            let right_shape = fold_shape(right);
            let short_circuits = matches!(
                (op, left.unspanned()),
                (BinaryOp::And, Expr::Value(Value::Boolean(false)))
                    | (BinaryOp::Or, Expr::Value(Value::Boolean(true)))
            );
            FoldShape {
                nodes: left_shape.nodes + right_shape.nodes + 1,
                constant: short_circuits || (left_shape.constant && right_shape.constant),
                candidate: left_shape.candidate
                    || right_shape.candidate
                    || short_circuits
                    || (left_shape.constant && right_shape.constant),
            }
        }
        Expr::Invoke {
            function,
            arguments,
        } => {
            let mut nodes = 1;
            let mut constant = !matches!(function, Builtin::Random | Builtin::Chance);
            let mut candidate = false;
            for argument in arguments {
                let child = fold_shape(argument);
                nodes += child.nodes;
                constant &= child.constant;
                candidate |= child.candidate;
            }
            FoldShape {
                nodes,
                constant,
                candidate: candidate || constant,
            }
        }
        Expr::Interpolate { parts } => {
            let mut nodes = 1;
            let mut constant = true;
            let mut candidate = false;
            for part in parts {
                if let StrPart::Hole(expression) = part {
                    let child = fold_shape(expression);
                    nodes += child.nodes;
                    constant &= child.constant;
                    candidate |= child.candidate;
                }
            }
            FoldShape {
                nodes,
                constant,
                candidate: candidate || constant,
            }
        }
    }
}

struct FoldNode {
    end: usize,
    constant: Option<Value>,
}

fn fold_plan(expression: &Expr, line: usize) -> Vec<FoldNode> {
    let shape = fold_shape(expression);
    if !shape.candidate {
        return Vec::new();
    }

    let mut nodes = Vec::with_capacity(shape.nodes);
    build_fold_plan(expression, line, &mut nodes);
    debug_assert_eq!(nodes.len(), shape.nodes);
    nodes
}

fn build_fold_plan(expression: &Expr, line: usize, nodes: &mut Vec<FoldNode>) -> Option<Value> {
    if let Expr::Spanned { expression, .. } = expression {
        return build_fold_plan(expression, line, nodes);
    }

    let start = nodes.len();
    nodes.push(FoldNode {
        end: usize::MAX,
        constant: None,
    });
    let constant = match expression {
        Expr::Spanned { .. } => unreachable!("spans are removed above"),
        Expr::Value(value) => Some(value.clone()),
        Expr::Variable(_) => None,
        Expr::Unary { op, value } => {
            build_fold_plan(value, line, nodes).and_then(|value| apply_unary(*op, value, line).ok())
        }
        Expr::Binary { left, op, right } => {
            let left = build_fold_plan(left, line, nodes);
            let right = build_fold_plan(right, line, nodes);
            match (op, left, right) {
                (BinaryOp::And, Some(Value::Boolean(false)), _) => Some(Value::Boolean(false)),
                (BinaryOp::Or, Some(Value::Boolean(true)), _) => Some(Value::Boolean(true)),
                (_, Some(left), Some(right)) => apply_binary(left, *op, right, line).ok(),
                _ => None,
            }
        }
        Expr::Invoke {
            function,
            arguments,
        } => {
            let mut values = Some(Vec::new());
            for argument in arguments {
                let value = build_fold_plan(argument, line, nodes);
                if let (Some(values), Some(value)) = (&mut values, value) {
                    values.push(value);
                } else {
                    values = None;
                }
            }
            if matches!(function, Builtin::Random | Builtin::Chance) {
                None
            } else {
                values.and_then(|values| invoke(*function, values, line).ok())
            }
        }
        Expr::Interpolate { parts } => fold_interpolation(parts, line, nodes),
    };
    nodes[start].end = nodes.len();
    nodes[start].constant.clone_from(&constant);
    constant
}

fn fold_interpolation(parts: &[StrPart], line: usize, nodes: &mut Vec<FoldNode>) -> Option<Value> {
    let mut text = Some(String::new());
    for part in parts {
        match part {
            StrPart::Literal(literal) => {
                let Some(output) = &mut text else { continue };
                let Some(length) = output.len().checked_add(literal.len()) else {
                    text = None;
                    continue;
                };
                if length > MAX_DATA_TEXT_BYTES {
                    text = None;
                } else {
                    output.push_str(literal);
                }
            }
            StrPart::Hole(expression) => {
                let value = build_fold_plan(expression, line, nodes);
                let rendered = match (&mut text, value) {
                    (Some(output), Some(value)) => {
                        value.append_to_display(output, MAX_DATA_TEXT_BYTES).is_ok()
                    }
                    _ => false,
                };
                if !rendered {
                    text = None;
                }
            }
        }
    }
    text.map(|text| Value::String(text.into()))
}

struct FoldCursor<'a> {
    nodes: &'a [FoldNode],
    at: usize,
}

impl<'a> FoldCursor<'a> {
    fn new(nodes: &'a [FoldNode]) -> Self {
        Self { nodes, at: 0 }
    }

    fn enter(&mut self) -> Option<Value> {
        let node = self.nodes.get(self.at)?;
        self.at += 1;
        if let Some(value) = &node.constant {
            self.at = node.end;
            Some(value.clone())
        } else {
            None
        }
    }
}

fn lower(
    expression: &Expr,
    chunk: &mut ExprChunk,
    slots: &mut SlotTable,
    column: usize,
    folds: &mut FoldCursor<'_>,
) {
    if let Expr::Spanned { span, expression } = expression {
        lower(expression, chunk, slots, span.column, folds);
        return;
    }
    if let Some(value) = folds.enter() {
        let index = chunk.constant(value);
        chunk.push(ExprOp::Const(index));
        return;
    }

    match expression {
        Expr::Spanned { .. } => unreachable!("spans are removed above"),
        Expr::Value(value) => {
            let index = chunk.constant(value.clone());
            chunk.push(ExprOp::Const(index));
        }
        Expr::Variable(name) => {
            let slot = slots.intern(name);
            chunk.push(ExprOp::Load {
                slot,
                column: crate::compact_source_position(column),
            });
        }
        Expr::Unary { op, value } => {
            lower(value, chunk, slots, column, folds);
            chunk.push(ExprOp::Unary(*op));
        }
        Expr::Binary { left, op, right } if matches!(op, BinaryOp::And | BinaryOp::Or) => {
            lower_short_circuit(left, *op, right, chunk, slots, column, folds);
        }
        Expr::Binary { left, op, right } => {
            lower(left, chunk, slots, column, folds);
            lower(right, chunk, slots, column, folds);
            chunk.push(ExprOp::Binary(*op));
        }
        Expr::Invoke {
            function,
            arguments,
        } => {
            for argument in arguments {
                lower(argument, chunk, slots, column, folds);
            }
            match function {
                Builtin::Random => {
                    let state_slot = slots.intern_rng_state();
                    chunk.push(ExprOp::Random { state_slot });
                }
                Builtin::Chance => {
                    let state_slot = slots.intern_rng_state();
                    chunk.push(ExprOp::Chance { state_slot });
                }
                _ => {
                    let argc = u32::try_from(arguments.len()).expect("argument count fits in u32");
                    chunk.push(ExprOp::Call {
                        function: *function,
                        argc,
                    });
                }
            }
        }
        Expr::Interpolate { parts } => {
            // Push every piece (literal segments as string constants, holes as
            // their evaluated value) then fold them into one string. This keeps
            // the display/concat semantics identical to the tree-walker.
            for part in parts {
                match part {
                    StrPart::Literal(text) => {
                        let index = chunk.constant(Value::String(text.clone().into()));
                        chunk.push(ExprOp::Const(index));
                    }
                    StrPart::Hole(expr) => lower(expr, chunk, slots, column, folds),
                }
            }
            let count = u32::try_from(parts.len()).expect("interpolation part count fits in u32");
            chunk.push(ExprOp::Concat(count));
        }
    }
}

/// Lowers `left and right` / `left or right` with short-circuit semantics.
///
/// Layout:
/// ```text
///   <left>                 ; leaves the left boolean on the stack
///   JumpIfFalse END        ; (for `and`) if false, keep it and skip the right
///   <right>                ; leaves the right value above the left
///   Binary And             ; validates and combines both operands
/// END:
/// ```
/// For `or` the guard is `JumpIfTrue`. When the jump is taken the left operand
/// is the whole result. Keeping the left operand on the fallthrough path lets
/// `Binary` reproduce the tree-walker's evaluation order and exact type error.
fn lower_short_circuit(
    left: &Expr,
    op: BinaryOp,
    right: &Expr,
    chunk: &mut ExprChunk,
    slots: &mut SlotTable,
    column: usize,
    folds: &mut FoldCursor<'_>,
) {
    lower(left, chunk, slots, column, folds);
    let guard = chunk.push(match op {
        BinaryOp::And => ExprOp::JumpIfFalse(u32::MAX),
        BinaryOp::Or => ExprOp::JumpIfTrue(u32::MAX),
        _ => unreachable!("only and/or reach short-circuit lowering"),
    });
    lower(right, chunk, slots, column, folds);
    chunk.push(ExprOp::Binary(op));
    let end = u32::try_from(chunk.ops.len()).expect("op index fits in u32");
    chunk.ops[guard] = match op {
        BinaryOp::And => ExprOp::JumpIfFalse(end),
        BinaryOp::Or => ExprOp::JumpIfTrue(end),
        _ => unreachable!(),
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use velin_syntax::Value;

    #[test]
    fn constants_are_deduplicated() {
        let mut slots = SlotTable::new();
        let expr = Expr::Binary {
            left: Box::new(Expr::Binary {
                left: Box::new(Expr::Variable("x".into())),
                op: BinaryOp::Add,
                right: Box::new(Expr::Value(Value::Integer(2))),
            }),
            op: BinaryOp::Add,
            right: Box::new(Expr::Value(Value::Integer(2))),
        };
        let chunk = compile_expression(&expr, &mut slots, 1);
        assert_eq!(chunk.constants.len(), 1);
        assert_eq!(chunk.ops.len(), 5);
    }

    #[test]
    fn folds_successful_constant_subtrees_but_preserves_runtime_errors() {
        let mut slots = SlotTable::new();
        let constant = velin_parse::parse_expression("1 + 2 * 3", "test", 1, 1).unwrap();
        let chunk = compile_expression(&constant, &mut slots, 1);
        assert_eq!(chunk.ops, vec![ExprOp::Const(0)]);
        assert_eq!(chunk.constants, vec![Value::Integer(7)]);

        let dynamic = velin_parse::parse_expression("x + 2 * 3", "test", 1, 1).unwrap();
        let chunk = compile_expression(&dynamic, &mut slots, 1);
        assert_eq!(chunk.ops.len(), 3);
        assert_eq!(chunk.constants, vec![Value::Integer(6)]);

        let failing = velin_parse::parse_expression("1 / 0", "test", 1, 1).unwrap();
        let chunk = compile_expression(&failing, &mut slots, 1);
        assert!(matches!(
            chunk.ops.last(),
            Some(ExprOp::Binary(BinaryOp::Divide))
        ));

        let short = velin_parse::parse_expression("false and missing", "test", 1, 1).unwrap();
        let chunk = compile_expression(&short, &mut slots, 1);
        assert_eq!(chunk.constants, vec![Value::Boolean(false)]);
        assert_eq!(chunk.ops, vec![ExprOp::Const(0)]);
    }

    #[test]
    fn dynamic_expressions_without_constant_subtrees_skip_the_fold_plan() {
        let guard = velin_parse::parse_expression(
            "hp - 10 > 0 and (level * 2 + 5) <= 100 or defeated == false",
            "test",
            1,
            1,
        )
        .unwrap();
        assert!(fold_plan(&guard, 1).is_empty());
    }

    #[test]
    fn folds_pure_arguments_but_not_random_calls() {
        let mut slots = SlotTable::new();
        let random = velin_parse::parse_expression("random(1 + 2, 6)", "test", 1, 1).unwrap();
        let chunk = compile_expression(&random, &mut slots, 1);
        assert_eq!(chunk.constants, vec![Value::Integer(3), Value::Integer(6)]);
        assert!(matches!(chunk.ops.last(), Some(ExprOp::Random { .. })));
    }

    #[test]
    fn folds_constant_interpolation_without_building_an_ast_copy() {
        let mut slots = SlotTable::new();
        let interpolation =
            velin_parse::parse_expression("\"answer [1 + 2]\"", "test", 1, 1).unwrap();
        let chunk = compile_expression(&interpolation, &mut slots, 1);
        assert_eq!(chunk.ops, vec![ExprOp::Const(0)]);
        assert_eq!(chunk.constants, vec![Value::String("answer 3".into())]);
    }

    #[test]
    fn variables_are_interned_into_slots() {
        let mut slots = SlotTable::new();
        let expr = Expr::Binary {
            left: Box::new(Expr::Variable("hp".into())),
            op: BinaryOp::Subtract,
            right: Box::new(Expr::Variable("hp".into())),
        };
        let chunk = compile_expression(&expr, &mut slots, 1);
        assert_eq!(slots.len(), 1); // both loads share one slot
        assert!(matches!(chunk.ops[0], ExprOp::Load { slot: 0, .. }));
        assert!(matches!(chunk.ops[1], ExprOp::Load { slot: 0, .. }));
    }

    #[test]
    fn and_emits_a_forward_guard_over_the_right_operand() {
        let mut slots = SlotTable::new();
        let expr = Expr::Binary {
            left: Box::new(Expr::Variable("a".into())),
            op: BinaryOp::And,
            right: Box::new(Expr::Variable("b".into())),
        };
        let chunk = compile_expression(&expr, &mut slots, 1);
        // Load a, JumpIfFalse END, Load b, Binary And -> END == 4
        match chunk.ops[1] {
            ExprOp::JumpIfFalse(target) => assert_eq!(target as usize, chunk.ops.len()),
            ref other => panic!("expected JumpIfFalse guard, got {other:?}"),
        }
    }

    #[test]
    fn or_emits_a_jump_if_true_guard() {
        let mut slots = SlotTable::new();
        let expr = Expr::Binary {
            left: Box::new(Expr::Variable("a".into())),
            op: BinaryOp::Or,
            right: Box::new(Expr::Variable("b".into())),
        };
        let chunk = compile_expression(&expr, &mut slots, 1);
        match chunk.ops[1] {
            ExprOp::JumpIfTrue(target) => assert_eq!(target as usize, chunk.ops.len()),
            ref other => panic!("expected JumpIfTrue guard, got {other:?}"),
        }
    }

    #[test]
    fn random_builtins_emit_dedicated_ops_with_one_shared_state_slot() {
        let mut slots = SlotTable::new();
        let random = velin_parse::parse_expression("random(1, 6)", "test", 1, 1).unwrap();
        let chance = velin_parse::parse_expression("chance(25)", "test", 2, 1).unwrap();
        let random_chunk = compile_expression(&random, &mut slots, 1);
        let chance_chunk = compile_expression(&chance, &mut slots, 2);
        let state_slot = slots.rng_state().expect("RNG slot allocated");
        assert!(matches!(
            random_chunk.ops.last(),
            Some(ExprOp::Random { state_slot: slot }) if *slot == state_slot
        ));
        assert!(matches!(
            chance_chunk.ops.last(),
            Some(ExprOp::Chance { state_slot: slot }) if *slot == state_slot
        ));
        assert_eq!(
            slots
                .names()
                .iter()
                .filter(|name| name.as_str() == crate::RNG_STATE_SLOT)
                .count(),
            1
        );
    }
}
