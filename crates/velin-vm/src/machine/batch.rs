use super::support::eval_host_args;
use super::{EvalError, HostEffect, MAX_IMMEDIATE_STEPS, Machine, Op};

impl Machine {
    /// Runs side-effect-only host commands into the machine's reusable batch
    /// buffer and returns the number of collected effects.
    ///
    /// The returned effects remain borrowed from the machine until the next
    /// execution call. Use [`Machine::drain_effect_batch`] to move them into a
    /// host-owned queue while retaining the buffer allocation for later
    /// batches. If execution fails after collecting effects, the effects are
    /// returned first and the error is reported by the next execution call.
    ///
    /// # Errors
    /// Returns an error when `limit` is zero, the machine is waiting for a
    /// host reply, or a previously deferred execution error is pending. An
    /// immediate-step or effect-argument error is deferred when the batch has
    /// already collected effects, and returned on the next execution call.
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
        if let Some(error) = &self.pending_batch_error {
            return Err(error.clone());
        }
        self.effect_buffer.clear();
        self.effect_buffer.reserve(limit.min(MAX_IMMEDIATE_STEPS));
        let mut steps = 0;
        while !self.finished && self.effect_buffer.len() < limit {
            steps += self.step_cost();
            if steps > MAX_IMMEDIATE_STEPS {
                return self.defer_batch_error(EvalError::new(
                    self.current_line(),
                    "possible infinite loop: too many steps without yielding",
                ));
            }
            if self.pc >= self.program.ops.len() {
                self.finished = true;
                break;
            }
            let Op::Host(host) = &self.program.ops[self.pc] else {
                match self.step() {
                    Ok(Some(_)) => {
                        unreachable!("bound host operations are handled before stepping")
                    }
                    Ok(None) => {}
                    Err(error) => return self.defer_batch_error(error),
                }
                continue;
            };
            if host.bind.is_some() {
                break;
            }
            self.profile.record(self.pc);
            let host_id = host.host_id;
            let values = match eval_host_args(
                &self.program,
                &self.metadata,
                &mut self.frame,
                &mut self.register_values,
                &mut self.register_metrics,
                host,
            ) {
                Ok(values) => values,
                Err(error) => return self.defer_batch_error(error),
            };
            self.pc += 1;
            self.effect_buffer.push(HostEffect { host_id, values });
        }
        Ok(self.effect_buffer.len())
    }

    fn defer_batch_error(&mut self, error: EvalError) -> Result<usize, EvalError> {
        if self.effect_buffer.is_empty() {
            Err(error)
        } else {
            self.pending_batch_error = Some(error);
            Ok(self.effect_buffer.len())
        }
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
        destination.append(&mut self.effect_buffer);
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
    /// When an error occurs after effects were collected, the effects are
    /// returned first and the error is returned by the next execution call.
    pub fn run_effect_batch(&mut self, limit: usize) -> Result<Vec<HostEffect>, EvalError> {
        self.run_effect_batch_reusable(limit)?;
        let mut effects = Vec::new();
        self.drain_effect_batch(&mut effects);
        Ok(effects)
    }
}
