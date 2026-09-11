//! Resource limits shared by source parsing and lowering.

/// Maximum UTF-8 size of one Velin source file.
pub const MAX_SOURCE_BYTES: usize = 1024 * 1024;
/// Maximum number of physical lines in one Velin source file.
pub const MAX_SOURCE_LINES: usize = 10_000;
/// Maximum nesting depth of statement blocks.
pub const MAX_STATEMENT_DEPTH: usize = 64;
