use super::{
    Builtin, DataFootprint, DataMetrics, EvalError, ExecutionMetadata, FrameAccess, FrameState,
    MAX_DATA_DEPTH, MAX_DATA_TEXT_BYTES, MAX_DATA_VALUES, Program, QuickenedCallRef,
    QuickenedOperand, Value, eval_validated_chunk, invoke_readonly_measured, invoke_stack_measured,
};

#[inline]
pub(super) fn cache_metrics(frame: &mut FrameState, slot: usize, metrics: DataMetrics) {
    frame.footprints[slot] = metrics.footprint;
    frame.depths[slot] = u8::try_from(metrics.max_depth)
        .expect("validated value depth fits in the compact frame cache");
}

pub(super) fn eval_chunk_for(
    program: &Program,
    metadata: &ExecutionMetadata,
    frame: &mut FrameState,
    expression_stack: &mut Vec<Value>,
    chunk_id: u32,
) -> Result<(Value, DataMetrics), EvalError> {
    let (execution, result_metrics, quickened) = metadata
        .chunk_plan(chunk_id)
        .expect("validated chunk metadata");
    if let Some(call) = quickened {
        return eval_quickened_call(program, frame, expression_stack, call);
    }
    let chunk = program
        .chunk(chunk_id)
        .expect("program expression range was validated");
    let (frame, frame_metrics) = if execution.mutates_frame {
        let frame_metrics = execution
            .inherits_slot_metrics
            .then_some((frame.footprints.as_slice(), frame.depths.as_slice()));
        (
            FrameAccess::Mutable(frame.values.as_mut_slice()),
            frame_metrics,
        )
    } else {
        let frame_metrics = execution
            .inherits_slot_metrics
            .then_some((frame.footprints.as_slice(), frame.depths.as_slice()));
        (
            FrameAccess::ReadOnly(frame.values.as_slice()),
            frame_metrics,
        )
    };
    let slots = &program.slots;
    eval_validated_chunk(
        chunk,
        frame,
        frame_metrics,
        expression_stack,
        execution.max_stack as usize,
        result_metrics,
        |slot| slots.name(slot).unwrap_or("?").to_owned(),
    )
}

#[allow(clippy::option_as_ref_cloned)]
pub(super) fn eval_quickened_call(
    program: &Program,
    frame: &FrameState,
    stack: &mut Vec<Value>,
    call: QuickenedCallRef<'_>,
) -> Result<(Value, DataMetrics), EvalError> {
    let line = call.line as usize;
    stack.clear();
    if let Some(result) = eval_quickened_readonly_call(program, frame, call)? {
        return Ok(result);
    }
    if stack.capacity() < call.operands.len() {
        stack.reserve(call.operands.len() - stack.capacity());
    }
    for operand in call.operands {
        let value = match operand {
            QuickenedOperand::Constant(index) => program.constants[*index as usize].clone(),
            QuickenedOperand::Slot(slot) => frame.values[*slot as usize]
                .as_ref()
                .cloned()
                .ok_or_else(|| {
                    velin_eval::unassigned(line, program.slots.name(*slot).unwrap_or("?"))
                })?,
        };
        stack.push(value);
    }
    let metrics = invoke_stack_measured(call.function, stack, call.operands.len(), line)?;
    let result = stack
        .pop()
        .expect("validated quickened built-in produced a result");
    debug_assert!(stack.is_empty());
    Ok((result, metrics))
}

pub(super) fn eval_quickened_readonly_call(
    program: &Program,
    frame: &FrameState,
    call: QuickenedCallRef<'_>,
) -> Result<Option<(Value, DataMetrics)>, EvalError> {
    if !matches!(
        call.function,
        Builtin::Len | Builtin::Get | Builtin::Contains
    ) {
        return Ok(None);
    }
    let line = call.line as usize;
    let resolve =
        |operand: &QuickenedOperand| resolve_quickened_operand(program, frame, *operand, line);
    let result = match call.operands {
        [first] => {
            let arguments = [resolve(first)?];
            invoke_readonly_measured(call.function, &arguments, line)?
        }
        [first, second] => {
            let arguments = [resolve(first)?, resolve(second)?];
            invoke_readonly_measured(call.function, &arguments, line)?
        }
        [first, second, third] => {
            let arguments = [resolve(first)?, resolve(second)?, resolve(third)?];
            invoke_readonly_measured(call.function, &arguments, line)?
        }
        _ => unreachable!("validated read-only built-in arity"),
    };
    Ok(Some(result))
}

pub(super) fn resolve_quickened_operand<'a>(
    program: &'a Program,
    frame: &'a FrameState,
    operand: QuickenedOperand,
    line: usize,
) -> Result<&'a Value, EvalError> {
    match operand {
        QuickenedOperand::Constant(index) => Ok(&program.constants[index as usize]),
        QuickenedOperand::Slot(slot) => frame.values[slot as usize]
            .as_ref()
            .ok_or_else(|| velin_eval::unassigned(line, program.slots.name(slot).unwrap_or("?"))),
    }
}

pub(super) fn value_metrics(value: &Value, line: usize) -> Result<DataMetrics, EvalError> {
    value
        .data_metrics()
        .map_err(|error| EvalError::new(line, error))
}

pub(super) fn ensure_child_depth(metrics: DataMetrics, line: usize) -> Result<(), EvalError> {
    if metrics.max_depth >= MAX_DATA_DEPTH {
        return Err(EvalError::new(
            line,
            "data exceeds 4096 values or 16 nesting levels",
        ));
    }
    Ok(())
}

pub(super) fn updated_collection_depth(
    current: DataMetrics,
    removed: Option<DataMetrics>,
    added: Option<DataMetrics>,
    result: &Value,
    line: usize,
) -> Result<usize, EvalError> {
    let added_depth = added.map_or(0, |metrics| metrics.max_depth + 1);
    let removed_was_deepest =
        removed.is_some_and(|metrics| metrics.max_depth + 1 == current.max_depth);
    if removed_was_deepest && added_depth < current.max_depth {
        return value_metrics(result, line).map(|metrics| metrics.max_depth);
    }
    Ok(current.max_depth.max(added_depth))
}

pub(super) fn adjusted_footprint(
    current: DataFootprint,
    removed: DataFootprint,
    added: DataFootprint,
    removed_key_bytes: usize,
    added_key_bytes: usize,
    line: usize,
) -> Result<DataFootprint, EvalError> {
    let values = current
        .values
        .checked_sub(removed.values)
        .and_then(|values| values.checked_add(added.values))
        .ok_or_else(|| EvalError::new(line, "data value count overflow"))?;
    if values > MAX_DATA_VALUES {
        return Err(EvalError::new(
            line,
            "data exceeds 4096 values or 16 nesting levels",
        ));
    }
    let removed_text = removed
        .text_bytes
        .checked_add(removed_key_bytes)
        .ok_or_else(|| EvalError::new(line, "data text size overflow"))?;
    let added_text = added
        .text_bytes
        .checked_add(added_key_bytes)
        .ok_or_else(|| EvalError::new(line, "data text size overflow"))?;
    let text_bytes = current
        .text_bytes
        .checked_sub(removed_text)
        .and_then(|bytes| bytes.checked_add(added_text))
        .ok_or_else(|| EvalError::new(line, "data text size overflow"))?;
    if text_bytes > MAX_DATA_TEXT_BYTES {
        return Err(EvalError::new(line, "data text exceeds 1 MiB"));
    }
    Ok(DataFootprint { values, text_bytes })
}

#[inline]
pub(super) fn checked_total(
    current: DataFootprint,
    added: DataFootprint,
    max_values: usize,
    max_text_bytes: usize,
    context: &'static str,
) -> Result<DataFootprint, &'static str> {
    let values = current
        .values
        .checked_add(added.values)
        .ok_or("runtime data value count overflow")?;
    let text_bytes = current
        .text_bytes
        .checked_add(added.text_bytes)
        .ok_or("runtime data text size overflow")?;
    if values > max_values {
        return Err(match context {
            "host payload" => "host payload exceeds 100,000 values",
            _ => "machine state exceeds 100,000 values",
        });
    }
    if text_bytes > max_text_bytes {
        return Err(match context {
            "host payload" => "host payload text exceeds 16 MiB",
            _ => "machine state text exceeds 16 MiB",
        });
    }
    Ok(DataFootprint { values, text_bytes })
}
