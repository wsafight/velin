use super::support::{cache_metrics, checked_total, eval_chunk_for};
use super::{
    BinaryOp, DataFootprint, DataMetrics, EvalError, MAX_HOST_PAYLOAD_TEXT_BYTES,
    MAX_HOST_PAYLOAD_VALUES, MAX_IMMEDIATE_STEPS, MAX_MACHINE_DATA_VALUES, MAX_MACHINE_TEXT_BYTES,
    Machine, Op, PendingHost, UpdateOp, Value, Yield,
};

impl Machine {
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
                    key,
                )?;
                let (value, metrics) = eval_chunk_for(
                    &self.program,
                    &self.metadata,
                    &mut self.frame,
                    &mut self.expression_stack,
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
            self.assign(slot, value, pending.line)?;
        }
        self.pending_host = None;
        self.run()
    }

    /// Executes one op. Returns `Some(Yield::Host)` if it yielded, `None`
    /// otherwise. Advances `pc` accordingly.
    #[allow(clippy::too_many_lines)]
    fn step(&mut self) -> Result<Option<Yield>, EvalError> {
        match &self.program.ops[self.pc] {
            Op::Set { slot, value } => {
                let slot = *slot;
                let value = *value;
                let (result, metrics) = eval_chunk_for(
                    &self.program,
                    &self.metadata,
                    &mut self.frame,
                    &mut self.expression_stack,
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
                self.step_update(slot, operation, line)?;
                self.pc += 1;
            }
            Op::Jump(target) => self.pc = *target as usize,
            Op::JumpIfFalse { condition, target } => {
                let condition_line = self.program.chunks[*condition as usize].line as usize;
                let condition_value = eval_chunk_for(
                    &self.program,
                    &self.metadata,
                    &mut self.frame,
                    &mut self.expression_stack,
                    *condition,
                )?
                .0;
                match condition_value {
                    Value::Boolean(false) => self.pc = *target as usize,
                    Value::Boolean(true) => self.pc += 1,
                    value => {
                        return Err(EvalError::new(
                            condition_line,
                            format!("condition expects boolean, found {}", value.type_name()),
                        ));
                    }
                }
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
                let mut values = Vec::with_capacity(host.args.len());
                let mut payload = DataFootprint::default();
                for chunk in host.args.iter().copied() {
                    let (value, metrics) = eval_chunk_for(
                        &self.program,
                        &self.metadata,
                        &mut self.frame,
                        &mut self.expression_stack,
                        chunk,
                    )?;
                    payload = checked_total(
                        payload,
                        metrics.footprint,
                        MAX_HOST_PAYLOAD_VALUES,
                        MAX_HOST_PAYLOAD_TEXT_BYTES,
                        "host payload",
                    )
                    .map_err(|error| EvalError::new(line, error))?;
                    values.push(value);
                }
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

    fn current_line(&self) -> usize {
        self.metadata
            .op(self.pc)
            .map_or(0, |metadata| metadata.line as usize)
    }
}
