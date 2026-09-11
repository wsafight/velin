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
use velin_syntax::{BinaryOp, Builtin, Expr, StrPart, Value};

/// Compiles `expression` into an [`ExprChunk`], interning any referenced
/// variables into `slots`.
///
/// `line` is recorded on the chunk for runtime error reporting.
#[must_use]
pub fn compile_expression(expression: &Expr, slots: &mut SlotTable, line: usize) -> ExprChunk {
    let mut chunk = ExprChunk::new(line);
    lower(expression, &mut chunk, slots, 1);
    chunk
}

fn lower(expression: &Expr, chunk: &mut ExprChunk, slots: &mut SlotTable, column: usize) {
    match expression {
        Expr::Spanned { span, expression } => lower(expression, chunk, slots, span.column),
        Expr::Value(value) => {
            let index = chunk.constant(value.clone());
            chunk.push(ExprOp::Const(index));
        }
        Expr::Variable(name) => {
            let slot = slots.intern(name);
            chunk.push(ExprOp::Load { slot, column });
        }
        Expr::Unary { op, value } => {
            lower(value, chunk, slots, column);
            chunk.push(ExprOp::Unary(*op));
        }
        Expr::Binary { left, op, right } if matches!(op, BinaryOp::And | BinaryOp::Or) => {
            lower_short_circuit(left, *op, right, chunk, slots, column);
        }
        Expr::Binary { left, op, right } => {
            lower(left, chunk, slots, column);
            lower(right, chunk, slots, column);
            chunk.push(ExprOp::Binary(*op));
        }
        Expr::Invoke {
            function,
            arguments,
        } => {
            for argument in arguments {
                lower(argument, chunk, slots, column);
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
                        let index = chunk.constant(Value::String(text.clone()));
                        chunk.push(ExprOp::Const(index));
                    }
                    StrPart::Hole(expr) => lower(expr, chunk, slots, column),
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
///   <right>                ; pops the surviving left, evaluates right
/// END:
/// ```
/// For `or` the guard is `JumpIfTrue`. When the jump is taken the left operand
/// is the whole result; when it falls through, the left is popped and the right
/// becomes the result. Type errors (a non-boolean operand) surface in the VM,
/// matching the tree-walker.
fn lower_short_circuit(
    left: &Expr,
    op: BinaryOp,
    right: &Expr,
    chunk: &mut ExprChunk,
    slots: &mut SlotTable,
    column: usize,
) {
    lower(left, chunk, slots, column);
    let guard = chunk.push(match op {
        BinaryOp::And => ExprOp::JumpIfFalse(u32::MAX),
        BinaryOp::Or => ExprOp::JumpIfTrue(u32::MAX),
        _ => unreachable!("only and/or reach short-circuit lowering"),
    });
    lower(right, chunk, slots, column);
    chunk.push(ExprOp::AssertBoolean(op));
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
            left: Box::new(Expr::Value(Value::Integer(2))),
            op: BinaryOp::Add,
            right: Box::new(Expr::Value(Value::Integer(2))),
        };
        let chunk = compile_expression(&expr, &mut slots, 1);
        assert_eq!(chunk.constants.len(), 1);
        assert_eq!(chunk.ops.len(), 3); // Const, Const, Binary
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
        // Load a, JumpIfFalse END, Load b  -> END == 3
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
