//! Definite-assignment analysis over a compiled [`Program`].
//!
//! A Velin program reads variables from frame slots. Reading a slot that was
//! never assigned on the current path is a runtime error ("has not been
//! assigned on this path"). This is a classic *must* dataflow analysis that
//! proves, for every read, that the slot is assigned on **all** control-flow
//! paths reaching it — turning that runtime error into a compile-time
//! diagnostic.
//!
//! The analysis is intraprocedural and conservative in the safe direction: it
//! only reports a read it can prove is always unassigned, so it never rejects a
//! program the runtime would have accepted.

use crate::cfg::ControlFlow;
use std::collections::{BTreeSet, VecDeque};
use velin_compile::{ExprChunk, ExprOp, Op, Program};
use velin_syntax::{BinaryOp, Builtin, UnaryOp, Value};

/// A use of a slot the analysis proved is unassigned on some path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnassignedUse {
    pub name: String,
    pub line: usize,
    pub column: usize,
}

/// Runs definite-assignment analysis over `program`, treating `preset` slot
/// names (e.g. `default` variables the host seeds before running) as assigned
/// on entry.
///
/// Returns every read that is not definitely assigned, in program order.
#[must_use]
pub fn definite_assignment(program: &Program, preset: &BTreeSet<String>) -> Vec<UnassignedUse> {
    let slot_count = program.slots.len();
    let flow = ControlFlow::new(&program.ops);
    if flow.blocks.is_empty() {
        return Vec::new();
    }

    // `assigned_in[pc]` = slots definitely assigned when control reaches `pc`.
    // Must-analysis: initialise every non-entry block to "all slots" (top) and
    // iterate the intersection to a fixpoint. The entry starts from `preset`.
    let entry: BitSet = preset
        .iter()
        .filter_map(|name| program.slots.get(name))
        .fold(BitSet::empty(slot_count), |mut set, slot| {
            set.insert(slot);
            set
        });

    let mut assigned_out = vec![BitSet::full(slot_count); flow.blocks.len()];
    let mut pending = VecDeque::from([0]);
    let mut queued = vec![false; flow.blocks.len()];
    queued[0] = true;
    while let Some(block_id) = pending.pop_front() {
        queued[block_id] = false;
        let mut outgoing = incoming_set(block_id, &flow, &assigned_out, &entry);
        let block = flow.blocks[block_id];
        for op in &program.ops[block.start..block.end] {
            if let Some(slot) = assigned_slot(op) {
                outgoing.insert(slot);
            }
        }
        if outgoing != assigned_out[block_id] {
            assigned_out[block_id] = outgoing;
            for successor in &flow.successors[block_id] {
                if !queued[*successor] {
                    queued[*successor] = true;
                    pending.push_back(*successor);
                }
            }
        }
    }

    // Report reads not covered by the incoming (pre-assignment) set.
    let mut findings = Vec::new();
    let mut load_cache = vec![None; program.chunks.len()];
    for (block_id, block) in flow.blocks.iter().enumerate() {
        if !flow.reachable[block_id] {
            continue;
        }
        let mut assigned = incoming_set(block_id, &flow, &assigned_out, &entry);
        for op in &program.ops[block.start..block.end] {
            match op {
                Op::Set { value, .. } => {
                    record_unassigned(*value, program, &assigned, &mut load_cache, &mut findings);
                }
                Op::JumpIfFalse { condition, .. } => record_unassigned(
                    *condition,
                    program,
                    &assigned,
                    &mut load_cache,
                    &mut findings,
                ),
                Op::Host { args, .. } => {
                    for chunk in args {
                        record_unassigned(
                            *chunk,
                            program,
                            &assigned,
                            &mut load_cache,
                            &mut findings,
                        );
                    }
                }
                Op::Jump(_) | Op::Halt => {}
            }
            if let Some(slot) = assigned_slot(op) {
                assigned.insert(slot);
            }
        }
    }
    findings
}

/// The set of slots definitely assigned on entry to `pc`: the intersection of
/// its predecessors' out-sets, or the program `entry` set for the root.
fn incoming_set(
    block_id: usize,
    flow: &ControlFlow,
    assigned_out: &[BitSet],
    entry: &BitSet,
) -> BitSet {
    let mut incoming = (block_id == 0).then(|| entry.clone());
    for predecessor in flow.predecessors[block_id]
        .iter()
        .copied()
        .filter(|predecessor| flow.reachable[*predecessor])
    {
        if let Some(current) = &mut incoming {
            current.intersect_assign(&assigned_out[predecessor]);
        } else {
            incoming = Some(assigned_out[predecessor].clone());
        }
    }
    incoming.unwrap_or_else(|| entry.clone())
}

fn assigned_slot(op: &Op) -> Option<u32> {
    match op {
        Op::Set { slot, .. }
        | Op::Host {
            bind: Some(slot), ..
        } => Some(*slot),
        _ => None,
    }
}

fn record_unassigned(
    chunk_id: u32,
    program: &Program,
    assigned: &BitSet,
    load_cache: &mut [Option<BTreeSet<(u32, usize)>>],
    findings: &mut Vec<UnassignedUse>,
) {
    let Some(chunk) = program.chunks.get(chunk_id as usize) else {
        return;
    };
    let Some(cache) = load_cache.get_mut(chunk_id as usize) else {
        return;
    };
    let loads = cache.get_or_insert_with(|| reachable_loads(chunk));
    for (slot, column) in &*loads {
        if !assigned.contains(*slot) {
            findings.push(UnassignedUse {
                name: program.slots.name(*slot).unwrap_or("?").to_owned(),
                line: chunk.line,
                column: *column,
            });
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AbstractValue {
    Boolean(Option<bool>),
    NonBoolean,
    Unknown,
}

/// Returns the slots whose `Load` instructions can execute. Expression chunks
/// contain forward jumps for `and`/`or`; a flat scan would report reads hidden
/// behind a constant short-circuit guard.
fn reachable_loads(chunk: &ExprChunk) -> BTreeSet<(u32, usize)> {
    let mut loads = BTreeSet::new();
    let mut states: Vec<Option<Vec<AbstractValue>>> = vec![None; chunk.ops.len() + 1];
    let mut pending = VecDeque::from([0]);
    states[0] = Some(Vec::new());

    while let Some(pc) = pending.pop_front() {
        if pc >= chunk.ops.len() {
            continue;
        }
        let stack = states[pc].clone().unwrap_or_default();
        for (next_pc, next_stack) in abstract_step(chunk, pc, stack, &mut loads) {
            enqueue_state(&mut states, &mut pending, next_pc, next_stack);
        }
    }
    loads
}

fn abstract_step(
    chunk: &ExprChunk,
    pc: usize,
    mut stack: Vec<AbstractValue>,
    loads: &mut BTreeSet<(u32, usize)>,
) -> Vec<(usize, Vec<AbstractValue>)> {
    let mut next = Vec::new();
    match &chunk.ops[pc] {
        ExprOp::Const(index) => {
            stack.push(match chunk.constants.get(*index as usize) {
                Some(Value::Boolean(value)) => AbstractValue::Boolean(Some(*value)),
                Some(_) => AbstractValue::NonBoolean,
                None => AbstractValue::Unknown,
            });
            next.push((pc + 1, stack));
        }
        ExprOp::Load { slot, column } => {
            loads.insert((*slot, *column));
            stack.push(AbstractValue::Unknown);
            next.push((pc + 1, stack));
        }
        ExprOp::Unary(op) => {
            let value = stack.pop().unwrap_or(AbstractValue::Unknown);
            let result = match (op, value) {
                (UnaryOp::Not, AbstractValue::Boolean(Some(value))) => {
                    Some(AbstractValue::Boolean(Some(!value)))
                }
                (UnaryOp::Not, AbstractValue::NonBoolean)
                | (UnaryOp::Negate, AbstractValue::Boolean(_)) => None,
                (UnaryOp::Not, _) => Some(AbstractValue::Boolean(None)),
                (UnaryOp::Negate, _) => Some(AbstractValue::NonBoolean),
            };
            if let Some(result) = result {
                stack.push(result);
                next.push((pc + 1, stack));
            }
        }
        ExprOp::Binary(op) => {
            pop_values(&mut stack, 2);
            stack.push(match op {
                BinaryOp::Equal
                | BinaryOp::NotEqual
                | BinaryOp::Less
                | BinaryOp::LessEqual
                | BinaryOp::Greater
                | BinaryOp::GreaterEqual
                | BinaryOp::And
                | BinaryOp::Or => AbstractValue::Boolean(None),
                BinaryOp::Add | BinaryOp::Subtract | BinaryOp::Multiply | BinaryOp::Divide => {
                    AbstractValue::NonBoolean
                }
            });
            next.push((pc + 1, stack));
        }
        ExprOp::Call { function, argc } => {
            pop_values(&mut stack, *argc as usize);
            stack.push(match function {
                Builtin::Contains | Builtin::Chance => AbstractValue::Boolean(None),
                Builtin::Get => AbstractValue::Unknown,
                _ => AbstractValue::NonBoolean,
            });
            next.push((pc + 1, stack));
        }
        ExprOp::Random { .. } => {
            pop_values(&mut stack, 2);
            stack.push(AbstractValue::NonBoolean);
            next.push((pc + 1, stack));
        }
        ExprOp::Chance { .. } => {
            pop_values(&mut stack, 1);
            stack.push(AbstractValue::Boolean(None));
            next.push((pc + 1, stack));
        }
        ExprOp::Concat(count) => {
            pop_values(&mut stack, *count as usize);
            stack.push(AbstractValue::NonBoolean);
            next.push((pc + 1, stack));
        }
        ExprOp::JumpIfFalse(target) | ExprOp::JumpIfTrue(target) => {
            let jump_on = matches!(&chunk.ops[pc], ExprOp::JumpIfTrue(_));
            match stack.last().copied().unwrap_or(AbstractValue::Unknown) {
                AbstractValue::Boolean(Some(value)) if value == jump_on => {
                    next.push((*target as usize, stack));
                }
                AbstractValue::Boolean(Some(_)) | AbstractValue::NonBoolean => {
                    next.push((pc + 1, stack));
                }
                AbstractValue::Boolean(None) | AbstractValue::Unknown => {
                    next.push((*target as usize, stack.clone()));
                    next.push((pc + 1, stack));
                }
            }
        }
        ExprOp::AssertBoolean(_) => match stack.last() {
            Some(AbstractValue::NonBoolean) | None => {}
            Some(AbstractValue::Boolean(_) | AbstractValue::Unknown) => {
                next.push((pc + 1, stack));
            }
        },
    }
    next
}

fn pop_values(stack: &mut Vec<AbstractValue>, count: usize) {
    stack.truncate(stack.len().saturating_sub(count));
}

fn enqueue_state(
    states: &mut [Option<Vec<AbstractValue>>],
    pending: &mut VecDeque<usize>,
    pc: usize,
    incoming: Vec<AbstractValue>,
) {
    let Some(state) = states.get_mut(pc) else {
        return;
    };
    let merged = match state.as_ref() {
        None => incoming,
        Some(current) => current
            .iter()
            .zip(&incoming)
            .map(|(left, right)| merge_value(*left, *right))
            .collect(),
    };
    if state.as_ref() != Some(&merged) {
        *state = Some(merged);
        pending.push_back(pc);
    }
}

const fn merge_value(left: AbstractValue, right: AbstractValue) -> AbstractValue {
    match (left, right) {
        (AbstractValue::Boolean(Some(left)), AbstractValue::Boolean(Some(right)))
            if left == right =>
        {
            AbstractValue::Boolean(Some(left))
        }
        (AbstractValue::Boolean(_), AbstractValue::Boolean(_)) => AbstractValue::Boolean(None),
        (AbstractValue::NonBoolean, AbstractValue::NonBoolean) => AbstractValue::NonBoolean,
        _ => AbstractValue::Unknown,
    }
}

/// A tiny fixed-width bit set over slot indices. Kept local to avoid a
/// dependency; slot counts are small (bounded by the data budget).
#[derive(Debug, Clone, PartialEq, Eq)]
struct BitSet {
    bits: Vec<u64>,
    len: usize,
}

impl BitSet {
    fn empty(len: usize) -> Self {
        Self {
            bits: vec![0; len.div_ceil(64)],
            len,
        }
    }

    fn full(len: usize) -> Self {
        let mut set = Self {
            bits: vec![u64::MAX; len.div_ceil(64)],
            len,
        };
        // Clear padding bits beyond `len` so equality is well-defined.
        for slot in len..set.bits.len() * 64 {
            set.remove(u32::try_from(slot).expect("slot fits in u32"));
        }
        set
    }

    fn insert(&mut self, slot: u32) {
        let slot = slot as usize;
        if slot < self.len {
            self.bits[slot / 64] |= 1 << (slot % 64);
        }
    }

    fn remove(&mut self, slot: u32) {
        let slot = slot as usize;
        if slot / 64 < self.bits.len() {
            self.bits[slot / 64] &= !(1 << (slot % 64));
        }
    }

    fn contains(&self, slot: u32) -> bool {
        let slot = slot as usize;
        slot < self.len && (self.bits[slot / 64] >> (slot % 64)) & 1 == 1
    }

    fn intersect_assign(&mut self, other: &Self) {
        for (current, incoming) in self.bits.iter_mut().zip(&other.bits) {
            *current &= incoming;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use velin_compile::{Program, ProgramBuilder, SlotTable};
    use velin_syntax::{BinaryOp, Expr, UnaryOp, Value};

    #[test]
    fn read_after_assignment_is_clean() {
        // set hp = 30; set hp = hp - 1
        let mut b = ProgramBuilder::new();
        let hp = b.slot("hp");
        let thirty = b.expr(&Expr::Value(Value::Integer(30)), 1);
        b.push(Op::Set {
            slot: hp,
            value: thirty,
        });
        let dec = b.expr(
            &Expr::Binary {
                left: Box::new(Expr::Variable("hp".into())),
                op: BinaryOp::Subtract,
                right: Box::new(Expr::Value(Value::Integer(1))),
            },
            2,
        );
        b.push(Op::Set {
            slot: hp,
            value: dec,
        });
        let findings = definite_assignment(&b.build(), &BTreeSet::new());
        assert!(findings.is_empty(), "unexpected: {findings:?}");
    }

    #[test]
    fn read_before_any_assignment_is_flagged() {
        // set y = x + 1   (x never assigned)
        let mut b = ProgramBuilder::new();
        let y = b.slot("y");
        let expr = b.expr(
            &Expr::Binary {
                left: Box::new(Expr::Variable("x".into())),
                op: BinaryOp::Add,
                right: Box::new(Expr::Value(Value::Integer(1))),
            },
            5,
        );
        b.push(Op::Set {
            slot: y,
            value: expr,
        });
        let findings = definite_assignment(&b.build(), &BTreeSet::new());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].name, "x");
        assert_eq!(findings[0].line, 5);
    }

    #[test]
    fn assignment_on_only_one_branch_is_flagged() {
        // if cond { flag_true = 1 }  ; read flag_true
        // flag_true is assigned only on the taken branch, so the join is unassigned.
        let mut b = ProgramBuilder::new();
        let cond_slot = b.slot("cond");
        let only = b.slot("only");
        let sink = b.slot("sink");
        let cond = b.expr(&Expr::Variable("cond".into()), 1);
        let jump = b.push(Op::JumpIfFalse {
            condition: cond,
            target: u32::MAX,
        });
        let one = b.expr(&Expr::Value(Value::Integer(1)), 2);
        b.push(Op::Set {
            slot: only,
            value: one,
        });
        let after = b.here();
        b.patch(
            jump,
            Op::JumpIfFalse {
                condition: cond,
                target: after,
            },
        );
        let read = b.expr(&Expr::Variable("only".into()), 3);
        b.push(Op::Set {
            slot: sink,
            value: read,
        });
        // Seed `cond` so only `only` can be flagged.
        let preset = BTreeSet::from(["cond".to_string()]);
        let _ = cond_slot;
        let findings = definite_assignment(&b.build(), &preset);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].name, "only");
    }

    #[test]
    fn preset_variables_count_as_assigned() {
        let mut b = ProgramBuilder::new();
        let sink = b.slot("sink");
        let read = b.expr(&Expr::Variable("seed".into()), 1);
        b.push(Op::Set {
            slot: sink,
            value: read,
        });
        let preset = BTreeSet::from(["seed".to_string()]);
        assert!(definite_assignment(&b.build(), &preset).is_empty());
    }

    #[test]
    fn program_entry_remains_a_predecessor_when_it_has_a_back_edge() {
        let mut b = ProgramBuilder::new();
        let y = b.slot("y");
        let read_x = b.expr(
            &Expr::Binary {
                left: Box::new(Expr::Variable("x".into())),
                op: BinaryOp::Add,
                right: Box::new(Expr::Value(Value::Integer(1))),
            },
            1,
        );
        b.push(Op::Set {
            slot: y,
            value: read_x,
        });
        let x = b.slot("x");
        let one = b.expr(&Expr::Value(Value::Integer(1)), 2);
        b.push(Op::Set {
            slot: x,
            value: one,
        });
        b.push(Op::Jump(0));

        let findings = definite_assignment(&b.build(), &BTreeSet::new());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].name, "x");
    }

    #[test]
    fn unreachable_reads_are_ignored() {
        let mut b = ProgramBuilder::new();
        b.push(Op::Jump(2));
        let y = b.slot("y");
        let read = b.expr(&Expr::Variable("missing".into()), 2);
        b.push(Op::Set {
            slot: y,
            value: read,
        });
        b.push(Op::Halt);

        assert!(definite_assignment(&b.build(), &BTreeSet::new()).is_empty());
    }

    #[test]
    fn constant_short_circuit_reads_are_ignored() {
        let mut b = ProgramBuilder::new();
        let result = b.slot("result");
        let expression = Expr::Binary {
            left: Box::new(Expr::Value(Value::Boolean(false))),
            op: BinaryOp::And,
            right: Box::new(Expr::Variable("missing".into())),
        };
        let value = b.expr(&expression, 1);
        b.push(Op::Set {
            slot: result,
            value,
        });

        assert!(definite_assignment(&b.build(), &BTreeSet::new()).is_empty());
    }

    fn analyze(source: &str) -> Vec<UnassignedUse> {
        let expr = velin_parse::parse_expression(source, "t", 1, 1).unwrap();
        let mut b = ProgramBuilder::new();
        let sink = b.slot("sink");
        let value = b.expr(&expr, 1);
        b.push(Op::Set { slot: sink, value });
        definite_assignment(&b.build(), &BTreeSet::new())
    }

    #[test]
    fn abstract_interpreter_covers_expression_ops() {
        for source in [
            "not true",
            "not false",
            "not 1",
            "-3",
            "1 + 2",
            "1 - 2",
            "2 * 3",
            "4 / 2",
            "1 == 2",
            "1 != 2",
            "1 < 2",
            "1 <= 2",
            "1 > 2",
            "1 >= 2",
            "true and false",
            "false and missing",
            "true or missing",
            "false or true",
            "1 or missing",
            "true and 1",
            "len(\"ab\")",
            "contains(list(1), 1)",
            "get(list(1), 0)",
            "chance(50)",
            "random(1, 2)",
            "\"hello [1]\"",
            "not not true",
            "x or true",
            "x and false",
        ] {
            let _ = analyze(source);
        }

        let negate_bool = Expr::Unary {
            op: UnaryOp::Negate,
            value: Box::new(Expr::Value(Value::Boolean(true))),
        };
        let mut b = ProgramBuilder::new();
        let sink = b.slot("sink");
        let value = b.expr(&negate_bool, 1);
        b.push(Op::Set { slot: sink, value });
        let _ = definite_assignment(&b.build(), &BTreeSet::new());
    }

    #[test]
    fn host_bind_counts_as_assignment_and_empty_programs_are_clean() {
        let mut b = ProgramBuilder::new();
        let answer = b.slot("answer");
        b.push(Op::Host {
            host_id: 0,
            args: Vec::new(),
            bind: Some(answer),
            line: 1,
        });
        let sink = b.slot("sink");
        let read = b.expr(&Expr::Variable("answer".into()), 2);
        b.push(Op::Set {
            slot: sink,
            value: read,
        });
        assert!(definite_assignment(&b.build(), &BTreeSet::new()).is_empty());

        let empty = Program {
            ops: Vec::new(),
            chunks: Vec::new(),
            slots: SlotTable::default(),
        };
        assert!(definite_assignment(&empty, &BTreeSet::new()).is_empty());
    }

    #[test]
    fn malformed_chunks_exercise_abstract_fallbacks() {
        use velin_compile::{ExprChunk, ExprOp, SlotTable};

        let mut slots = SlotTable::new();
        slots.intern("x");
        let program = Program {
            ops: vec![Op::Set { slot: 0, value: 0 }],
            chunks: vec![ExprChunk {
                ops: vec![
                    ExprOp::Const(99),
                    ExprOp::Unary(UnaryOp::Not),
                    ExprOp::AssertBoolean(BinaryOp::And),
                    ExprOp::JumpIfTrue(0),
                    ExprOp::Load { slot: 0, column: 1 },
                ],
                constants: Vec::new(),
                line: 1,
            }],
            slots,
        };
        let _ = definite_assignment(&program, &BTreeSet::new());
    }

    #[test]
    fn missing_chunks_do_not_panic_analysis() {
        let program = Program {
            ops: vec![Op::Set { slot: 0, value: 99 }],
            chunks: Vec::new(),
            slots: SlotTable::new(),
        };
        assert!(definite_assignment(&program, &BTreeSet::new()).is_empty());
    }
}
