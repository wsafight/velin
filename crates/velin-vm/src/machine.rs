//! The program executor: control flow, frames, and host-effect yielding.
//!
//! [`Machine`] runs a [`Program`] until it either finishes or reaches a host
//! effect. Host effects are the sole boundary to the outside world: on
//! [`Op::Host`] the machine evaluates the effect's arguments and returns
//! [`Yield::Host`] to the embedder, which performs the effect and calls
//! [`Machine::resume`] to continue. A loop that never yields is bounded by
//! [`MAX_IMMEDIATE_STEPS`] so a runaway script cannot hang the host.

use crate::chunk::eval_validated_chunk;
use std::sync::Arc;
use velin_compile::{Op, Program, ProgramValidationError};
use velin_eval::EvalError;
use velin_syntax::Value;

/// The maximum number of control-flow ops executed between two yields.
///
/// Mirrors the original runtime's guard against infinite loops. Reaching it is
/// reported as an error rather than hanging.
pub const MAX_IMMEDIATE_STEPS: usize = 10_000;

/// Seed used by [`Machine::new`] when a program contains random expressions.
pub const DEFAULT_RNG_SEED: i64 = 0;

/// Why the machine stopped running.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Yield {
    /// A host effect must be performed. `values` are the evaluated arguments.
    /// The embedder handles `host_id` and calls [`Machine::resume`].
    Host { host_id: u32, values: Vec<Value> },
    /// The program halted normally.
    Finished,
}

/// A running program instance. Cloning it creates an in-memory checkpoint: the
/// program counter, variables, pending host request, and RNG slot are all
/// copied, so restoring the clone also rewinds future random draws.
#[derive(Debug, Clone)]
pub struct Machine {
    program: Arc<Program>,
    frame: Vec<Option<Value>>,
    pc: usize,
    /// The host effect execution is currently waiting to resume from.
    pending_host: Option<PendingHost>,
    finished: bool,
}

#[derive(Debug, Clone, Copy)]
struct PendingHost {
    bind: Option<u32>,
    line: usize,
}

impl Machine {
    /// Creates a machine for a validated `program` with an empty variable frame.
    ///
    /// # Errors
    /// Returns an error when serialized or manually assembled bytecode is
    /// malformed or exceeds a program budget.
    pub fn new(program: impl Into<Arc<Program>>) -> Result<Self, ProgramValidationError> {
        Self::with_seed(program, DEFAULT_RNG_SEED)
    }

    /// Creates a machine and initializes its threaded RNG slot with `seed`.
    ///
    /// # Errors
    /// Returns an error when `program` fails structural validation.
    pub fn with_seed(
        program: impl Into<Arc<Program>>,
        seed: i64,
    ) -> Result<Self, ProgramValidationError> {
        let program = program.into();
        program.validate()?;
        let width = program.slots.len();
        let mut frame = vec![None; width];
        if let Some(slot) = program.slots.rng_state() {
            frame[slot as usize] = Some(Value::Integer(seed));
        }
        Ok(Self {
            program,
            frame,
            pc: 0,
            pending_host: None,
            finished: false,
        })
    }

    /// Returns the immutable bytecode shared by this machine and its snapshots.
    #[must_use]
    pub fn program(&self) -> &Program {
        &self.program
    }

    /// Re-seeds the RNG state. Returns `false` when the program has no random
    /// expression and therefore no RNG slot.
    pub fn set_rng_seed(&mut self, seed: i64) -> bool {
        let Some(slot) = self.program.slots.rng_state() else {
            return false;
        };
        self.frame[slot as usize] = Some(Value::Integer(seed));
        true
    }

    /// Returns the current RNG state when the program contains random ops.
    #[must_use]
    pub fn rng_state(&self) -> Option<i64> {
        let slot = self.program.slots.rng_state()?;
        match self.frame[slot as usize].as_ref()? {
            Value::Integer(state) => Some(*state),
            _ => None,
        }
    }

    /// Presets a slot's value by name (e.g. for `default`-style initial state).
    ///
    /// Returns `false` if the program never referenced that name or `value`
    /// exceeds the deterministic data budget.
    pub fn set_variable(&mut self, name: &str, value: Value) -> bool {
        if value.validate_data().is_err() {
            return false;
        }
        match self.program.slots.get(name) {
            Some(slot) => {
                self.frame[slot as usize] = Some(value);
                true
            }
            None => false,
        }
    }

    /// Reads a slot's current value by name.
    #[must_use]
    pub fn variable(&self, name: &str) -> Option<&Value> {
        let slot = self.program.slots.get(name)?;
        self.frame[slot as usize].as_ref()
    }

    /// Runs until the program yields a host effect or finishes.
    ///
    /// # Errors
    /// Returns [`EvalError`] from any expression evaluation, or a synthetic
    /// error if [`MAX_IMMEDIATE_STEPS`] is exceeded.
    pub fn run(&mut self) -> Result<Yield, EvalError> {
        if let Some(pending) = self.pending_host {
            return Err(EvalError::new(
                pending.line,
                "machine is waiting for the host; call `resume`",
            ));
        }
        let mut steps = 0;
        while !self.finished {
            steps += 1;
            if steps > MAX_IMMEDIATE_STEPS {
                return Err(EvalError::new(
                    self.current_line(),
                    "possible infinite loop: too many steps without yielding",
                ));
            }
            if self.pc >= self.program.ops.len() {
                self.finished = true;
                break;
            }
            if let Some(effect) = self.step()? {
                return Ok(effect);
            }
        }
        Ok(Yield::Finished)
    }

    /// Resumes after a [`Yield::Host`]. A host op with a `bind` destination
    /// requires `Some(value)`; side-effect-only host ops ignore a supplied value.
    ///
    /// # Errors
    /// Propagates evaluation errors from continued execution.
    pub fn resume(&mut self, value: Option<Value>) -> Result<Yield, EvalError> {
        let pending = self.pending_host.ok_or_else(|| {
            EvalError::new(
                self.current_line(),
                "cannot resume: no host effect is pending",
            )
        })?;
        if let Some(slot) = pending.bind {
            let value = value.ok_or_else(|| {
                EvalError::new(pending.line, "bound host effect returned no value")
            })?;
            value
                .validate_data()
                .map_err(|error| EvalError::new(pending.line, error))?;
            self.frame[slot as usize] = Some(value);
        }
        self.pending_host = None;
        self.run()
    }

    /// Executes one op. Returns `Some(Yield::Host)` if it yielded, `None`
    /// otherwise. Advances `pc` accordingly.
    fn step(&mut self) -> Result<Option<Yield>, EvalError> {
        match self.program.ops[self.pc].clone() {
            Op::Set { slot, value } => {
                let result = self.eval(value)?;
                self.frame[slot as usize] = Some(result);
                self.pc += 1;
            }
            Op::Jump(target) => self.pc = target as usize,
            Op::JumpIfFalse { condition, target } => match self.eval(condition)? {
                Value::Boolean(false) => self.pc = target as usize,
                Value::Boolean(true) => self.pc += 1,
                value => {
                    return Err(EvalError::new(
                        self.program.chunks[condition as usize].line,
                        format!("condition expects boolean, found {}", value.type_name()),
                    ));
                }
            },
            Op::Host {
                host_id,
                args,
                bind,
                line,
            } => {
                let values = args
                    .iter()
                    .map(|chunk| self.eval(*chunk))
                    .collect::<Result<Vec<_>, _>>()?;
                self.pc += 1; // resume past the effect, never re-run it
                self.pending_host = Some(PendingHost { bind, line });
                return Ok(Some(Yield::Host { host_id, values }));
            }
            Op::Halt => self.finished = true,
        }
        Ok(None)
    }

    fn eval(&mut self, chunk_id: u32) -> Result<Value, EvalError> {
        let chunk = &self.program.chunks[chunk_id as usize];
        let slots = &self.program.slots;
        eval_validated_chunk(chunk, &mut self.frame, |slot| {
            slots.name(slot).unwrap_or("?").to_owned()
        })
    }

    fn current_line(&self) -> usize {
        self.program
            .ops
            .get(self.pc)
            .and_then(|op| match op {
                Op::Set { value, .. }
                | Op::JumpIfFalse {
                    condition: value, ..
                } => self.program.chunks.get(*value as usize).map(|c| c.line),
                Op::Host { line, .. } => Some(*line),
                _ => None,
            })
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use velin_compile::ProgramBuilder;
    use velin_syntax::{BinaryOp, Expr};

    #[test]
    fn runs_assignments_and_conditionals_to_completion() {
        // hp = 30; if hp < 20 { hp = 0 }  -> hp stays 30
        let mut b = ProgramBuilder::new();
        let hp = b.slot("hp");
        let thirty = b.expr(&Expr::Value(Value::Integer(30)), 1);
        b.push(Op::Set {
            slot: hp,
            value: thirty,
        });
        let cond = b.expr(
            &Expr::Binary {
                left: Box::new(Expr::Variable("hp".into())),
                op: BinaryOp::Less,
                right: Box::new(Expr::Value(Value::Integer(20))),
            },
            2,
        );
        let jump = b.push(Op::JumpIfFalse {
            condition: cond,
            target: u32::MAX,
        });
        let zero = b.expr(&Expr::Value(Value::Integer(0)), 3);
        b.push(Op::Set {
            slot: hp,
            value: zero,
        });
        let end = b.here();
        b.patch(
            jump,
            Op::JumpIfFalse {
                condition: cond,
                target: end,
            },
        );
        let mut machine = Machine::new(b.build()).unwrap();
        assert_eq!(machine.run().unwrap(), Yield::Finished);
        assert_eq!(machine.variable("hp"), Some(&Value::Integer(30)));
    }

    #[test]
    fn yields_host_effects_and_binds_resume_values() {
        // ask(host 1); set choice from resume; done
        let mut b = ProgramBuilder::new();
        let choice = b.slot("choice");
        let prompt = b.expr(&Expr::Value(Value::String("pick".into())), 1);
        b.push(Op::Host {
            host_id: 1,
            args: vec![prompt],
            bind: Some(choice),
            line: 1,
        });
        let mut machine = Machine::new(b.build()).unwrap();
        let effect = machine.run().unwrap();
        assert_eq!(
            effect,
            Yield::Host {
                host_id: 1,
                values: vec![Value::String("pick".into())]
            }
        );
        assert_eq!(
            machine.resume(Some(Value::Integer(2))).unwrap(),
            Yield::Finished
        );
        assert_eq!(machine.variable("choice"), Some(&Value::Integer(2)));
    }

    #[test]
    fn infinite_loops_are_bounded() {
        // loop: jump loop
        let mut b = ProgramBuilder::new();
        b.push(Op::Jump(0));
        let mut machine = Machine::new(b.build()).unwrap();
        let error = machine.run().unwrap_err();
        assert!(error.message.contains("infinite loop"));
    }

    #[test]
    fn cloning_a_machine_rolls_back_rng_with_the_frame() {
        // Draw and yield once, clone the yielded machine as a checkpoint, then
        // draw again. Resuming the checkpoint must reproduce the second draw.
        let mut b = ProgramBuilder::new();
        let first = b.slot("first");
        let second = b.slot("second");
        let first_roll = velin_parse::parse_expression("random(1, 1000)", "t", 1, 1).unwrap();
        let first_chunk = b.expr(&first_roll, 1);
        b.push(Op::Set {
            slot: first,
            value: first_chunk,
        });
        let show_first = b.expr(&Expr::Variable("first".into()), 2);
        b.push(Op::Host {
            host_id: 1,
            args: vec![show_first],
            bind: None,
            line: 2,
        });
        let second_roll = velin_parse::parse_expression("random(1, 1000)", "t", 3, 1).unwrap();
        let second_chunk = b.expr(&second_roll, 3);
        b.push(Op::Set {
            slot: second,
            value: second_chunk,
        });
        let show_second = b.expr(&Expr::Variable("second".into()), 4);
        b.push(Op::Host {
            host_id: 2,
            args: vec![show_second],
            bind: None,
            line: 4,
        });

        let mut machine = Machine::with_seed(b.build(), 42).unwrap();
        assert!(matches!(
            machine.run().unwrap(),
            Yield::Host { host_id: 1, .. }
        ));
        let checkpoint = machine.clone();

        let expected = machine.resume(None).unwrap();
        let expected_state = machine.rng_state();
        let mut restored = checkpoint;
        let replayed = restored.resume(None).unwrap();
        assert_eq!(replayed, expected);
        assert_eq!(restored.rng_state(), expected_state);
        assert_eq!(restored.variable("second"), machine.variable("second"));
    }

    #[test]
    fn the_same_seed_replays_and_different_seeds_diverge() {
        let mut b = ProgramBuilder::new();
        let roll = b.slot("roll");
        let expression = velin_parse::parse_expression("random(0, 1000000)", "t", 1, 1).unwrap();
        let chunk = b.expr(&expression, 1);
        b.push(Op::Set {
            slot: roll,
            value: chunk,
        });
        let program = b.build();

        let mut first = Machine::with_seed(program.clone(), 5).unwrap();
        let mut replay = Machine::with_seed(program.clone(), 5).unwrap();
        let mut different = Machine::with_seed(program, 6).unwrap();
        first.run().unwrap();
        replay.run().unwrap();
        different.run().unwrap();
        assert_eq!(first.variable("roll"), replay.variable("roll"));
        assert_eq!(first.rng_state(), replay.rng_state());
        assert_ne!(first.rng_state(), different.rng_state());
    }

    #[test]
    fn non_boolean_conditions_are_rejected() {
        let mut b = ProgramBuilder::new();
        let condition = b.expr(&Expr::Value(Value::Integer(1)), 4);
        b.push(Op::JumpIfFalse {
            condition,
            target: 1,
        });
        let error = Machine::new(b.build()).unwrap().run().unwrap_err();
        assert_eq!(error.line, 4);
        assert!(error.message.contains("condition expects boolean"));
    }

    #[test]
    fn bound_host_effects_require_a_resume_value() {
        let mut b = ProgramBuilder::new();
        let choice = b.slot("choice");
        b.push(Op::Host {
            host_id: 1,
            args: Vec::new(),
            bind: Some(choice),
            line: 7,
        });
        let mut machine = Machine::new(b.build()).unwrap();
        machine.run().unwrap();

        let error = machine.resume(None).unwrap_err();
        assert_eq!(error.line, 7);
        assert!(error.message.contains("returned no value"));
        assert!(machine.variable("choice").is_none());

        let oversized = Value::String("x".repeat(velin_syntax::MAX_DATA_TEXT_BYTES + 1));
        let error = machine.resume(Some(oversized)).unwrap_err();
        assert_eq!(error.line, 7);
        assert!(error.message.contains("data text exceeds"));
        assert!(machine.variable("choice").is_none());

        assert_eq!(
            machine.resume(Some(Value::Integer(2))).unwrap(),
            Yield::Finished
        );
        assert_eq!(machine.variable("choice"), Some(&Value::Integer(2)));
    }

    #[test]
    fn run_cannot_skip_a_pending_host_effect() {
        let mut b = ProgramBuilder::new();
        b.push(Op::Host {
            host_id: 1,
            args: Vec::new(),
            bind: None,
            line: 3,
        });
        let mut machine = Machine::new(b.build()).unwrap();
        machine.run().unwrap();
        assert!(machine.run().unwrap_err().message.contains("call `resume`"));
        assert_eq!(machine.resume(None).unwrap(), Yield::Finished);
        assert!(
            machine
                .resume(None)
                .unwrap_err()
                .message
                .contains("no host effect")
        );
    }

    #[test]
    fn external_values_must_fit_the_data_budget() {
        let mut b = ProgramBuilder::new();
        b.slot("seed");
        let mut machine = Machine::new(b.build()).unwrap();
        let oversized = Value::String("x".repeat(velin_syntax::MAX_DATA_TEXT_BYTES + 1));
        assert!(!machine.set_variable("seed", oversized));
        assert!(machine.variable("seed").is_none());
    }

    #[test]
    fn rng_seed_and_unknown_variables_and_jump_off_end() {
        let mut plain = ProgramBuilder::new();
        plain.push(Op::Halt);
        let mut machine = Machine::new(plain.build()).unwrap();
        assert!(!machine.set_rng_seed(1));
        assert!(machine.rng_state().is_none());
        assert!(!machine.set_variable("missing", Value::Integer(1)));

        let mut random = ProgramBuilder::new();
        let roll = random.slot("roll");
        let expression = velin_parse::parse_expression("random(0, 10)", "t", 1, 1).unwrap();
        let chunk = random.expr(&expression, 1);
        random.push(Op::Set {
            slot: roll,
            value: chunk,
        });
        let mut machine = Machine::with_seed(random.build(), 1).unwrap();
        assert!(machine.set_rng_seed(99));
        assert_eq!(machine.rng_state(), Some(99));
        machine.set_variable(velin_compile::RNG_STATE_SLOT, Value::Boolean(true));
        assert!(machine.rng_state().is_none());

        let mut jump = ProgramBuilder::new();
        jump.push(Op::Jump(8));
        let error = Machine::new(jump.build()).unwrap_err();
        assert!(error.message.contains("past program end"));
    }
}
