//! Source-level control for a [`Machine`] using the bytecode debug sidecar.

use super::{ExecutionPolicy, Machine, Yield};
use std::collections::{BTreeMap, BTreeSet};
use velin_bytecode::{DebugLocation, DebugTable, Program, ProgramValidationError};
use velin_eval::EvalError;
use velin_syntax::Value;

/// Why a debugger returned control to its host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebugPauseReason {
    Breakpoint,
    Step,
}

/// A source-level debugger event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DebugEvent {
    Paused {
        reason: DebugPauseReason,
        location: DebugLocation,
    },
    Host {
        host_id: u32,
        values: Vec<Value>,
        location: DebugLocation,
    },
    Finished,
}

/// One named variable visible in a paused machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebugVariable {
    pub name: String,
    pub value: Value,
}

/// Profile hits attributed to one source line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceProfileHit {
    pub line: u32,
    pub hits: u64,
}

/// An opaque, replayable debugger checkpoint.
#[derive(Debug, Clone)]
pub struct DebugSnapshot {
    machine: Machine,
    skip_breakpoint_pc: Option<usize>,
}

/// A deterministic source debugger over a cloned VM state.
#[derive(Debug, Clone)]
pub struct DebugSession {
    machine: Machine,
    debug: DebugTable,
    breakpoints: BTreeSet<u32>,
    skip_breakpoint_pc: Option<usize>,
}

impl DebugSession {
    /// Validates `program`, enables bounded profiling, and retains its debug table.
    ///
    /// # Errors
    /// Returns the normal bytecode validation error for an invalid program.
    pub fn new(program: Program) -> Result<Self, ProgramValidationError> {
        Self::with_policy(program, ExecutionPolicy::default())
    }

    /// Creates a debugger with an explicit execution policy.
    ///
    /// # Errors
    /// Returns the normal bytecode validation error for an invalid program.
    pub fn with_policy(
        program: Program,
        policy: ExecutionPolicy,
    ) -> Result<Self, ProgramValidationError> {
        let debug = program.debug_table();
        let machine = Machine::new_with_policy(program, policy)?;
        Ok(Self::from_machine(machine, debug))
    }

    /// Wraps an initialized machine and its matching debug metadata.
    #[must_use]
    pub const fn from_machine(machine: Machine, debug: DebugTable) -> Self {
        Self {
            machine,
            debug,
            breakpoints: BTreeSet::new(),
            skip_breakpoint_pc: None,
        }
    }

    /// Replaces the active set of one-based source-line breakpoints.
    pub fn set_breakpoints(&mut self, lines: impl IntoIterator<Item = u32>) {
        self.breakpoints.clear();
        self.breakpoints
            .extend(lines.into_iter().filter(|line| *line != 0));
    }

    /// Requests cooperative cancellation at the next VM fuel checkpoint.
    pub fn cancel(&mut self) {
        self.machine.cancel();
    }

    /// Clears a previous cooperative cancellation request.
    pub fn clear_cancellation(&mut self) {
        self.machine.clear_cancellation();
    }

    /// Returns the current source location, if the next op has one.
    #[must_use]
    pub fn location(&self) -> Option<DebugLocation> {
        self.debug
            .op(self.machine.pc)
            .filter(|location| location.line != 0)
    }

    /// Runs until a breakpoint, host-effect boundary, or completion.
    ///
    /// # Errors
    /// Returns VM evaluation and execution-policy failures unchanged.
    pub fn continue_execution(&mut self) -> Result<DebugEvent, EvalError> {
        let mut immediate_fuel = 0;
        loop {
            if self.machine.finished {
                return Ok(DebugEvent::Finished);
            }
            if self.machine.pending_host.is_some() {
                return Err(EvalError::new(
                    self.machine.current_line(),
                    "debugger is waiting for a host reply; call `resume`",
                ));
            }
            let pc = self.machine.pc;
            if self.skip_breakpoint_pc != Some(pc)
                && self
                    .location()
                    .is_some_and(|location| self.breakpoints.contains(&location.line))
            {
                self.skip_breakpoint_pc = Some(pc);
                return Ok(DebugEvent::Paused {
                    reason: DebugPauseReason::Breakpoint,
                    location: self
                        .location()
                        .unwrap_or(DebugLocation { line: 0, column: 0 }),
                });
            }
            self.skip_breakpoint_pc = None;
            if let Some(event) = self.execute_one(&mut immediate_fuel)? {
                return Ok(event);
            }
        }
    }

    /// Executes one bytecode operation and pauses at the next mapped location.
    ///
    /// # Errors
    /// Returns VM evaluation and execution-policy failures unchanged.
    pub fn step(&mut self) -> Result<DebugEvent, EvalError> {
        if self.machine.finished {
            return Ok(DebugEvent::Finished);
        }
        if self.machine.pending_host.is_some() {
            return Err(EvalError::new(
                self.machine.current_line(),
                "debugger is waiting for a host reply; call `resume`",
            ));
        }
        let mut immediate_fuel = 0;
        if let Some(event) = self.execute_one(&mut immediate_fuel)? {
            return Ok(event);
        }
        if self.machine.finished {
            return Ok(DebugEvent::Finished);
        }
        self.skip_breakpoint_pc = Some(self.machine.pc);
        Ok(DebugEvent::Paused {
            reason: DebugPauseReason::Step,
            location: self
                .location()
                .unwrap_or(DebugLocation { line: 0, column: 0 }),
        })
    }

    /// Supplies the pending host result without running past the effect boundary.
    ///
    /// # Errors
    /// Returns a contract error when no effect is pending or a required value
    /// is absent, and preserves normal VM resource errors.
    pub fn resume(&mut self, value: Option<Value>) -> Result<DebugEvent, EvalError> {
        self.machine.resume_pending(value)?;
        self.skip_breakpoint_pc = None;
        if self.machine.finished {
            Ok(DebugEvent::Finished)
        } else {
            Ok(DebugEvent::Paused {
                reason: DebugPauseReason::Step,
                location: self
                    .location()
                    .unwrap_or(DebugLocation { line: 0, column: 0 }),
            })
        }
    }

    /// Returns cloned named values, ordered by dense slot ID.
    #[must_use]
    pub fn variables(&self) -> Vec<DebugVariable> {
        self.machine
            .program
            .slots
            .names()
            .iter()
            .enumerate()
            .filter_map(|(slot, name)| {
                (!name.starts_with('\0'))
                    .then(|| self.machine.frame.values.get(slot)?.as_ref())
                    .flatten()
                    .map(|value| DebugVariable {
                        name: name.clone(),
                        value: value.clone(),
                    })
            })
            .collect()
    }

    /// Captures all replay-relevant VM state, including fuel and RNG state.
    #[must_use]
    pub fn snapshot(&self) -> DebugSnapshot {
        DebugSnapshot {
            machine: self.machine.clone(),
            skip_breakpoint_pc: self.skip_breakpoint_pc,
        }
    }

    /// Restores a checkpoint while retaining the session's breakpoint set.
    pub fn restore(&mut self, snapshot: &DebugSnapshot) {
        self.machine = snapshot.machine.clone();
        self.skip_breakpoint_pc = snapshot.skip_breakpoint_pc;
    }

    /// Aggregates the VM's bounded PC counters by one-based source line.
    #[must_use]
    pub fn source_profile(&self) -> Vec<SourceProfileHit> {
        let mut hits = BTreeMap::<u32, u64>::new();
        for (pc, count) in self.machine.profile().op_hits().iter().copied().enumerate() {
            let Some(location) = self.debug.op(pc) else {
                continue;
            };
            if location.line != 0 && count != 0 {
                let entry = hits.entry(location.line).or_default();
                *entry = entry.saturating_add(count);
            }
        }
        hits.into_iter()
            .map(|(line, hits)| SourceProfileHit { line, hits })
            .collect()
    }

    /// Provides read-only access to the underlying bounded machine.
    #[must_use]
    pub const fn machine(&self) -> &Machine {
        &self.machine
    }

    fn execute_one(&mut self, immediate_fuel: &mut u64) -> Result<Option<DebugEvent>, EvalError> {
        let cost = u64::try_from(self.machine.step_cost()).unwrap_or(u64::MAX);
        self.machine.consume_fuel(cost, immediate_fuel)?;
        let location = self
            .location()
            .unwrap_or(DebugLocation { line: 0, column: 0 });
        match self.machine.step()? {
            Some(Yield::Host { host_id, values }) => Ok(Some(DebugEvent::Host {
                host_id,
                values,
                location,
            })),
            Some(Yield::Finished) => Ok(Some(DebugEvent::Finished)),
            None if self.machine.finished => Ok(Some(DebugEvent::Finished)),
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use velin_bytecode::{Op, Program, SlotTable};

    fn program() -> Program {
        let mut slots = SlotTable::new();
        let value = slots.intern("value");
        Program::from_chunks(
            vec![
                Op::SetConst {
                    slot: value,
                    value: Value::Integer(1),
                    line: 1,
                },
                Op::host(0, Vec::new(), None, 2),
                Op::SetConst {
                    slot: value,
                    value: Value::Integer(2),
                    line: 3,
                },
                Op::Halt,
            ],
            Vec::new(),
            slots,
        )
    }

    #[test]
    fn breakpoints_steps_variables_snapshots_and_profiles_share_debug_locations() {
        let mut session = DebugSession::new(program()).unwrap();
        session.set_breakpoints([1, 3]);
        assert!(matches!(
            session.continue_execution().unwrap(),
            DebugEvent::Paused {
                reason: DebugPauseReason::Breakpoint,
                location: DebugLocation { line: 1, .. }
            }
        ));
        let snapshot = session.snapshot();
        assert!(matches!(
            session.continue_execution().unwrap(),
            DebugEvent::Host {
                location: DebugLocation { line: 2, .. },
                ..
            }
        ));
        session.resume(None).unwrap();
        assert!(matches!(
            session.continue_execution().unwrap(),
            DebugEvent::Paused {
                location: DebugLocation { line: 3, .. },
                ..
            }
        ));
        session.step().unwrap();
        assert_eq!(session.variables()[0].value, Value::Integer(2));
        assert!(!session.source_profile().is_empty());
        session.restore(&snapshot);
        assert!(session.variables().is_empty());
    }
}
