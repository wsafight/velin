use super::support::eval_host_args;
use super::{EvalError, HostEffect, MAX_IMMEDIATE_STEPS, Machine, Op};

impl Machine {
    /// Runs until completion, a value-returning host command, or `limit`
    /// side-effect-only host commands have been collected.
    ///
    /// Only host operations without a `bind` destination are collected. A
    /// bound operation is left at the current program counter so the caller
    /// can use the existing [`Machine::run`] and [`Machine::resume`] protocol.
    /// An empty result means that execution is either finished or waiting at a
    /// bound host operation; call [`Machine::run`] to distinguish those cases.
    ///
    /// # Errors
    /// Returns the same evaluation and immediate-step errors as [`Machine::run`].
    pub fn run_effect_batch(&mut self, limit: usize) -> Result<Vec<HostEffect>, EvalError> {
        if limit == 0 {
            return Err(EvalError::new(
                self.current_line(),
                "host effect batch limit must be greater than zero",
            ));
        }
        if let Some(pending) = self.pending_host {
            return Err(EvalError::new(
                pending.line,
                "machine is waiting for the host; call `resume`",
            ));
        }
        let mut effects = Vec::new();
        let mut steps = 0;
        while !self.finished && effects.len() < limit {
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
            let Op::Host(host) = &self.program.ops[self.pc] else {
                if self.step()?.is_some() {
                    unreachable!("bound host operations are handled before stepping")
                }
                continue;
            };
            if host.bind.is_some() {
                break;
            }
            let host_id = host.host_id;
            let values = eval_host_args(
                &self.program,
                &self.metadata,
                &mut self.frame,
                &mut self.expression_stack,
                &mut self.expression_metrics,
                &mut self.register_values,
                host,
            )?;
            self.pc += 1;
            effects.push(HostEffect { host_id, values });
        }
        Ok(effects)
    }
}
