//! Optional source locations kept beside an execution image.

use crate::{ExprOp, Op, Program};

/// A source location attached to a bytecode item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DebugLocation {
    /// One-based source line, or zero when the item has no source location.
    pub line: u32,
    /// One-based source column, or zero when the item has no column.
    pub column: u32,
}

/// Source locations that can be dropped from a numeric-slot execution image.
///
/// The table uses packed parallel arrays so it can be retained by tooling
/// without widening the hot instruction representation. Expression columns
/// are indexed by the packed `Program::expr_ops` arena; chunk and operation
/// locations use their respective program IDs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebugTable {
    chunk_locations: Box<[DebugLocation]>,
    op_locations: Box<[DebugLocation]>,
    expr_columns: Box<[u32]>,
}

impl DebugTable {
    pub(crate) fn from_program(program: &Program) -> Self {
        let chunk_locations = program
            .chunks
            .iter()
            .map(|chunk| DebugLocation {
                line: chunk.line,
                column: 0,
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();
        let op_locations = program
            .ops
            .iter()
            .map(|op| op_location(program, op))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        let expr_columns = program
            .expr_ops
            .iter()
            .map(|op| match op {
                ExprOp::Load { column, .. } => *column,
                _ => 0,
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Self {
            chunk_locations,
            op_locations,
            expr_columns,
        }
    }

    /// Returns the source location for an expression chunk.
    #[must_use]
    pub fn chunk(&self, id: u32) -> Option<DebugLocation> {
        self.chunk_locations.get(id as usize).copied()
    }

    /// Returns the source location for a program operation.
    #[must_use]
    pub fn op(&self, pc: usize) -> Option<DebugLocation> {
        self.op_locations.get(pc).copied()
    }

    /// Returns the source column for a packed expression operation.
    #[must_use]
    pub fn expression_column(&self, arena_index: usize) -> Option<u32> {
        self.expr_columns
            .get(arena_index)
            .copied()
            .filter(|column| *column != 0)
    }

    /// Returns the number of source locations in this table.
    #[must_use]
    pub fn len(&self) -> usize {
        self.chunk_locations.len() + self.op_locations.len() + self.expr_columns.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

fn op_location(program: &Program, op: &Op) -> DebugLocation {
    match op {
        Op::Set { value, .. }
        | Op::JumpIfFalse {
            condition: value, ..
        }
        | Op::JumpIfIntegerCompare {
            condition: value, ..
        } => program.chunks.get(*value as usize).map_or(
            DebugLocation { line: 0, column: 0 },
            |chunk| DebugLocation {
                line: chunk.line,
                column: 0,
            },
        ),
        Op::SetConst { line, .. } => DebugLocation {
            line: *line,
            column: 0,
        },
        Op::Host(host) => DebugLocation {
            line: host.line,
            column: 0,
        },
        Op::CopySlot { line, column, .. } | Op::Update { line, column, .. } => DebugLocation {
            line: *line,
            column: *column,
        },
        Op::Jump(_) | Op::Halt => DebugLocation { line: 0, column: 0 },
    }
}
