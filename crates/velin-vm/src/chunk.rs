//! The expression stack machine.
//!
//! Executes one [`ExprChunk`] against a frame of variable slots and returns the
//! single [`Value`] it leaves on the operand stack. All the actual arithmetic,
//! comparison, and built-in semantics are delegated to `velin-eval`, so the
//! bytecode path and the tree-walker are guaranteed to agree operation for
//! operation.

use velin_compile::{ExprChunk, ExprOp};
use velin_eval::{EvalError, apply_binary, apply_unary, invoke, invoke_random, unassigned};
use velin_syntax::{BinaryOp, Builtin, DataFootprint, MAX_DATA_TEXT_BYTES, Value};

/// A variable frame: slot `i` holds `Some(value)` once assigned, `None`
/// otherwise. Reading a `None` slot reproduces the tree-walker's
/// "unassigned on this path" error.
pub type Frame = [Option<Value>];

/// Evaluates `chunk` against `frame`, returning the resulting [`Value`].
///
/// The frame is mutable because dedicated random ops explicitly advance their
/// state slot. Chunks without random ops leave it untouched.
///
/// # Errors
/// Returns [`EvalError`] when the chunk is malformed or exceeds a bytecode
/// budget, or for an unassigned slot, type mismatch, integer overflow,
/// division by zero, or built-in failure. The chunk's recorded `line` is used
/// for reporting.
pub fn eval_chunk(
    chunk: &ExprChunk,
    frame: &mut Frame,
    slot_name: impl Fn(u32) -> String,
) -> Result<Value, EvalError> {
    chunk
        .validate(frame.len())
        .map_err(|error| EvalError::new(chunk.line, format!("invalid bytecode: {error}")))?;
    eval_validated_chunk(chunk, frame, slot_name).map(|(value, _)| value)
}

/// Executes a chunk belonging to a `Program` already validated by `Machine`.
pub(crate) fn eval_validated_chunk(
    chunk: &ExprChunk,
    frame: &mut Frame,
    slot_name: impl Fn(u32) -> String,
) -> Result<(Value, DataFootprint), EvalError> {
    let line = chunk.line;
    let mut stack: Vec<Value> = Vec::new();
    let mut pc = 0;
    while pc < chunk.ops.len() {
        match &chunk.ops[pc] {
            ExprOp::Const(index) => stack.push(chunk.constants[*index as usize].clone()),
            ExprOp::Load { slot, .. } => {
                let value = frame
                    .get(*slot as usize)
                    .and_then(Option::as_ref)
                    .cloned()
                    .ok_or_else(|| unassigned(line, &slot_name(*slot)))?;
                stack.push(value);
            }
            ExprOp::Unary(op) => {
                let value = stack.pop().expect("unary operand present");
                stack.push(apply_unary(*op, value, line)?);
            }
            ExprOp::Binary(op) => {
                let right = stack.pop().expect("binary right operand present");
                let left = stack.pop().expect("binary left operand present");
                stack.push(apply_binary(left, *op, right, line)?);
            }
            ExprOp::Call { function, argc } => {
                let at = stack.len() - *argc as usize;
                let arguments = stack.split_off(at);
                stack.push(invoke(*function, arguments, line)?);
            }
            ExprOp::Random { state_slot } => {
                let at = stack.len() - 2;
                let arguments = stack.split_off(at);
                stack.push(invoke_random(
                    Builtin::Random,
                    &arguments,
                    rng_state(frame, *state_slot, line)?,
                    line,
                )?);
            }
            ExprOp::Chance { state_slot } => {
                let argument = stack.pop().expect("chance percentage present");
                stack.push(invoke_random(
                    Builtin::Chance,
                    &[argument],
                    rng_state(frame, *state_slot, line)?,
                    line,
                )?);
            }
            ExprOp::Concat(count) => {
                let at = stack.len() - *count as usize;
                let pieces = stack.split_off(at);
                let mut text = String::new();
                for piece in &pieces {
                    let rendered = piece
                        .try_to_display()
                        .map_err(|error| EvalError::new(line, error))?;
                    let length = text
                        .len()
                        .checked_add(rendered.len())
                        .ok_or_else(|| EvalError::new(line, "data text exceeds 1 MiB"))?;
                    if length > MAX_DATA_TEXT_BYTES {
                        return Err(EvalError::new(line, "data text exceeds 1 MiB"));
                    }
                    text.push_str(&rendered);
                }
                stack.push(Value::String(text));
            }
            ExprOp::JumpIfFalse(target) => {
                if stack.last() == Some(&Value::Boolean(false)) {
                    pc = *target as usize;
                    continue;
                }
            }
            ExprOp::JumpIfTrue(target) => {
                if stack.last() == Some(&Value::Boolean(true)) {
                    pc = *target as usize;
                    continue;
                }
            }
            ExprOp::AssertBoolean(op) => {
                let value = stack.last().expect("boolean operand present");
                if !matches!(value, Value::Boolean(_)) {
                    let left = Value::Boolean(*op == BinaryOp::And);
                    return apply_binary(left, *op, value.clone(), line)
                        .map(|_| unreachable!("non-boolean operation cannot succeed"));
                }
            }
        }
        pc += 1;
    }
    let result = stack.pop().expect("chunk leaves exactly one value");
    let footprint = result
        .data_footprint()
        .map_err(|error| EvalError::new(line, error))?;
    Ok((result, footprint))
}

/// Resolves the serializable integer slot that owns RNG state.
fn rng_state(frame: &mut Frame, slot: u32, line: usize) -> Result<&mut i64, EvalError> {
    let value = frame
        .get_mut(slot as usize)
        .and_then(Option::as_mut)
        .ok_or_else(|| EvalError::new(line, "RNG state has not been initialized"))?;
    let Value::Integer(state) = value else {
        return Err(EvalError::new(line, "RNG state must be an integer"));
    };
    Ok(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use velin_compile::SlotTable;
    use velin_compile::compile_expression;
    use velin_syntax::Expr;

    fn run(
        expr: &Expr,
        frame: &mut [Option<Value>],
        slots: &SlotTable,
    ) -> Result<Value, EvalError> {
        let mut table = slots.clone();
        let chunk = compile_expression(expr, &mut table, 1);
        eval_chunk(&chunk, frame, |slot| {
            table.name(slot).unwrap_or("?").to_owned()
        })
    }

    #[test]
    fn evaluates_arithmetic_and_builtins() {
        let slots = SlotTable::new();
        let expr = Expr::Binary {
            left: Box::new(Expr::Value(Value::Integer(1))),
            op: BinaryOp::Add,
            right: Box::new(Expr::Binary {
                left: Box::new(Expr::Value(Value::Integer(2))),
                op: BinaryOp::Multiply,
                right: Box::new(Expr::Value(Value::Integer(3))),
            }),
        };
        assert_eq!(run(&expr, &mut [], &slots).unwrap(), Value::Integer(7));
    }

    #[test]
    fn short_circuit_skips_unassigned_right_operand() {
        let mut slots = SlotTable::new();
        slots.intern("a");
        slots.intern("missing");
        let and = Expr::Binary {
            left: Box::new(Expr::Value(Value::Boolean(false))),
            op: BinaryOp::And,
            right: Box::new(Expr::Variable("missing".into())),
        };
        assert_eq!(
            run(&and, &mut [None, None], &slots).unwrap(),
            Value::Boolean(false)
        );
        let or = Expr::Binary {
            left: Box::new(Expr::Value(Value::Boolean(true))),
            op: BinaryOp::Or,
            right: Box::new(Expr::Variable("missing".into())),
        };
        assert_eq!(
            run(&or, &mut [None, None], &slots).unwrap(),
            Value::Boolean(true)
        );
    }

    #[test]
    fn unassigned_slot_names_the_variable() {
        let mut slots = SlotTable::new();
        slots.intern("hp");
        let error = run(&Expr::Variable("hp".into()), &mut [None], &slots).unwrap_err();
        assert!(error.message.contains("`hp`"));
    }

    #[test]
    fn concat_budget_non_boolean_or_and_rng_type() {
        let slots = SlotTable::new();
        let oversized = Expr::Interpolate {
            parts: vec![
                velin_syntax::StrPart::Literal("x".repeat(MAX_DATA_TEXT_BYTES / 2 + 1)),
                velin_syntax::StrPart::Literal("y".repeat(MAX_DATA_TEXT_BYTES / 2 + 1)),
            ],
        };
        assert!(
            run(&oversized, &mut [], &slots)
                .unwrap_err()
                .message
                .contains("exceeds 1 MiB")
        );

        let or = Expr::Binary {
            left: Box::new(Expr::Value(Value::Integer(1))),
            op: BinaryOp::Or,
            right: Box::new(Expr::Value(Value::Boolean(true))),
        };
        assert!(run(&or, &mut [], &slots).is_err());

        let chance = velin_parse::parse_expression("chance(50)", "t", 1, 1).unwrap();
        let mut table = SlotTable::new();
        let chunk = compile_expression(&chance, &mut table, 1);
        let rng = table.rng_state().unwrap();
        let mut frame = vec![None; table.len()];
        frame[rng as usize] = Some(Value::Boolean(true));
        let error = eval_chunk(&chunk, &mut frame, |slot| {
            table.name(slot).unwrap_or("?").to_owned()
        })
        .unwrap_err();
        assert!(error.message.contains("integer"));
    }

    #[test]
    fn malformed_chunks_return_errors_instead_of_panicking() {
        let chunk = ExprChunk {
            ops: vec![ExprOp::Binary(BinaryOp::Add)],
            constants: Vec::new(),
            line: 7,
        };
        let error = eval_chunk(&chunk, &mut [], |_| "?".to_owned()).unwrap_err();
        assert_eq!(error.line, 7);
        assert!(error.message.contains("invalid bytecode"));
    }
}
