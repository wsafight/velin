//! Deterministic bytecode virtual machine for Velin.
//!
//! [`Machine`] executes a [`velin_compile::Program`], running control flow over
//! a slot-addressed variable frame and evaluating expressions via the flat
//! stack machine in [`eval_chunk`]. All value semantics are delegated to
//! `velin-eval`, so the bytecode path and the tree-walker agree exactly (see
//! the differential tests in `tests/`).
//!
//! The machine is safe Rust with no JIT and no host-domain concepts. The only
//! boundary to the outside world is [`Yield::Host`]: the machine hands an
//! opaque effect to the embedder and continues when the embedder calls
//! [`Machine::resume`].

mod chunk;
mod machine;

pub use chunk::{Frame, eval_chunk};
pub use machine::{DEFAULT_RNG_SEED, MAX_IMMEDIATE_STEPS, Machine, Yield};
