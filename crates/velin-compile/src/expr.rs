//! Lowering from the `Expr` tree to a flat [`ExprChunk`].
//!
//! The compiler assigns an explicit destination register to every expression.
//! Operands are emitted before the instruction that consumes them. The only
//! subtlety is `and`/`or`, which must short-circuit: they are lowered to
//! conditional jumps so the right operand is skipped when the result is
//! already determined by the left operand.

use velin_bytecode::{ExprChunk, ExprOp, SlotTable};
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
    chunk.ops.shrink_to_fit();
    chunk.constants.shrink_to_fit();
    chunk
}

#[inline]
pub(crate) fn compile_expression_into(
    expression: &Expr,
    slots: &mut SlotTable,
    line: usize,
    chunk: &mut ExprChunk,
) {
    chunk.reset(line);
    let plan = fold_plan(expression, line);
    let result = chunk.result;
    let mut context = LoweringContext {
        chunk,
        slots,
        folds: FoldCursor::new(&plan),
    };
    lower_into(expression, result, 1, &mut context);
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

struct LoweringContext<'a, 'b> {
    chunk: &'a mut ExprChunk,
    slots: &'a mut SlotTable,
    folds: FoldCursor<'b>,
}

fn lower_into(expression: &Expr, dst: u16, column: usize, context: &mut LoweringContext<'_, '_>) {
    if let Expr::Spanned { span, expression } = expression {
        lower_into(expression, dst, span.column, context);
        return;
    }
    if let Some(value) = context.folds.enter() {
        let constant = context.chunk.constant(value);
        context.chunk.push(ExprOp::Const { dst, constant });
        return;
    }

    match expression {
        Expr::Spanned { .. } => unreachable!("spans are removed above"),
        Expr::Value(value) => {
            let constant = context.chunk.constant(value.clone());
            context.chunk.push(ExprOp::Const { dst, constant });
        }
        Expr::Variable(name) => {
            let slot = context.slots.intern(name);
            context.chunk.push(ExprOp::Load {
                dst,
                slot,
                column: crate::compact_source_position(column),
            });
        }
        Expr::Unary { op, value } => {
            lower_into(value, dst, column, context);
            context.chunk.push(ExprOp::Unary {
                dst,
                op: *op,
                source: dst,
            });
        }
        Expr::Binary { left, op, right } if matches!(op, BinaryOp::And | BinaryOp::Or) => {
            lower_short_circuit(left, *op, right, dst, column, context);
        }
        Expr::Binary { left, op, right } => {
            lower_into(left, dst, column, context);
            let right_register = context.chunk.register();
            lower_into(right, right_register, column, context);
            context.chunk.push(ExprOp::Binary {
                dst,
                left: dst,
                op: *op,
                right: right_register,
            });
        }
        Expr::Invoke {
            function,
            arguments,
        } => {
            let argument_registers = context.chunk.register_range(arguments.len());
            for (argument, register) in arguments.iter().zip(argument_registers.clone()) {
                lower_into(argument, register, column, context);
            }
            match function {
                Builtin::Random => {
                    let state_slot = context.slots.intern_rng_state();
                    context.chunk.push(ExprOp::Random {
                        dst,
                        args: argument_registers,
                        state_slot,
                    });
                }
                Builtin::Chance => {
                    let state_slot = context.slots.intern_rng_state();
                    context.chunk.push(ExprOp::Chance {
                        dst,
                        args: argument_registers,
                        state_slot,
                    });
                }
                _ => {
                    context.chunk.push(ExprOp::Call {
                        dst,
                        function: *function,
                        args: argument_registers,
                    });
                }
            }
        }
        Expr::Interpolate { parts } => lower_interpolation(parts, dst, column, context),
    }
}

fn lower_interpolation(
    parts: &[StrPart],
    dst: u16,
    column: usize,
    context: &mut LoweringContext<'_, '_>,
) {
    let value_registers = context.chunk.register_range(parts.len());
    for (part, register) in parts.iter().zip(value_registers.clone()) {
        match part {
            StrPart::Literal(text) => {
                let constant = context.chunk.constant(Value::String(text.clone().into()));
                context.chunk.push(ExprOp::Const {
                    dst: register,
                    constant,
                });
            }
            StrPart::Hole(expr) => lower_into(expr, register, column, context),
        }
    }
    context.chunk.push(ExprOp::Concat {
        dst,
        values: value_registers,
    });
}

/// Lowers `left and right` / `left or right` with short-circuit semantics.
///
/// Register layout:
/// ```text
///   <left -> dst>
///   JumpIfFalse dst, END
///   <right -> rhs>
///   Binary dst, dst, And, rhs
/// END:
/// ```
/// For `or` the guard is `JumpIfTrue`. When the jump is taken the left operand
/// is the whole result. Keeping the left operand on the fallthrough path lets
/// `Binary` reproduce the tree-walker's evaluation order and exact type error.
fn lower_short_circuit(
    left: &Expr,
    op: BinaryOp,
    right: &Expr,
    dst: u16,
    column: usize,
    context: &mut LoweringContext<'_, '_>,
) {
    lower_into(left, dst, column, context);
    let guard = context.chunk.push(match op {
        BinaryOp::And => ExprOp::JumpIfFalse {
            condition: dst,
            target: u32::MAX,
        },
        BinaryOp::Or => ExprOp::JumpIfTrue {
            condition: dst,
            target: u32::MAX,
        },
        _ => unreachable!("only and/or reach short-circuit lowering"),
    });
    let right_register = context.chunk.register();
    lower_into(right, right_register, column, context);
    context.chunk.push(ExprOp::Binary {
        dst,
        left: dst,
        op,
        right: right_register,
    });
    let end = u32::try_from(context.chunk.ops.len()).expect("op index fits in u32");
    context.chunk.ops[guard] = match op {
        BinaryOp::And => ExprOp::JumpIfFalse {
            condition: dst,
            target: end,
        },
        BinaryOp::Or => ExprOp::JumpIfTrue {
            condition: dst,
            target: end,
        },
        _ => unreachable!(),
    };
}

#[cfg(test)]
#[path = "expr_tests.rs"]
mod tests;
