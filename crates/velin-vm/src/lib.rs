//! Deterministic bytecode virtual machine for Velin.
//!
//! [`Machine`] executes a [`velin_bytecode::Program`], running control flow over
//! a slot-addressed variable frame and evaluating expressions via the flat
//! register interpreter in [`eval_chunk`]. All value semantics are delegated to
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
pub use machine::{
    DEFAULT_RNG_SEED, ExecutionProfile, FastYield, HostEffect, MAX_HOST_PAYLOAD_TEXT_BYTES,
    MAX_HOST_PAYLOAD_VALUES, MAX_IMMEDIATE_STEPS, MAX_MACHINE_DATA_VALUES, MAX_MACHINE_TEXT_BYTES,
    Machine, MachineInvoker, SetVariableError, Yield,
};

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
