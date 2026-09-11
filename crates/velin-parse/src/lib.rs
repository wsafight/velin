//! Expression parsing for Velin.
//!
//! Turns a bounded source string into a [`velin_syntax::Expr`]. The grammar and
//! limits are host-agnostic and shared by every Velin frontend.

mod expression;

pub use expression::{
    MAX_EXPRESSION_BYTES, MAX_EXPRESSION_NESTING, MAX_EXPRESSION_TOKENS,
    MAX_EXPRESSION_TOTAL_TOKENS, MAX_EXPRESSION_WORK_BYTES, MAX_INTERPOLATION_DEPTH,
    parse_expression,
};
