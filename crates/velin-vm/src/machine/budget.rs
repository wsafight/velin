use super::{EvalError, ExecutionPolicy, ExecutionProgress, Machine};

impl Machine {
    /// Returns the policy copied into this machine and its snapshots.
    #[must_use]
    pub const fn policy(&self) -> &ExecutionPolicy {
        &self.policy
    }

    /// Returns cumulative fuel consumed since construction or restart.
    #[must_use]
    pub const fn fuel_used(&self) -> u64 {
        self.fuel_used
    }

    /// Returns host effects yielded since construction or restart.
    #[must_use]
    pub const fn host_effects(&self) -> usize {
        self.host_effects
    }

    /// Returns the current VM call depth.
    #[must_use]
    pub const fn call_depth(&self) -> usize {
        self.call_depth
    }

    /// Requests cooperative cancellation at the next fuel checkpoint.
    pub fn cancel(&mut self) {
        self.cancelled = true;
    }

    /// Clears a previous cancellation request before starting another run.
    pub fn clear_cancellation(&mut self) {
        self.cancelled = false;
    }

    /// Returns whether the host has requested cancellation.
    #[must_use]
    pub const fn is_cancelled(&self) -> bool {
        self.cancelled
    }

    pub(super) fn check_execution_policy(&self) -> Result<(), EvalError> {
        if self.frame_total.values > self.policy.max_machine_values
            || self.frame_total.text_bytes > self.policy.max_machine_text_bytes
        {
            return Err(EvalError::new(
                self.current_line(),
                "machine state exceeds the execution policy",
            ));
        }
        if self.call_depth > self.policy.max_call_depth {
            return Err(EvalError::new(
                self.current_line(),
                format!(
                    "call depth exceeds execution policy (limit {})",
                    self.policy.max_call_depth
                ),
            ));
        }
        if self.cancelled {
            return Err(EvalError::cancelled(self.current_line()));
        }
        Ok(())
    }

    pub(super) fn consume_fuel(
        &mut self,
        amount: u64,
        immediate: &mut u64,
    ) -> Result<(), EvalError> {
        self.check_execution_policy()?;
        self.consume_fuel_prechecked(amount, immediate)
    }

    #[inline]
    pub(super) fn consume_fuel_prechecked(
        &mut self,
        amount: u64,
        immediate: &mut u64,
    ) -> Result<(), EvalError> {
        let next_immediate = immediate.checked_add(amount).ok_or_else(|| {
            EvalError::fuel_exhausted(self.current_line(), self.policy.max_immediate_fuel, true)
        })?;
        if next_immediate > self.policy.max_immediate_fuel {
            return Err(EvalError::fuel_exhausted(
                self.current_line(),
                self.policy.max_immediate_fuel,
                true,
            ));
        }
        let next_total = self.fuel_used.checked_add(amount).ok_or_else(|| {
            EvalError::fuel_exhausted(self.current_line(), self.policy.max_fuel, false)
        })?;
        if next_total > self.policy.max_fuel {
            return Err(EvalError::fuel_exhausted(
                self.current_line(),
                self.policy.max_fuel,
                false,
            ));
        }
        *immediate = next_immediate;
        self.fuel_used = next_total;
        if let Some(callback) = &self.policy.progress_callback {
            let interval = self.policy.progress_interval.max(1);
            let crossed = self.fuel_used / interval != (self.fuel_used - amount) / interval;
            if crossed
                && !callback(ExecutionProgress {
                    fuel_used: self.fuel_used,
                    immediate_fuel: *immediate,
                    host_effects: self.host_effects,
                    call_depth: self.call_depth,
                })
            {
                self.cancelled = true;
                return Err(EvalError::cancelled(self.current_line()));
            }
        }
        Ok(())
    }

    pub(super) fn record_host_effect(&mut self) -> Result<(), EvalError> {
        if self.host_effects >= self.policy.max_host_effects {
            return Err(EvalError::host_effects_exceeded(
                self.current_line(),
                self.policy.max_host_effects,
            ));
        }
        self.host_effects += 1;
        Ok(())
    }
}
