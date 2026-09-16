use super::support::eval_chunk_for;
use super::{EvalError, FastYield, Machine, Op, Yield};

impl Machine {
    /// Runs until completion or a selected single-argument host operation.
    ///
    /// Host operations whose IDs appear in `single_argument_hosts` and have
    /// exactly one argument use [`FastYield::HostOne`], avoiding a temporary
    /// one-element vector. Other host operations retain the ordinary yield
    /// representation and semantics.
    ///
    /// # Errors
    /// Returns the same evaluation and immediate-step errors as [`Self::run`].
    pub fn run_with_single_argument_hosts(
        &mut self,
        single_argument_hosts: &[u32],
    ) -> Result<FastYield, EvalError> {
        if let Some(pending) = self.pending_host {
            return Err(EvalError::new(
                pending.line,
                "machine is waiting for the host; call `resume`",
            ));
        }
        if let Some(error) = &self.pending_batch_error {
            return Err(error.clone());
        }
        let mut immediate_fuel = 0;
        while !self.finished {
            let fused_jump_target = self.update_jump_target();
            self.consume_fuel(
                if fused_jump_target.is_some() { 2 } else { 1 },
                &mut immediate_fuel,
            )?;
            if self.pc >= self.program.ops.len() {
                self.finished = true;
                break;
            }
            if let Some(effect) = self.step_single_argument_host(single_argument_hosts)? {
                return Ok(effect);
            }
            if let Some(effect) = self.step_with_known_jump_target(fused_jump_target)? {
                return Ok(match effect {
                    Yield::Host { host_id, values } => FastYield::Host { host_id, values },
                    Yield::Finished => FastYield::Finished,
                });
            }
        }
        Ok(FastYield::Finished)
    }

    fn step_single_argument_host(
        &mut self,
        single_argument_hosts: &[u32],
    ) -> Result<Option<FastYield>, EvalError> {
        let Some(Op::Host(host)) = self.program.ops.get(self.pc) else {
            return Ok(None);
        };
        if host.args.len() != 1 || !single_argument_hosts.contains(&host.host_id) {
            return Ok(None);
        }
        let host_id = host.host_id;
        let bind = host.bind;
        let line = host.line as usize;
        let chunk = host.args[0];
        self.profile.record(self.pc);
        let (value, metrics) = eval_chunk_for(
            &self.program,
            &self.metadata,
            &mut self.frame,
            &mut self.register_values,
            &mut self.register_metrics,
            &mut self.register_touched,
            chunk,
        )?;
        if metrics.footprint.values > self.policy.max_value_values
            || metrics.footprint.text_bytes > self.policy.max_value_text_bytes
            || metrics.footprint.values > self.policy.max_host_payload_values
            || metrics.footprint.text_bytes > self.policy.max_host_payload_text_bytes
        {
            return Err(EvalError::new(
                line,
                "host payload value exceeds the execution policy",
            ));
        }
        self.record_host_effect()?;
        self.pc += 1;
        self.pending_host = Some(super::PendingHost { bind, line });
        Ok(Some(FastYield::HostOne { host_id, value }))
    }
}
