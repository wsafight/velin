//! Velin — an embeddable, deterministic scripting language.
//!
//! Velin is a small, host-agnostic scripting core. It has no floating point (so
//! evaluation is reproducible across platforms), no `unsafe`, and no JIT. The
//! only way a script reaches the outside world is by *yielding a host effect*:
//! the [`Machine`] hands an opaque `(host_id, values)` back to the embedder,
//! which performs the effect and resumes execution. That single seam is what
//! lets each host inject its own commands without Velin knowing anything about
//! their meaning.
//!
//! # The pipeline
//!
//! ```text
//! .velin  ──parse──▶ Stmt ──lower──▶ CompiledScript { Program, hosts, labels, defaults }
//! source                                    │
//!                                       Machine::new ──run──▶ Yield::Host ⇄ resume ──▶ Finished
//! ```
//!
//! There are two entry points, from highest-level to lowest:
//!
//! * The **surface language** ([`compile`], [`parse_program`]): an
//!   indentation-sensitive statement syntax (`set`, `if`, `while`, `perform`,
//!   `label`/`jump`) that lowers to a bytecode [`Program`]. This is what a
//!   `.velin` file is written in.
//! * The **expression/bytecode API** ([`ProgramBuilder`], [`Expr`]): build a
//!   program by hand when you want to host Velin's evaluation core without its
//!   statement syntax.
//!
//! This facade re-exports the whole pipeline so most embedders depend only on
//! `velin`:
//!
//! * [`compile`] / [`parse_program`] / [`check_script`] — the surface language.
//! * [`parse_expression`] — source text → [`Expr`].
//! * [`check_expression`] / [`infer`] / [`definite_assignment`] — conservative
//!   static checks that never reject a program the runtime would accept.
//! * [`compile_expression`] / [`ProgramBuilder`] — lower to slot-addressed
//!   bytecode.
//! * [`evaluate`] — the reference tree-walking evaluator.
//! * [`Machine`] — the bytecode VM with host-effect yielding.
//! * [`ScriptRunner`] — checked script instantiation and bounded host driving.
//! * [`PureModule`] — deterministic value-in/value-out module execution.
//!
//! # Example: compile and run a `.velin` script
//!
//! ```
//! use velin::{compile, Machine, Value, Yield};
//!
//! let script = compile(
//!     "heal.velin",
//!     "default hp = 30\n\
//!      perform say(\"hello\")\n\
//!      set hp = hp + 5\n",
//! )
//! .unwrap();
//!
//! let mut machine = Machine::new(script.program.clone()).unwrap();
//! for (name, value) in &script.defaults {
//!     machine.set_variable(name, value.clone());
//! }
//!
//! // The one `perform say(...)` yields a host effect the embedder handles.
//! let Yield::Host { host_id, values } = machine.run().unwrap() else {
//!     panic!("expected a host effect");
//! };
//! assert_eq!(script.host_name(host_id), Some("say"));
//! assert_eq!(values, vec![Value::String("hello".into())]);
//!
//! machine.resume(None).unwrap();
//! assert_eq!(machine.variable("hp"), Some(&Value::Integer(35)));
//! ```
//!
//! # Example: run a program that yields one host effect
//!
//! ```
//! use velin::{Expr, Machine, Op, ProgramBuilder, Value, Yield};
//!
//! // choice = ask("left or right?"); hp = choice
//! let mut b = ProgramBuilder::new();
//! let choice = b.slot("choice");
//! let hp = b.slot("hp");
//! let prompt = b.expr(&Expr::Value(Value::String("left or right?".into())), 1);
//! b.push(Op::host(7, vec![prompt], Some(choice), 1));
//! let echo = b.expr(&Expr::Variable("choice".into()), 2);
//! b.push(Op::Set { slot: hp, value: echo });
//! let program = b.build();
//!
//! let mut machine = Machine::new(program).unwrap();
//! let Yield::Host { host_id, values } = machine.run().unwrap() else {
//!     panic!("expected a host effect");
//! };
//! assert_eq!(host_id, 7);
//! assert_eq!(values, vec![Value::String("left or right?".into())]);
//!
//! // The host performs the effect and resumes with its result.
//! machine.resume(Some(Value::Integer(1))).unwrap();
//! assert_eq!(machine.variable("hp"), Some(&Value::Integer(1)));
//! ```

pub use velin_syntax::{
    BinaryOp, Builtin, Diagnostic, Expr, Severity, SharedString, Span, UnaryOp, Value,
};

pub use velin_parse::parse_expression;

pub use velin_eval::{EvalError, EvalErrorKind, Variables, evaluate, evaluate_with_rng};

pub use velin_bytecode::{
    ChunkId, ExecutionImage, ExprChunk, ExprChunkRef, ExprOp, HostOp, InitialFrame,
    InitialFrameError, InitialValue, Op, Pc, Program, ProgramChunk, ProgramValidationError,
    SlotTable, UpdateOp, ValidatedProgram,
};

pub use velin_compile::{ProgramBuilder, compile_expression};

pub use velin_vm::{
    DEFAULT_MAX_FUEL, DEFAULT_RNG_SEED, ExecutionPolicy, ExecutionProfile, ExecutionProgress,
    FastYield, HostEffect, MAX_HOST_PAYLOAD_TEXT_BYTES, MAX_HOST_PAYLOAD_VALUES,
    MAX_IMMEDIATE_STEPS, MAX_MACHINE_DATA_VALUES, MAX_MACHINE_TEXT_BYTES, Machine, MachineInvoker,
    ProgressCallback, SetVariableError, Yield,
};

pub use runtime::{
    DEFAULT_MAX_HOST_EFFECTS, ExecutionLimits, HostEvent, HostEventQueue, HostEventQueueError,
    HostEventQueueLimits, ScriptRunError, ScriptRunner, ScriptYield,
};

pub use velin_check::{
    Environment, HostSignature, HostSignatures, Type, TypeCheckKind, TypeCheckSite, TypeError,
    UnassignedUse, check_condition, check_expression, check_program_types,
    check_program_types_with_hosts, definite_assignment, infer,
};

pub use velin_lang::{
    ARTIFACT_MAGIC, ARTIFACT_VERSION, ArtifactError, BytecodeArtifact, CompiledScript, Condition,
    HostSchema, LowerError, MAX_ARTIFACT_BYTES, MAX_SOURCE_BYTES, MAX_SOURCE_LINES,
    MAX_STATEMENT_DEPTH, ParseError, RecoveredProgram, Stmt, artifact_cache_key,
    artifact_cache_path, check_script, check_script_with_bindings,
    check_script_with_bindings_and_host_schema, check_script_with_host_schema, compile,
    decode_artifact, encode_artifact, load_artifact_cache, parse_program, parse_program_recovering,
    store_artifact_cache,
};
mod pure;
mod repl;
mod runtime;

pub use pure::{PureModule, PureModuleError, PureModuleInvoker};
pub use repl::{ReplError, ReplSession};
