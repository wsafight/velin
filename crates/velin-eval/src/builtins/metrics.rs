use crate::eval::execution;
use velin_syntax::{
    DataFootprint, DataMetrics, MAX_DATA_DEPTH, MAX_DATA_TEXT_BYTES, MAX_DATA_VALUES, Value,
};

pub(super) fn scalar_metrics() -> DataMetrics {
    DataMetrics {
        footprint: DataFootprint {
            values: 1,
            text_bytes: 0,
        },
        max_depth: 0,
    }
}

pub(super) fn collection_metrics(
    children: &[DataMetrics],
    line: usize,
) -> Result<DataMetrics, crate::eval::EvalError> {
    let mut footprint = DataFootprint {
        values: 1,
        text_bytes: 0,
    };
    let mut max_depth = 0;
    for child in children {
        footprint.values = footprint
            .values
            .checked_add(child.footprint.values)
            .ok_or_else(|| execution(line, "data value count overflow"))?;
        footprint.text_bytes = footprint
            .text_bytes
            .checked_add(child.footprint.text_bytes)
            .ok_or_else(|| execution(line, "data text size overflow"))?;
        max_depth = max_depth.max(child.max_depth + 1);
    }
    validate_metrics(
        DataMetrics {
            footprint,
            max_depth,
        },
        line,
    )
}

pub(super) fn record_metrics(
    values: &[Value],
    metrics: &[DataMetrics],
    line: usize,
) -> Option<Result<DataMetrics, crate::eval::EvalError>> {
    if !values.len().is_multiple_of(2) {
        return None;
    }
    let mut result = DataMetrics {
        footprint: DataFootprint {
            values: 1,
            text_bytes: 0,
        },
        max_depth: 0,
    };
    let (value_pairs, value_remainder) = values.as_chunks::<2>();
    let (metric_pairs, metric_remainder) = metrics.as_chunks::<2>();
    if !value_remainder.is_empty() || !metric_remainder.is_empty() {
        return None;
    }
    for ([key, _], [_, value]) in value_pairs.iter().zip(metric_pairs) {
        let Value::String(key) = key else {
            return None;
        };
        result.footprint.values = result
            .footprint
            .values
            .checked_add(value.footprint.values)?;
        result.footprint.text_bytes = result
            .footprint
            .text_bytes
            .checked_add(key.len())
            .and_then(|bytes| bytes.checked_add(value.footprint.text_bytes))?;
        result.max_depth = result.max_depth.max(value.max_depth + 1);
    }
    Some(validate_metrics(result, line))
}

pub(super) fn push_metrics(
    values: &[Value],
    metrics: &[DataMetrics],
    line: usize,
) -> Option<Result<DataMetrics, crate::eval::EvalError>> {
    let [Value::List(_), _] = values else {
        return None;
    };
    let [source, value] = metrics else {
        return None;
    };
    Some(validate_metrics(
        DataMetrics {
            footprint: DataFootprint {
                values: source
                    .footprint
                    .values
                    .checked_add(value.footprint.values)?,
                text_bytes: source
                    .footprint
                    .text_bytes
                    .checked_add(value.footprint.text_bytes)?,
            },
            max_depth: source.max_depth.max(value.max_depth + 1),
        },
        line,
    ))
}

fn validate_metrics(
    metrics: DataMetrics,
    line: usize,
) -> Result<DataMetrics, crate::eval::EvalError> {
    if metrics.footprint.values > MAX_DATA_VALUES || metrics.max_depth > MAX_DATA_DEPTH {
        return Err(execution(
            line,
            "data exceeds 4096 values or 16 nesting levels",
        ));
    }
    if metrics.footprint.text_bytes > MAX_DATA_TEXT_BYTES {
        return Err(execution(line, "data text exceeds 1 MiB"));
    }
    Ok(metrics)
}
