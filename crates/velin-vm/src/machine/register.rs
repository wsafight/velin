use super::{
    DataMetrics, EvalError, FrameState, Program, RegisterExpr, RegisterOp, RegisterType, Value,
};
use crate::chunk::shallow_metrics;
use velin_eval::{apply_binary, apply_boolean_not, apply_integer_binary, apply_integer_unary};

/// Executes the non-serialized scalar plan prepared for a validated chunk.
///
/// The workspace is reused between evaluations, but every operation validates its
/// operands through the shared evaluator when the inferred type is unknown or
/// does not match the runtime value. This keeps the plan an optimization hint,
/// never a second source of language semantics.
pub(super) fn eval_register_expr(
    program: &Program,
    frame: &FrameState,
    expression: &RegisterExpr,
    values: &mut Vec<Option<Value>>,
    line: usize,
) -> Result<(Value, DataMetrics), EvalError> {
    let register_count = expression.registers as usize;
    if values.len() < register_count {
        values.resize_with(register_count, || None);
    }
    for operation in &expression.ops {
        match *operation {
            RegisterOp::LoadConstant { dst, constant, .. } => {
                values[dst as usize] = Some(program.constants[constant as usize].clone());
            }
            RegisterOp::LoadSlot { dst, slot, .. } => {
                let value = frame.values[slot as usize].clone().ok_or_else(|| {
                    velin_eval::unassigned(line, program.slots.name(slot).unwrap_or("?"))
                })?;
                values[dst as usize] = Some(value);
            }
            RegisterOp::Unary {
                dst,
                op,
                source,
                result_type,
            } => {
                let value = values[source as usize]
                    .take()
                    .expect("validated register unary operand");
                values[dst as usize] = Some(apply_unary(result_type, op, value, line)?);
            }
            RegisterOp::Binary {
                dst,
                left,
                op,
                right,
                result_type,
            } => {
                let right = values[right as usize]
                    .take()
                    .expect("validated register binary right operand");
                let left = values[left as usize]
                    .take()
                    .expect("validated register binary left operand");
                values[dst as usize] =
                    Some(apply_binary_typed(left, op, right, result_type, line)?);
            }
        }
    }
    let result = values[expression.result as usize]
        .take()
        .expect("validated register result");
    let result_metrics = shallow_metrics(&result);
    Ok((result, result_metrics))
}

#[inline]
fn apply_unary(
    result_type: RegisterType,
    op: velin_syntax::UnaryOp,
    value: Value,
    line: usize,
) -> Result<Value, EvalError> {
    match (result_type, &value) {
        (RegisterType::Integer, Value::Integer(value)) if op == velin_syntax::UnaryOp::Negate => {
            apply_integer_unary(*value, line)
        }
        (RegisterType::Boolean, Value::Boolean(value)) if op == velin_syntax::UnaryOp::Not => {
            Ok(apply_boolean_not(*value))
        }
        _ => velin_eval::apply_unary(op, value, line),
    }
}

#[inline]
fn apply_binary_typed(
    left: Value,
    op: velin_syntax::BinaryOp,
    right: Value,
    result_type: RegisterType,
    line: usize,
) -> Result<Value, EvalError> {
    if result_type == RegisterType::Integer
        && let (Value::Integer(left), Value::Integer(right)) = (&left, &right)
    {
        return apply_integer_binary(*left, op, *right, line);
    }
    apply_binary(left, op, right, line)
}
