//! Register expression execution.
//!
//! Every instruction names its source and destination registers. Arithmetic,
//! built-ins, interpolation, random operations, and short-circuit branches all
//! execute through this one path; there is no operand-stack fallback.

use velin_bytecode::{ExprChunk, ExprChunkRef, ExprOp};
use velin_eval::{
    EvalError, apply_binary, apply_boolean_not, apply_integer_binary, apply_integer_unary,
    invoke_measured_with_metrics_reusable, invoke_random, unassigned,
};
use velin_syntax::{BinaryOp, Builtin, DataFootprint, DataMetrics, MAX_DATA_TEXT_BYTES, Value};

#[path = "chunk_metrics.rs"]
mod metrics_helpers;
use metrics_helpers::{rng_state, scalar_metrics, shallow_metrics, string_metrics};

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
    let mut touched = Vec::new();
    let mut arguments = Vec::new();
    let mut argument_metrics = Vec::new();
    eval_validated_chunk(
        chunk.as_chunk_ref(),
        FrameAccess::Mutable(frame),
        None,
        None,
        &mut values,
        &mut metrics,
        &mut touched,
        &mut arguments,
        &mut argument_metrics,
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
    touched: &mut Vec<usize>,
    arguments: &mut Vec<Value>,
    argument_metrics: &mut Vec<DataMetrics>,
    slot_name: impl Fn(u32) -> String,
) -> Result<(Value, DataMetrics), EvalError> {
    let register_count = usize::from(chunk.registers);
    reset_workspace(values, metrics, touched, register_count);
    let result = execute_registers(
        chunk,
        &mut frame,
        frame_metrics,
        constant_metrics,
        values,
        metrics,
        touched,
        arguments,
        argument_metrics,
        &slot_name,
    );
    clear_workspace(values, metrics, touched);
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
    touched: &mut Vec<usize>,
    arguments: &mut Vec<Value>,
    argument_metrics: &mut Vec<DataMetrics>,
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
                write_register(values, metrics, touched, *dst, value, value_metrics);
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
                write_register(values, metrics, touched, *dst, value, value_metrics);
            }
            ExprOp::Unary { dst, op, source } => {
                let source = register_value(values, *source);
                let result = apply_unary_typed_ref(*op, source, line)?;
                let result_metrics = scalar_metrics(&result);
                write_register(values, metrics, touched, *dst, result, result_metrics);
            }
            ExprOp::Binary {
                dst,
                left,
                op,
                right,
            } => {
                let left = register_value(values, *left);
                let right = register_value(values, *right);
                let result = apply_binary_typed_ref(left, *op, right, line)?;
                let result_metrics = shallow_metrics(&result);
                write_register(values, metrics, touched, *dst, result, result_metrics);
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
                        collect_registers(
                            values,
                            metrics,
                            args.clone(),
                            arguments,
                            argument_metrics,
                        );
                        let result = invoke_measured_with_metrics_reusable(
                            *function,
                            arguments,
                            argument_metrics,
                            line,
                        );
                        arguments.clear();
                        argument_metrics.clear();
                        result?
                    };
                write_register(values, metrics, touched, *dst, result, result_metrics);
            }
            ExprOp::Random {
                dst,
                args,
                state_slot,
            } => {
                collect_values(values, args.clone(), arguments);
                let result = invoke_random(
                    Builtin::Random,
                    arguments,
                    frame.rng_state(*state_slot, line)?,
                    line,
                );
                arguments.clear();
                let result = result?;
                let result_metrics = scalar_metrics(&result);
                write_register(values, metrics, touched, *dst, result, result_metrics);
            }
            ExprOp::Chance {
                dst,
                args,
                state_slot,
            } => {
                collect_values(values, args.clone(), arguments);
                let result = invoke_random(
                    Builtin::Chance,
                    arguments,
                    frame.rng_state(*state_slot, line)?,
                    line,
                );
                arguments.clear();
                let result = result?;
                let result_metrics = scalar_metrics(&result);
                write_register(values, metrics, touched, *dst, result, result_metrics);
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
                    touched,
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

fn collect_values(
    values: &[Option<Value>],
    registers: std::ops::Range<u16>,
    arguments: &mut Vec<Value>,
) {
    arguments.clear();
    arguments.extend(registers.map(|register| register_value(values, register).clone()));
}

fn invoke_readonly_registers(
    function: Builtin,
    values: &[Option<Value>],
    registers: std::ops::Range<u16>,
    line: usize,
) -> Result<(Value, DataMetrics), EvalError> {
    let argc = registers.len();
    if !function.accepts(argc) {
        return Err(EvalError::new(line, "invalid built-in argument count"));
    }
    let first = register_value(values, registers.start);
    match function {
        Builtin::Len => {
            let length = match first {
                Value::List(values) => values.len(),
                Value::Record(values) => values.len(),
                Value::String(value) => value.chars().count(),
                _ => return Err(EvalError::new(line, "len expects a list, record or string")),
            };
            let length =
                i64::try_from(length).map_err(|_| EvalError::new(line, "length overflow"))?;
            Ok((
                Value::Integer(length),
                scalar_metrics(&Value::Integer(length)),
            ))
        }
        Builtin::Get => {
            let second = register_value(values, registers.start + 1);
            let value = match (first, second) {
                (Value::List(values), Value::Integer(index)) => usize::try_from(*index)
                    .ok()
                    .and_then(|index| values.get(index)),
                (Value::Record(values), Value::String(key)) => values.get(key.as_str()),
                _ => {
                    return Err(EvalError::new(
                        line,
                        "get expects a list and integer index, or a record and string key",
                    ));
                }
            };
            let value = value
                .or_else(|| (argc == 3).then(|| register_value(values, registers.start + 2)))
                .cloned()
                .ok_or_else(|| EvalError::new(line, "missing key or list index"))?;
            let metrics = value
                .data_metrics()
                .map_err(|error| EvalError::new(line, error))?;
            Ok((value, metrics))
        }
        Builtin::Contains => {
            let second = register_value(values, registers.start + 1);
            let contains = match (first, second) {
                (Value::List(values), value) => values.contains(value),
                (Value::Record(values), Value::String(key)) => values.contains_key(key.as_str()),
                (Value::String(text), Value::String(part)) => text.contains(part.as_str()),
                _ => {
                    return Err(EvalError::new(
                        line,
                        "contains expects a list, record or string",
                    ));
                }
            };
            Ok((
                Value::Boolean(contains),
                scalar_metrics(&Value::Boolean(contains)),
            ))
        }
        _ => unreachable!("validated read-only built-in"),
    }
}

fn collect_registers(
    values: &[Option<Value>],
    metrics: &[Option<DataMetrics>],
    registers: std::ops::Range<u16>,
    arguments: &mut Vec<Value>,
    argument_metrics: &mut Vec<DataMetrics>,
) {
    arguments.clear();
    argument_metrics.clear();
    arguments.reserve(registers.len());
    argument_metrics.reserve(registers.len());
    for register in registers {
        arguments.push(register_value(values, register).clone());
        argument_metrics
            .push(metrics[register as usize].expect("validated register argument metrics"));
    }
}

fn register_value(values: &[Option<Value>], register: u16) -> &Value {
    values[register as usize]
        .as_ref()
        .expect("validated register source")
}

fn write_register(
    values: &mut [Option<Value>],
    metrics: &mut [Option<DataMetrics>],
    touched: &mut Vec<usize>,
    dst: u16,
    value: Value,
    value_metrics: DataMetrics,
) {
    let index = dst as usize;
    if values[index].is_none() {
        touched.push(index);
    }
    values[index] = Some(value);
    metrics[index] = Some(value_metrics);
}

fn reset_workspace(
    values: &mut Vec<Option<Value>>,
    metrics: &mut Vec<Option<DataMetrics>>,
    touched: &mut Vec<usize>,
    len: usize,
) {
    for index in touched.drain(..) {
        values[index] = None;
        metrics[index] = None;
    }
    if values.len() < len {
        values.resize_with(len, || None);
    }
    if metrics.len() < len {
        metrics.resize_with(len, || None);
    }
    touched.reserve(len.min(8));
}

fn clear_workspace(
    values: &mut [Option<Value>],
    metrics: &mut [Option<DataMetrics>],
    touched: &mut Vec<usize>,
) {
    for index in touched.drain(..) {
        values[index] = None;
        metrics[index] = None;
    }
}

#[inline]
fn apply_unary_typed_ref(
    op: velin_syntax::UnaryOp,
    value: &Value,
    line: usize,
) -> Result<Value, EvalError> {
    match (op, value) {
        (velin_syntax::UnaryOp::Negate, Value::Integer(value)) => apply_integer_unary(*value, line),
        (velin_syntax::UnaryOp::Not, Value::Boolean(value)) => Ok(apply_boolean_not(*value)),
        _ => velin_eval::apply_unary(op, value.clone(), line),
    }
}

#[inline]
fn apply_binary_typed_ref(
    left: &Value,
    op: BinaryOp,
    right: &Value,
    line: usize,
) -> Result<Value, EvalError> {
    if let (Value::Integer(left), Value::Integer(right)) = (left, right) {
        return match op {
            BinaryOp::Add | BinaryOp::Subtract | BinaryOp::Multiply | BinaryOp::Divide => {
                apply_integer_binary(*left, op, *right, line)
            }
            BinaryOp::Equal => Ok(Value::Boolean(left == right)),
            BinaryOp::NotEqual => Ok(Value::Boolean(left != right)),
            BinaryOp::Less => Ok(Value::Boolean(left < right)),
            BinaryOp::LessEqual => Ok(Value::Boolean(left <= right)),
            BinaryOp::Greater => Ok(Value::Boolean(left > right)),
            BinaryOp::GreaterEqual => Ok(Value::Boolean(left >= right)),
            _ => velin_eval::apply_binary(Value::Integer(*left), op, Value::Integer(*right), line),
        };
    }
    if let (Value::Boolean(left), Value::Boolean(right)) = (left, right) {
        return match op {
            BinaryOp::Equal => Ok(Value::Boolean(left == right)),
            BinaryOp::NotEqual => Ok(Value::Boolean(left != right)),
            BinaryOp::And => Ok(Value::Boolean(*left && *right)),
            BinaryOp::Or => Ok(Value::Boolean(*left || *right)),
            _ => velin_eval::apply_binary(Value::Boolean(*left), op, Value::Boolean(*right), line),
        };
    }
    apply_binary(left.clone(), op, right.clone(), line)
}

#[cfg(test)]
#[path = "chunk_tests.rs"]
mod tests;
