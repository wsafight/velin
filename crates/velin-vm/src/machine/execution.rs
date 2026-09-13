use super::support::{cache_metrics, checked_total, eval_chunk_for, eval_host_args};
use super::{
    BinaryOp, DataFootprint, DataMetrics, EvalError, InitialFrame, MAX_IMMEDIATE_STEPS,
    MAX_MACHINE_DATA_VALUES, MAX_MACHINE_TEXT_BYTES, Machine, Op, PendingHost, PreparedExpr,
    UpdateOp, Value, Yield,
};

impl Machine {
    /// Restarts execution from a prevalidated initial frame while reusing the
    /// machine's frame and expression-stack allocations.
    ///
    /// The frame width and aggregate budget are checked before any state is
    /// changed, so a failed restart leaves the current machine untouched.
    ///
    /// # Errors
    /// Returns an error when the frame width or aggregate machine-state budget
    /// is incompatible with this machine.
    pub fn restart(&mut self, initial: &InitialFrame, seed: i64) -> Result<(), &'static str> {
        if initial.values().len() != self.program.slots.len() {
            return Err("initial frame width does not match program slots");
        }
        let mut total = initial.total();
        if let Some(slot) = self.program.slots.rng_state() {
            let index = slot as usize;
            let old = initial.footprints()[index];
            total.values = total
                .values
                .checked_sub(old.values)
                .and_then(|values| values.checked_add(1))
                .ok_or("machine state value count overflow")?;
            total.text_bytes = total
                .text_bytes
                .checked_sub(old.text_bytes)
                .ok_or("machine state text size overflow")?;
            if total.values > MAX_MACHINE_DATA_VALUES {
                return Err("machine state exceeds 100,000 values");
            }
            if total.text_bytes > MAX_MACHINE_TEXT_BYTES {
                return Err("machine state text exceeds 16 MiB");
            }
        } else if total.values > MAX_MACHINE_DATA_VALUES {
            return Err("machine state exceeds 100,000 values");
        } else if total.text_bytes > MAX_MACHINE_TEXT_BYTES {
            return Err("machine state text exceeds 16 MiB");
        }

        self.frame.values.clone_from_slice(initial.values());
        self.frame.footprints.clone_from_slice(initial.footprints());
        self.frame.depths.clone_from_slice(initial.depths());
        self.frame_total = total;
        if let Some(slot) = self.program.slots.rng_state() {
            let index = slot as usize;
            self.frame.values[index] = Some(Value::Integer(seed));
            cache_metrics(
                &mut self.frame,
                index,
                DataMetrics {
                    footprint: DataFootprint {
                        values: 1,
                        text_bytes: 0,
                    },
                    max_depth: 0,
                },
            );
        }
        self.expression_stack.clear();
        self.expression_metrics.clear();
        self.register_values.clear();
        self.effect_buffer.clear();
        self.pc = 0;
        self.pending_host = None;
        self.finished = false;
        Ok(())
    }

    fn step_update(
        &mut self,
        slot: u32,
        operation: UpdateOp,
        line: usize,
    ) -> Result<(), EvalError> {
        if let UpdateOp::AddInteger { value } = operation {
            return self.update_add_integer(slot, value, line);
        }
        self.require_assigned(slot, line)?;
        match operation {
            UpdateOp::Add { rhs } => {
                let (rhs, _) = eval_chunk_for(
                    &self.program,
                    &self.metadata,
                    &mut self.frame,
                    &mut self.expression_stack,
                    &mut self.expression_metrics,
                    &mut self.register_values,
                    rhs,
                )?;
                self.update_add(slot, rhs, line)
            }
            UpdateOp::AddInteger { .. } => unreachable!("handled before expression updates"),
            UpdateOp::Push { value } => {
                let (value, metrics) = eval_chunk_for(
                    &self.program,
                    &self.metadata,
                    &mut self.frame,
                    &mut self.expression_stack,
                    &mut self.expression_metrics,
                    &mut self.register_values,
                    value,
                )?;
                self.update_push(slot, value, metrics, line)
            }
            UpdateOp::Put { key, value } => {
                let (key, _) = eval_chunk_for(
                    &self.program,
                    &self.metadata,
                    &mut self.frame,
                    &mut self.expression_stack,
                    &mut self.expression_metrics,
                    &mut self.register_values,
                    key,
                )?;
                let (value, metrics) = eval_chunk_for(
                    &self.program,
                    &self.metadata,
                    &mut self.frame,
                    &mut self.expression_stack,
                    &mut self.expression_metrics,
                    &mut self.register_values,
                    value,
                )?;
                self.update_put(slot, key, value, metrics, line)
            }
            UpdateOp::Remove { key } => {
                let (key, _) = eval_chunk_for(
                    &self.program,
                    &self.metadata,
                    &mut self.frame,
                    &mut self.expression_stack,
                    &mut self.expression_metrics,
                    &mut self.register_values,
                    key,
                )?;
                self.update_remove(slot, key, line)
            }
        }
    }

    fn update_add_integer(&mut self, slot: u32, value: i64, line: usize) -> Result<(), EvalError> {
        let index = slot as usize;
        let slot_name = self.program.slots.name(slot).unwrap_or("?");
        let frame = &mut self.frame;
        let Some(source) = frame.values[index].as_ref() else {
            return Err(velin_eval::unassigned(line, slot_name));
        };
        let Value::Integer(source) = source else {
            return Err(EvalError::new(
                line,
                format!("`+` cannot combine {} and integer", source.type_name()),
            ));
        };
        let result = source
            .checked_add(value)
            .ok_or_else(|| EvalError::new(line, "integer overflow"))?;
        let footprint = DataFootprint {
            values: 1,
            text_bytes: 0,
        };
        let old = frame.footprints[index];
        if old == footprint {
            frame.values[index] = Some(Value::Integer(result));
            cache_metrics(
                frame,
                index,
                DataMetrics {
                    footprint,
                    max_depth: 0,
                },
            );
            return Ok(());
        }
        let retained = DataFootprint {
            values: self.frame_total.values - old.values,
            text_bytes: self.frame_total.text_bytes - old.text_bytes,
        };
        let total = checked_total(
            retained,
            footprint,
            MAX_MACHINE_DATA_VALUES,
            MAX_MACHINE_TEXT_BYTES,
            "machine state",
        )
        .map_err(|error| EvalError::new(line, error))?;
        frame.values[index] = Some(Value::Integer(result));
        cache_metrics(
            frame,
            index,
            DataMetrics {
                footprint,
                max_depth: 0,
            },
        );
        self.frame_total = total;
        Ok(())
    }

    fn require_assigned(&self, slot: u32, line: usize) -> Result<(), EvalError> {
        if self.frame.values[slot as usize].is_some() {
            return Ok(());
        }
        Err(velin_eval::unassigned(
            line,
            self.program.slots.name(slot).unwrap_or("?"),
        ))
    }

    fn compare_integer_literal(
        source: &Value,
        comparison: BinaryOp,
        value: i64,
        line: usize,
    ) -> Result<bool, EvalError> {
        let Value::Integer(source) = source else {
            return match comparison {
                BinaryOp::Equal => Ok(false),
                BinaryOp::NotEqual => Ok(true),
                _ => Err(EvalError::new(
                    line,
                    format!(
                        "comparison cannot combine {} and integer",
                        source.type_name()
                    ),
                )),
            };
        };
        Ok(match comparison {
            BinaryOp::Equal => *source == value,
            BinaryOp::NotEqual => *source != value,
            BinaryOp::Less => *source < value,
            BinaryOp::LessEqual => *source <= value,
            BinaryOp::Greater => *source > value,
            BinaryOp::GreaterEqual => *source >= value,
            _ => unreachable!("validated integer comparison"),
        })
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
            self.assign(slot, value, pending.line)?;
        }
        self.pending_host = None;
        self.run()
    }

    /// Executes one op. Returns `Some(Yield::Host)` if it yielded, `None`
    /// otherwise. Advances `pc` accordingly.
    #[allow(clippy::too_many_lines)]
    pub(super) fn step(&mut self) -> Result<Option<Yield>, EvalError> {
        let current_pc = self.pc;
        self.profile.record(current_pc);
        if self.update_jump_target().is_some() {
            self.profile.record(current_pc.saturating_add(1));
        }
        match &self.program.ops[self.pc] {
            Op::Set { slot, value } => {
                let slot = *slot;
                let value = *value;
                let (result, metrics) = eval_chunk_for(
                    &self.program,
                    &self.metadata,
                    &mut self.frame,
                    &mut self.expression_stack,
                    &mut self.expression_metrics,
                    &mut self.register_values,
                    value,
                )?;
                let line = self.program.chunks[value as usize].line as usize;
                self.assign_measured(slot, result, metrics, line)?;
                self.pc += 1;
            }
            Op::SetConst { slot, value, line } => {
                let slot = *slot;
                let value = value.clone();
                let line = *line as usize;
                let metrics = self
                    .metadata
                    .op_constant_metrics(self.pc)
                    .expect("validated constant metadata");
                self.assign_measured(slot, value, metrics, line)?;
                self.pc += 1;
            }
            Op::CopySlot {
                slot, source, line, ..
            } => {
                let slot = *slot;
                let source = *source;
                let line = *line as usize;
                self.require_assigned(source, line)?;
                if slot != source {
                    let value = self.frame.values[source as usize]
                        .as_ref()
                        .expect("copy source was checked")
                        .clone();
                    let metrics = self.frame.metrics(source as usize);
                    self.assign_measured(slot, value, metrics, line)?;
                }
                self.pc += 1;
            }
            Op::Update {
                slot,
                operation,
                line,
                ..
            } => {
                let slot = *slot;
                let operation = *operation;
                let line = *line as usize;
                let jump_target = self.update_jump_target();
                self.step_update(slot, operation, line)?;
                self.pc = jump_target.unwrap_or(self.pc + 1);
            }
            Op::Jump(target) => self.pc = *target as usize,
            Op::JumpIfFalse { condition, target } => {
                self.step_jump_if_false(*condition, *target)?;
            }
            Op::JumpIfIntegerCompare {
                condition,
                slot,
                comparison,
                value,
                target,
            } => {
                let line = self.program.chunks[*condition as usize].line as usize;
                self.require_assigned(*slot, line)?;
                let source = self.frame.values[*slot as usize]
                    .as_ref()
                    .expect("comparison source was checked");
                let result = Self::compare_integer_literal(source, *comparison, *value, line)?;
                if result {
                    self.pc += 1;
                } else {
                    self.pc = *target as usize;
                }
            }
            Op::Host(host) => {
                let host_id = host.host_id;
                let bind = host.bind;
                let line = host.line as usize;
                let values = eval_host_args(
                    &self.program,
                    &self.metadata,
                    &mut self.frame,
                    &mut self.expression_stack,
                    &mut self.expression_metrics,
                    &mut self.register_values,
                    host,
                )?;
                self.pc += 1; // resume past the effect, never re-run it
                self.pending_host = Some(PendingHost { bind, line });
                return Ok(Some(Yield::Host { host_id, values }));
            }
            Op::Halt => self.finished = true,
        }
        Ok(None)
    }

    fn assign(&mut self, slot: u32, value: Value, line: usize) -> Result<(), EvalError> {
        let metrics = value
            .data_metrics()
            .map_err(|error| EvalError::new(line, error))?;
        self.assign_measured(slot, value, metrics, line)
    }

    fn step_jump_if_false(&mut self, condition: u32, target: u32) -> Result<(), EvalError> {
        let condition_line = self.program.chunks[condition as usize].line as usize;
        if let Some(PreparedExpr::Load { slot }) = self.metadata.prepared_expr(condition) {
            let value = self.frame.values[slot as usize].as_ref().ok_or_else(|| {
                velin_eval::unassigned(condition_line, self.program.slots.name(slot).unwrap_or("?"))
            })?;
            return match value {
                Value::Boolean(false) => {
                    self.pc = target as usize;
                    Ok(())
                }
                Value::Boolean(true) => {
                    self.pc += 1;
                    Ok(())
                }
                value => Err(EvalError::new(
                    condition_line,
                    format!("condition expects boolean, found {}", value.type_name()),
                )),
            };
        }

        let condition_value = eval_chunk_for(
            &self.program,
            &self.metadata,
            &mut self.frame,
            &mut self.expression_stack,
            &mut self.expression_metrics,
            &mut self.register_values,
            condition,
        )?
        .0;
        match condition_value {
            Value::Boolean(false) => self.pc = target as usize,
            Value::Boolean(true) => self.pc += 1,
            value => {
                return Err(EvalError::new(
                    condition_line,
                    format!("condition expects boolean, found {}", value.type_name()),
                ));
            }
        }
        Ok(())
    }

    fn assign_measured(
        &mut self,
        slot: u32,
        value: Value,
        metrics: DataMetrics,
        line: usize,
    ) -> Result<(), EvalError> {
        self.replace_slot(slot, value, metrics)
            .map_err(|error| EvalError::new(line, error))
    }

    pub(super) fn replace_slot(
        &mut self,
        slot: u32,
        value: Value,
        metrics: DataMetrics,
    ) -> Result<(), &'static str> {
        let index = slot as usize;
        let frame = &mut self.frame;
        let old = frame.footprints[index];
        if old == metrics.footprint {
            frame.values[index] = Some(value);
            frame.depths[index] = u8::try_from(metrics.max_depth)
                .expect("validated value depth fits in the compact frame cache");
            return Ok(());
        }
        let retained = DataFootprint {
            values: self.frame_total.values - old.values,
            text_bytes: self.frame_total.text_bytes - old.text_bytes,
        };
        let total = checked_total(
            retained,
            metrics.footprint,
            MAX_MACHINE_DATA_VALUES,
            MAX_MACHINE_TEXT_BYTES,
            "machine state",
        )?;
        frame.values[index] = Some(value);
        cache_metrics(frame, index, metrics);
        self.frame_total = total;
        Ok(())
    }

    pub(super) fn current_line(&self) -> usize {
        self.metadata
            .op(self.pc)
            .map_or(0, |metadata| metadata.line as usize)
    }

    /// Returns the number of original program ops represented by the next VM
    /// step. Update/jump tails are executed together, but still consume two
    /// budget units so the infinite-loop guard remains stable.
    pub(super) fn step_cost(&self) -> usize {
        if self.update_jump_target().is_some() {
            2
        } else {
            1
        }
    }

    fn update_jump_target(&self) -> Option<usize> {
        if !matches!(self.program.ops.get(self.pc), Some(Op::Update { .. })) {
            return None;
        }
        match self.program.ops.get(self.pc + 1) {
            Some(Op::Jump(target)) => Some(*target as usize),
            _ => None,
        }
    }
}
