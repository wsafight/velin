//! Register expression execution.
//!
//! Every instruction names its source and destination registers. Arithmetic,
//! built-ins, interpolation, random operations, and short-circuit branches all
//! execute through this one path; there is no operand-stack fallback.

use velin_bytecode::{ExprChunk, ExprChunkRef, ExprOp};
use velin_eval::{
    EvalError, apply_binary, apply_boolean_not, apply_integer_binary, apply_integer_unary,
    invoke_measured_with_metrics, invoke_random, invoke_readonly_measured, unassigned,
};
use velin_syntax::{BinaryOp, Builtin, DataFootprint, DataMetrics, MAX_DATA_TEXT_BYTES, Value};

/// A variable frame: slot `i` is assigned when it contains a value.
pub type Frame = [Option<Value>];

pub(crate) enum FrameAccess<'a> {
    ReadOnly(&'a Frame),
    Mutable(&'a mut Frame),
}

impl FrameAccess<'_> {
    fn values(&self) -> &Frame {
        match self {
            Self::ReadOnly(frame) => frame,
            Self::Mutable(frame) => frame,
        }
    }

    fn rng_state(&mut self, slot: u32, line: usize) -> Result<&mut i64, EvalError> {
        let Self::Mutable(frame) = self else {
            unreachable!("validated read-only chunks do not contain random ops")
        };
        rng_state(frame, slot, line)
    }
}

/// Evaluates a standalone, untrusted register chunk.
///
/// # Errors
/// Returns an error for invalid bytecode, unassigned slots, type mismatches,
/// arithmetic failures, invalid built-ins, or resource-limit violations.
pub fn eval_chunk(
    chunk: &ExprChunk,
    frame: &mut Frame,
    slot_name: impl Fn(u32) -> String,
) -> Result<Value, EvalError> {
    chunk.validate(frame.len()).map_err(|error| {
        EvalError::new(chunk.line as usize, format!("invalid bytecode: {error}"))
    })?;
    let mut values = Vec::new();
    let mut metrics = Vec::new();
    eval_validated_chunk(
        chunk.as_chunk_ref(),
        FrameAccess::Mutable(frame),
        None,
        None,
        &mut values,
        &mut metrics,
        slot_name,
    )
    .map(|(value, _)| value)
}

/// Executes one already-validated register chunk with reusable workspaces.
#[allow(clippy::too_many_arguments)]
pub(crate) fn eval_validated_chunk(
    chunk: ExprChunkRef<'_>,
    mut frame: FrameAccess<'_>,
    frame_metrics: Option<(&[DataFootprint], &[u8])>,
    constant_metrics: Option<(&[DataMetrics], u32)>,
    values: &mut Vec<Option<Value>>,
    metrics: &mut Vec<Option<DataMetrics>>,
    slot_name: impl Fn(u32) -> String,
) -> Result<(Value, DataMetrics), EvalError> {
    let register_count = usize::from(chunk.registers);
    reset_workspace(values, register_count);
    reset_workspace(metrics, register_count);
    let result = execute_registers(
        chunk,
        &mut frame,
        frame_metrics,
        constant_metrics,
        values,
        metrics,
        &slot_name,
    );
    clear_workspace(values, register_count);
    clear_workspace(metrics, register_count);
    result
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn execute_registers(
    chunk: ExprChunkRef<'_>,
    frame: &mut FrameAccess<'_>,
    frame_metrics: Option<(&[DataFootprint], &[u8])>,
    constant_metrics: Option<(&[DataMetrics], u32)>,
    values: &mut [Option<Value>],
    metrics: &mut [Option<DataMetrics>],
    slot_name: &impl Fn(u32) -> String,
) -> Result<(Value, DataMetrics), EvalError> {
    let line = chunk.line as usize;
    let mut pc = 0;
    while pc < chunk.ops.len() {
        match &chunk.ops[pc] {
            ExprOp::Const { dst, constant } => {
                let value = chunk.constants[*constant as usize].clone();
                let value_metrics = constant_metrics
                    .and_then(|(all, start)| all.get((start + *constant) as usize).copied())
                    .unwrap_or_else(|| {
                        value
                            .data_metrics()
                            .expect("validated register constant metrics")
                    });
                write_register(values, metrics, *dst, value, value_metrics);
            }
            ExprOp::Load { dst, slot, .. } => {
                let value = frame.values()[*slot as usize]
                    .clone()
                    .ok_or_else(|| unassigned(line, &slot_name(*slot)))?;
                let value_metrics = frame_metrics.map_or_else(
                    || {
                        value
                            .data_metrics()
                            .expect("validated register slot metrics")
                    },
                    |(footprints, depths)| DataMetrics {
                        footprint: footprints[*slot as usize],
                        max_depth: usize::from(depths[*slot as usize]),
                    },
                );
                write_register(values, metrics, *dst, value, value_metrics);
            }
            ExprOp::Unary { dst, op, source } => {
                let source = register_value(values, *source).clone();
                let result = apply_unary_typed(*op, source, line)?;
                let result_metrics = scalar_metrics(&result);
                write_register(values, metrics, *dst, result, result_metrics);
            }
            ExprOp::Binary {
                dst,
                left,
                op,
                right,
            } => {
                let left = register_value(values, *left).clone();
                let right = register_value(values, *right).clone();
                let result = apply_binary_typed(left, *op, right, line)?;
                let result_metrics = shallow_metrics(&result);
                write_register(values, metrics, *dst, result, result_metrics);
            }
            ExprOp::Call {
                dst,
                function,
                args,
            } => {
                let (result, result_metrics) =
                    if matches!(function, Builtin::Len | Builtin::Get | Builtin::Contains) {
                        invoke_readonly_registers(*function, values, args.clone(), line)?
                    } else {
                        let (arguments, argument_metrics) =
                            collect_registers(values, metrics, args.clone());
                        invoke_measured_with_metrics(*function, arguments, &argument_metrics, line)?
                    };
                write_register(values, metrics, *dst, result, result_metrics);
            }
            ExprOp::Random {
                dst,
                args,
                state_slot,
            } => {
                let arguments = collect_values(values, args.clone());
                let result = invoke_random(
                    Builtin::Random,
                    &arguments,
                    frame.rng_state(*state_slot, line)?,
                    line,
                )?;
                let result_metrics = scalar_metrics(&result);
                write_register(values, metrics, *dst, result, result_metrics);
            }
            ExprOp::Chance {
                dst,
                args,
                state_slot,
            } => {
                let arguments = collect_values(values, args.clone());
                let result = invoke_random(
                    Builtin::Chance,
                    &arguments,
                    frame.rng_state(*state_slot, line)?,
                    line,
                )?;
                let result_metrics = scalar_metrics(&result);
                write_register(values, metrics, *dst, result, result_metrics);
            }
            ExprOp::JumpIfFalse { condition, target } => {
                if register_value(values, *condition) == &Value::Boolean(false) {
                    pc = *target as usize;
                    continue;
                }
            }
            ExprOp::JumpIfTrue { condition, target } => {
                if register_value(values, *condition) == &Value::Boolean(true) {
                    pc = *target as usize;
                    continue;
                }
            }
            ExprOp::Concat {
                dst,
                values: sources,
            } => {
                let rendered_bytes = sources.clone().try_fold(0usize, |total, register| {
                    let length = register_value(values, register)
                        .display_len_known()
                        .map_err(|error| EvalError::new(line, error))?;
                    total
                        .checked_add(length)
                        .ok_or_else(|| EvalError::new(line, "string size overflow"))
                })?;
                if rendered_bytes > MAX_DATA_TEXT_BYTES {
                    return Err(EvalError::new(line, "data text exceeds 1 MiB"));
                }
                let mut text = String::with_capacity(rendered_bytes);
                for register in sources.clone() {
                    register_value(values, register)
                        .append_to_display_known(&mut text, MAX_DATA_TEXT_BYTES)
                        .map_err(|error| EvalError::new(line, error))?;
                }
                let result_metrics = string_metrics(&text);
                write_register(
                    values,
                    metrics,
                    *dst,
                    Value::String(text.into()),
                    result_metrics,
                );
            }
        }
        pc += 1;
    }
    let result = values[chunk.result as usize]
        .take()
        .expect("validated register result");
    let result_metrics = metrics[chunk.result as usize]
        .take()
        .expect("validated register result metrics");
    Ok((result, result_metrics))
}

fn collect_values(values: &[Option<Value>], registers: std::ops::Range<u16>) -> Vec<Value> {
    registers
        .map(|register| register_value(values, register).clone())
        .collect()
}

fn invoke_readonly_registers(
    function: Builtin,
    values: &[Option<Value>],
    registers: std::ops::Range<u16>,
    line: usize,
) -> Result<(Value, DataMetrics), EvalError> {
    let result = match registers.len() {
        1 => {
            let arguments = [register_value(values, registers.start)];
            invoke_readonly_measured(function, &arguments, line)?
        }
        2 => {
            let arguments = [
                register_value(values, registers.start),
                register_value(values, registers.start + 1),
            ];
            invoke_readonly_measured(function, &arguments, line)?
        }
        3 => {
            let arguments = [
                register_value(values, registers.start),
                register_value(values, registers.start + 1),
                register_value(values, registers.start + 2),
            ];
            invoke_readonly_measured(function, &arguments, line)?
        }
        _ => unreachable!("validated read-only built-in arity"),
    };
    Ok(result)
}

fn collect_registers(
    values: &[Option<Value>],
    metrics: &[Option<DataMetrics>],
    registers: std::ops::Range<u16>,
) -> (Vec<Value>, Vec<DataMetrics>) {
    let mut arguments = Vec::with_capacity(registers.len());
    let mut argument_metrics = Vec::with_capacity(registers.len());
    for register in registers {
        arguments.push(register_value(values, register).clone());
        argument_metrics
            .push(metrics[register as usize].expect("validated register argument metrics"));
    }
    (arguments, argument_metrics)
}

fn register_value(values: &[Option<Value>], register: u16) -> &Value {
    values[register as usize]
        .as_ref()
        .expect("validated register source")
}

fn write_register(
    values: &mut [Option<Value>],
    metrics: &mut [Option<DataMetrics>],
    dst: u16,
    value: Value,
    value_metrics: DataMetrics,
) {
    values[dst as usize] = Some(value);
    metrics[dst as usize] = Some(value_metrics);
}

fn reset_workspace<T>(workspace: &mut Vec<Option<T>>, len: usize) {
    if workspace.len() < len {
        workspace.resize_with(len, || None);
    }
    clear_workspace(workspace, len);
}

fn clear_workspace<T>(workspace: &mut [Option<T>], len: usize) {
    for value in &mut workspace[..len] {
        *value = None;
    }
}

#[inline]
fn apply_unary_typed(
    op: velin_syntax::UnaryOp,
    value: Value,
    line: usize,
) -> Result<Value, EvalError> {
    match (op, &value) {
        (velin_syntax::UnaryOp::Negate, Value::Integer(value)) => apply_integer_unary(*value, line),
        (velin_syntax::UnaryOp::Not, Value::Boolean(value)) => Ok(apply_boolean_not(*value)),
        _ => velin_eval::apply_unary(op, value, line),
    }
}

#[inline]
fn apply_binary_typed(
    left: Value,
    op: BinaryOp,
    right: Value,
    line: usize,
) -> Result<Value, EvalError> {
    if matches!(
        op,
        BinaryOp::Subtract | BinaryOp::Multiply | BinaryOp::Divide
    ) && let (Value::Integer(left), Value::Integer(right)) = (&left, &right)
    {
        return apply_integer_binary(*left, op, *right, line);
    }
    apply_binary(left, op, right, line)
}

fn scalar_metrics(_value: &Value) -> DataMetrics {
    DataMetrics {
        footprint: DataFootprint {
            values: 1,
            text_bytes: 0,
        },
        max_depth: 0,
    }
}

fn string_metrics(text: &str) -> DataMetrics {
    DataMetrics {
        footprint: DataFootprint {
            values: 1,
            text_bytes: text.len(),
        },
        max_depth: 0,
    }
}

pub(crate) fn shallow_metrics(value: &Value) -> DataMetrics {
    match value {
        Value::String(text) => string_metrics(text),
        _ => scalar_metrics(value),
    }
}

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
#[path = "chunk_tests.rs"]
mod tests;
