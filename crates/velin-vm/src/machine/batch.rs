use super::support::eval_host_args;
use super::{EvalError, HostEffect, MAX_IMMEDIATE_STEPS, Machine, Op};

impl Machine {
    /// Runs side-effect-only host commands into the machine's reusable batch
    /// buffer and returns the number of collected effects.
    ///
    /// The returned effects remain borrowed from the machine until the next
    /// execution call. Use [`Machine::drain_effect_batch`] to move them into a
    /// host-owned queue while retaining the buffer allocation for later
    /// batches.
    pub fn run_effect_batch_reusable(&mut self, limit: usize) -> Result<usize, EvalError> {
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
        self.effect_buffer.clear();
        self.effect_buffer.reserve(limit);
        let mut steps = 0;
        while !self.finished && self.effect_buffer.len() < limit {
            steps += self.step_cost();
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
            self.effect_buffer.push(HostEffect { host_id, values });
        }
        Ok(self.effect_buffer.len())
    }

    /// Borrows the effects collected by [`Machine::run_effect_batch_reusable`].
    #[must_use]
    pub fn effect_batch(&self) -> &[HostEffect] {
        &self.effect_buffer
    }

    /// Moves the current batch into `destination` and retains its allocation
    /// inside the machine for the next batch.
    pub fn drain_effect_batch(&mut self, destination: &mut Vec<HostEffect>) {
        destination.clear();
        destination.extend(self.effect_buffer.drain(..));
    }

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
        self.run_effect_batch_reusable(limit)?;
        let mut effects = Vec::new();
        self.drain_effect_batch(&mut effects);
        Ok(effects)
    }
}
