use super::support::{
    adjusted_footprint, cache_metrics, checked_total, ensure_child_depth, updated_collection_depth,
    value_metrics,
};
use super::{
    Arc, DataFootprint, DataMetrics, EvalError, MAX_DATA_TEXT_BYTES, MAX_MACHINE_DATA_VALUES,
    MAX_MACHINE_TEXT_BYTES, Machine, Value,
};

impl Machine {
    pub(super) fn update_add(
        &mut self,
        slot: u32,
        rhs: Value,
        line: usize,
    ) -> Result<(), EvalError> {
        let source = self.frame.values[slot as usize]
            .as_ref()
            .expect("update source was checked");
        let footprint = match (source, &rhs) {
            (Value::Integer(left), Value::Integer(right)) => {
                left.checked_add(*right)
                    .ok_or_else(|| EvalError::new(line, "integer overflow"))?;
                DataFootprint {
                    values: 1,
                    text_bytes: 0,
                }
            }
            (Value::String(left), Value::String(right)) => {
                let text_bytes = left
                    .len()
                    .checked_add(right.len())
                    .ok_or_else(|| EvalError::new(line, "data text exceeds 1 MiB"))?;
                if text_bytes > MAX_DATA_TEXT_BYTES {
                    return Err(EvalError::new(line, "data text exceeds 1 MiB"));
                }
                DataFootprint {
                    values: 1,
                    text_bytes,
                }
            }
            _ => {
                return Err(EvalError::new(
                    line,
                    format!(
                        "`+` cannot combine {} and {}",
                        source.type_name(),
                        rhs.type_name()
                    ),
                ));
            }
        };
        let total = self.replacement_total(slot, footprint, line)?;
        let source = self.frame.values[slot as usize]
            .take()
            .expect("update source was checked");
        let result = match (source, rhs) {
            (Value::Integer(left), Value::Integer(right)) => {
                Value::Integer(left.checked_add(right).expect("addition was preflighted"))
            }
            (Value::String(mut left), Value::String(right)) => {
                left.make_mut().push_str(&right);
                Value::String(left)
            }
            _ => unreachable!("update types were preflighted"),
        };
        self.install_prechecked(
            slot,
            result,
            DataMetrics {
                footprint,
                max_depth: 0,
            },
            total,
        );
        Ok(())
    }

    pub(super) fn update_push(
        &mut self,
        slot: u32,
        value: Value,
        metrics: DataMetrics,
        line: usize,
    ) -> Result<(), EvalError> {
        let Value::List(_) = self.frame.values[slot as usize]
            .as_ref()
            .expect("update source was checked")
        else {
            return Err(EvalError::new(line, "push expects a list"));
        };
        ensure_child_depth(metrics, line)?;
        let current = self.frame.metrics(slot as usize);
        let footprint = adjusted_footprint(
            current.footprint,
            DataFootprint::default(),
            metrics.footprint,
            0,
            0,
            line,
        )?;
        let total = self.replacement_total(slot, footprint, line)?;
        let Value::List(mut values) = self.frame.values[slot as usize]
            .take()
            .expect("update source was checked")
        else {
            unreachable!("update type was preflighted")
        };
        Arc::make_mut(&mut values).push(value);
        self.install_prechecked(
            slot,
            Value::List(values),
            DataMetrics {
                footprint,
                max_depth: current.max_depth.max(metrics.max_depth + 1),
            },
            total,
        );
        Ok(())
    }

    pub(super) fn update_put(
        &mut self,
        slot: u32,
        key: Value,
        value: Value,
        metrics: DataMetrics,
        line: usize,
    ) -> Result<(), EvalError> {
        ensure_child_depth(metrics, line)?;
        let source = self.frame.values[slot as usize]
            .as_ref()
            .expect("update source was checked");
        let (removed, removed_key_bytes, added_key_bytes) = match (source, &key) {
            (Value::List(values), Value::Integer(index)) => {
                let index = usize::try_from(*index)
                    .ok()
                    .filter(|index| *index < values.len())
                    .ok_or_else(|| EvalError::new(line, "list index out of bounds"))?;
                (Some(value_metrics(&values[index], line)?), 0, 0)
            }
            (Value::Record(values), Value::String(key)) => match values.get(key.as_str()) {
                Some(previous) => (Some(value_metrics(previous, line)?), 0, 0),
                None => (None, 0, key.len()),
            },
            _ => {
                return Err(EvalError::new(
                    line,
                    "put/remove expects a list and integer index, or a record and string key",
                ));
            }
        };
        let current = self.frame.metrics(slot as usize);
        let footprint = adjusted_footprint(
            current.footprint,
            removed.map_or(DataFootprint::default(), |metrics| metrics.footprint),
            metrics.footprint,
            removed_key_bytes,
            added_key_bytes,
            line,
        )?;
        let total = self.replacement_total(slot, footprint, line)?;
        let source = self.frame.values[slot as usize]
            .take()
            .expect("update source was checked");
        let result = match (source, key) {
            (Value::List(mut values), Value::Integer(index)) => {
                let index = usize::try_from(index).expect("list index was preflighted");
                Arc::make_mut(&mut values)[index] = value;
                Value::List(values)
            }
            (Value::Record(mut values), Value::String(key)) => {
                Arc::make_mut(&mut values).insert(key.into_string(), value);
                Value::Record(values)
            }
            _ => unreachable!("update types were preflighted"),
        };
        let max_depth = updated_collection_depth(current, removed, Some(metrics), &result, line)?;
        self.install_prechecked(
            slot,
            result,
            DataMetrics {
                footprint,
                max_depth,
            },
            total,
        );
        Ok(())
    }

    pub(super) fn update_remove(
        &mut self,
        slot: u32,
        key: Value,
        line: usize,
    ) -> Result<(), EvalError> {
        let source = self.frame.values[slot as usize]
            .as_ref()
            .expect("update source was checked");
        let (removed, removed_key_bytes) = match (source, &key) {
            (Value::List(values), Value::Integer(index)) => {
                let index = usize::try_from(*index)
                    .ok()
                    .filter(|index| *index < values.len())
                    .ok_or_else(|| EvalError::new(line, "list index out of bounds"))?;
                (value_metrics(&values[index], line)?, 0)
            }
            (Value::Record(values), Value::String(key)) => {
                let previous = values
                    .get(key.as_str())
                    .ok_or_else(|| EvalError::new(line, "missing record key"))?;
                (value_metrics(previous, line)?, key.len())
            }
            _ => {
                return Err(EvalError::new(
                    line,
                    "put/remove expects a list and integer index, or a record and string key",
                ));
            }
        };
        let current = self.frame.metrics(slot as usize);
        let footprint = adjusted_footprint(
            current.footprint,
            removed.footprint,
            DataFootprint::default(),
            removed_key_bytes,
            0,
            line,
        )?;
        let total = self.replacement_total(slot, footprint, line)?;
        let source = self.frame.values[slot as usize]
            .take()
            .expect("update source was checked");
        let result = match (source, key) {
            (Value::List(mut values), Value::Integer(index)) => {
                let index = usize::try_from(index).expect("list index was preflighted");
                Arc::make_mut(&mut values).remove(index);
                Value::List(values)
            }
            (Value::Record(mut values), Value::String(key)) => {
                Arc::make_mut(&mut values).remove(key.as_str());
                Value::Record(values)
            }
            _ => unreachable!("update types were preflighted"),
        };
        let max_depth = updated_collection_depth(current, Some(removed), None, &result, line)?;
        self.install_prechecked(
            slot,
            result,
            DataMetrics {
                footprint,
                max_depth,
            },
            total,
        );
        Ok(())
    }

    fn replacement_total(
        &self,
        slot: u32,
        footprint: DataFootprint,
        line: usize,
    ) -> Result<DataFootprint, EvalError> {
        let old = self.frame.metrics(slot as usize).footprint;
        let retained = DataFootprint {
            values: self.frame_total.values - old.values,
            text_bytes: self.frame_total.text_bytes - old.text_bytes,
        };
        checked_total(
            retained,
            footprint,
            MAX_MACHINE_DATA_VALUES,
            MAX_MACHINE_TEXT_BYTES,
            "machine state",
        )
        .map_err(|error| EvalError::new(line, error))
    }

    fn install_prechecked(
        &mut self,
        slot: u32,
        value: Value,
        metrics: DataMetrics,
        total: DataFootprint,
    ) {
        let index = slot as usize;
        let frame = &mut self.frame;
        frame.values[index] = Some(value);
        cache_metrics(frame, index, metrics);
        self.frame_total = total;
    }
}
