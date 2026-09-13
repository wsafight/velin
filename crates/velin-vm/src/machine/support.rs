use super::{
    DataFootprint, DataMetrics, EvalError, ExecutionMetadata, FrameAccess, FrameState, HostOp,
    MAX_DATA_DEPTH, MAX_DATA_TEXT_BYTES, MAX_DATA_VALUES, MAX_HOST_PAYLOAD_TEXT_BYTES,
    MAX_HOST_PAYLOAD_VALUES, Program, Value, eval_validated_chunk,
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
    register_values: &mut Vec<Option<Value>>,
    register_metrics: &mut Vec<Option<DataMetrics>>,
    chunk_id: u32,
) -> Result<(Value, DataMetrics), EvalError> {
    let (execution, _) = metadata
        .chunk_with_constant_metrics(chunk_id)
        .expect("validated chunk metadata");
    let chunk = program
        .chunk(chunk_id)
        .expect("program expression range was validated");
    let constant_metrics = Some((
        metadata.program_constant_metrics(),
        program.chunks[chunk_id as usize].constants.start,
    ));
    let (frame, frame_metrics) = if execution.mutates_frame {
        let frame_metrics = Some((frame.footprints.as_slice(), frame.depths.as_slice()));
        (
            FrameAccess::Mutable(frame.values.as_mut_slice()),
            frame_metrics,
        )
    } else {
        let frame_metrics = Some((frame.footprints.as_slice(), frame.depths.as_slice()));
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
        constant_metrics,
        register_values,
        register_metrics,
        |slot| slots.name(slot).unwrap_or("?").to_owned(),
    )
}

pub(super) fn eval_host_args(
    program: &Program,
    metadata: &ExecutionMetadata,
    frame: &mut FrameState,
    register_values: &mut Vec<Option<Value>>,
    register_metrics: &mut Vec<Option<DataMetrics>>,
    host: &HostOp,
) -> Result<Vec<Value>, EvalError> {
    let line = host.line as usize;
    let mut values = Vec::with_capacity(host.args.len());
    let mut payload = DataFootprint::default();
    for chunk in host.args.iter().copied() {
        let (value, metrics) = eval_chunk_for(
            program,
            metadata,
            frame,
            register_values,
            register_metrics,
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
    Ok(values)
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
