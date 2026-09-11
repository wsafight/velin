//! Program-level bytecode: control flow over expression chunks.
//!
//! A [`Program`] is the unit the VM executes. It is a flat list of [`Op`]s
//! indexed by a program counter, plus the pool of [`ExprChunk`]s those ops
//! evaluate and the [`SlotTable`] describing the variable frame.
//!
//! This layer is deliberately generic. The only way to reach the outside world
//! is [`Op::Host`], an opaque effect the VM yields to the embedder.

use crate::bytecode::ExprChunk;
use crate::expr::compile_expression;
use crate::slots::SlotTable;
use serde::{Deserialize, Serialize};
use velin_syntax::Expr;

/// An index into [`Program::chunks`].
pub type ChunkId = u32;
/// A program-counter target: an index into [`Program::ops`].
pub type Pc = u32;

/// A control-flow instruction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Op {
    /// Evaluate a chunk and store the result into a frame slot.
    Set { slot: u32, value: ChunkId },
    /// Unconditional jump to a program counter.
    Jump(Pc),
    /// Evaluate a chunk; if it is boolean `false`, jump to the target.
    JumpIfFalse { condition: ChunkId, target: Pc },
    /// Yield an opaque host effect: evaluate `args` and hand `(host_id,
    /// values)` back to the embedder. Execution resumes at the next op when the
    /// host calls `resume`; a `bind` destination makes its return value required.
    Host {
        host_id: u32,
        args: Vec<ChunkId>,
        /// Destination for a required host return value, or `None` for a
        /// side-effect-only command.
        bind: Option<u32>,
        /// Source line used when a bound command resumes without a value.
        line: usize,
    },
    /// Halt execution successfully.
    Halt,
}

/// A compiled, executable program.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Program {
    pub ops: Vec<Op>,
    pub chunks: Vec<ExprChunk>,
    pub slots: SlotTable,
}

// `SlotTable` is compile-time-only state, but programs are serialized for
// tooling/inspection, so it derives the same traits via a thin manual impl.
impl Serialize for SlotTable {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.names().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for SlotTable {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let names = Vec::<String>::deserialize(deserializer)?;
        let mut table = SlotTable::new();
        for name in names {
            if table.get(&name).is_some() {
                return Err(serde::de::Error::custom(format!(
                    "duplicate slot name `{name}`"
                )));
            }
            table.intern(&name);
        }
        Ok(table)
    }
}

impl<'de> Deserialize<'de> for Program {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct ProgramWire {
            ops: Vec<Op>,
            chunks: Vec<ExprChunk>,
            slots: SlotTable,
        }

        let wire = ProgramWire::deserialize(deserializer)?;
        let program = Self {
            ops: wire.ops,
            chunks: wire.chunks,
            slots: wire.slots,
        };
        program.validate().map_err(serde::de::Error::custom)?;
        Ok(program)
    }
}

/// Incrementally assembles a [`Program`].
///
/// The builder owns one [`SlotTable`] shared across every expression, so a
/// variable has the same slot everywhere in the program. Expressions are
/// compiled on demand and pooled; `Op`s refer to them by [`ChunkId`].
#[derive(Debug, Default)]
pub struct ProgramBuilder {
    ops: Vec<Op>,
    chunks: Vec<ExprChunk>,
    slots: SlotTable,
}

impl ProgramBuilder {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Interns a variable name, returning its frame slot.
    pub fn slot(&mut self, name: &str) -> u32 {
        self.slots.intern(name)
    }

    /// Compiles an expression and returns its pooled [`ChunkId`].
    ///
    /// # Panics
    /// Panics only if more than `u32::MAX` chunks are pooled, which the data
    /// budget makes unreachable.
    pub fn expr(&mut self, expression: &Expr, line: usize) -> ChunkId {
        let chunk = compile_expression(expression, &mut self.slots, line);
        let id = u32::try_from(self.chunks.len()).expect("chunk id fits in u32");
        self.chunks.push(chunk);
        id
    }

    /// Appends an op, returning its [`Pc`] (useful for patching jumps).
    ///
    /// # Panics
    /// Panics only if more than `u32::MAX` ops are pushed, which the data
    /// budget makes unreachable.
    pub fn push(&mut self, op: Op) -> Pc {
        let pc = u32::try_from(self.ops.len()).expect("pc fits in u32");
        self.ops.push(op);
        pc
    }

    /// The program counter the next pushed op will occupy.
    ///
    /// # Panics
    /// Panics only if more than `u32::MAX` ops have been pushed, which the data
    /// budget makes unreachable.
    #[must_use]
    pub fn here(&self) -> Pc {
        u32::try_from(self.ops.len()).expect("pc fits in u32")
    }

    /// Overwrites a previously pushed op, used to back-patch jump targets.
    ///
    /// # Panics
    /// Panics if `pc` is out of range.
    pub fn patch(&mut self, pc: Pc, op: Op) {
        self.ops[pc as usize] = op;
    }

    /// Finishes the program, appending a trailing [`Op::Halt`] if the last op
    /// is not already a terminator.
    #[must_use]
    pub fn build(mut self) -> Program {
        if !matches!(self.ops.last(), Some(Op::Halt | Op::Jump(_))) {
            self.ops.push(Op::Halt);
        }
        Program {
            ops: self.ops,
            chunks: self.chunks,
            slots: self.slots,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use velin_syntax::{BinaryOp, Value};

    #[test]
    fn builder_shares_slots_across_expressions() {
        let mut builder = ProgramBuilder::new();
        let hp = builder.slot("hp");
        let e1 = builder.expr(&Expr::Variable("hp".into()), 1);
        let e2 = builder.expr(
            &Expr::Binary {
                left: Box::new(Expr::Variable("hp".into())),
                op: BinaryOp::Subtract,
                right: Box::new(Expr::Value(Value::Integer(1))),
            },
            2,
        );
        assert_ne!(e1, e2);
        let program = builder.build();
        assert_eq!(program.slots.get("hp"), Some(hp));
        assert!(matches!(program.ops.last(), Some(Op::Halt)));
    }

    #[test]
    fn programs_round_trip_through_json() {
        let mut builder = ProgramBuilder::new();
        let slot = builder.slot("x");
        let value = builder.expr(&Expr::Value(Value::Integer(7)), 1);
        builder.push(Op::Set { slot, value });
        let program = builder.build();
        let json = serde_json::to_string(&program).unwrap();
        let restored: Program = serde_json::from_str(&json).unwrap();
        assert_eq!(program, restored);
    }

    #[test]
    fn random_programs_round_trip_with_their_state_slot() {
        let mut builder = ProgramBuilder::new();
        let roll = builder.slot("roll");
        let expression = velin_parse::parse_expression("random(1, 6)", "test", 1, 1).unwrap();
        let value = builder.expr(&expression, 1);
        builder.push(Op::Set { slot: roll, value });
        let program = builder.build();
        let rng_slot = program.slots.rng_state().expect("RNG slot serialized");
        let json = serde_json::to_string(&program).unwrap();
        let restored: Program = serde_json::from_str(&json).unwrap();
        assert_eq!(program, restored);
        assert_eq!(restored.slots.rng_state(), Some(rng_slot));
        assert!(matches!(
            restored.chunks[0].ops.last(),
            Some(crate::ExprOp::Random { state_slot }) if *state_slot == rng_slot
        ));
    }

    #[test]
    fn deserialization_rejects_malformed_programs_and_duplicate_slots() {
        let missing_chunk = r#"{
            "ops":[{"Set":{"slot":0,"value":9}}],
            "chunks":[],
            "slots":["x"]
        }"#;
        let error = serde_json::from_str::<Program>(missing_chunk).unwrap_err();
        assert!(error.to_string().contains("missing expression chunk"));

        let duplicate_slots = r#"{
            "ops":["Halt"],
            "chunks":[],
            "slots":["x","x"]
        }"#;
        let error = serde_json::from_str::<Program>(duplicate_slots).unwrap_err();
        assert!(error.to_string().contains("duplicate slot name"));
    }
}
