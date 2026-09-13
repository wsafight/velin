//! The closed set of deterministic built-in functions.
//!
//! Built-ins are pure value operations except for deterministic random draws,
//! whose state is supplied explicitly by the evaluator. Every result is
//! validated against the shared data budget in [`Value::validate_data`].

use crate::eval::{EvalError, execution};
use std::collections::BTreeMap;
use std::sync::Arc;
use velin_syntax::{Builtin, DataMetrics, Value};

mod metrics;
use metrics::{collection_metrics, push_metrics, record_metrics, scalar_metrics};

/// Invokes `function` with already-evaluated `arguments`.
///
/// # Errors
/// Returns [`EvalError`] for an invalid argument count, a type mismatch, an
/// out-of-range index or key, or a result that exceeds the data budget.
pub fn invoke(function: Builtin, arguments: Vec<Value>, line: usize) -> Result<Value, EvalError> {
    invoke_measured(function, arguments, line).map(|(value, _)| value)
}

/// Invokes a built-in and returns the resource metrics produced while
/// validating its result.
///
/// The bytecode VM retains these metrics through the end of an expression so
/// collection results are not traversed a second time before assignment.
///
/// # Errors
/// Returns the same failures as [`invoke`].
pub fn invoke_measured(
    function: Builtin,
    mut arguments: Vec<Value>,
    line: usize,
) -> Result<(Value, DataMetrics), EvalError> {
    let argc = arguments.len();
    let metrics = invoke_stack_measured(function, &mut arguments, argc, line)?;
    let result = arguments
        .pop()
        .ok_or_else(|| execution(line, "built-in produced no result"))?;
    debug_assert!(arguments.is_empty());
    Ok((result, metrics))
}

/// Consumes `argc` values from the end of `stack`, pushes one result, and
/// returns its validated metrics.
///
/// Fixed-arity built-ins pop their operands directly, avoiding a temporary
/// argument vector in the bytecode VM. `list` reuses or creates exactly the
/// vector needed for the resulting collection.
///
/// # Errors
/// Returns the same failures as [`invoke`], plus an invalid argument-count
/// error if `stack` does not contain all declared operands.
pub fn invoke_stack_measured(
    function: Builtin,
    stack: &mut Vec<Value>,
    argc: usize,
    line: usize,
) -> Result<DataMetrics, EvalError> {
    if !function.accepts(argc) || stack.len() < argc {
        return Err(execution(line, "invalid built-in argument count"));
    }
    let result = invoke_stack(function, stack, argc, line)?;
    let metrics = result
        .data_metrics()
        .map_err(|error| execution(line, error))?;
    stack.push(result);
    Ok(metrics)
}

/// Invokes a built-in while reusing metrics that already belong to its
/// operands. The value stack and metrics stack must contain the same number of
/// entries before the call. Collection constructors and `push` can therefore
/// avoid walking their freshly-created result a second time.
///
/// Operations whose result shape depends on searching or replacing an
/// arbitrary child use the regular measurement fallback. This keeps the
/// optimization local and leaves all value semantics in [`invoke_stack`].
///
/// # Errors
/// Returns the same failures as [`invoke_stack_measured`].
pub fn invoke_stack_measured_with_metrics(
    function: Builtin,
    stack: &mut Vec<Value>,
    metrics: &mut Vec<DataMetrics>,
    argc: usize,
    line: usize,
) -> Result<DataMetrics, EvalError> {
    if !function.accepts(argc) || stack.len() < argc || metrics.len() < argc {
        return Err(execution(line, "invalid built-in argument count"));
    }
    let at = stack.len() - argc;
    let known_metrics = match function {
        Builtin::List => Some(collection_metrics(&metrics[at..], line)),
        Builtin::Record => record_metrics(&stack[at..], &metrics[at..], line),
        Builtin::Push => push_metrics(&stack[at..], &metrics[at..], line),
        Builtin::Len | Builtin::Contains => Some(Ok(scalar_metrics())),
        Builtin::Get | Builtin::Put | Builtin::Remove | Builtin::Random | Builtin::Chance => None,
    };
    let result = invoke_stack(function, stack, argc, line)?;
    metrics.truncate(at);
    let result_metrics = if let Some(known) = known_metrics.transpose()? {
        known
    } else {
        result
            .data_metrics()
            .map_err(|error| execution(line, error))?
    };
    stack.push(result);
    metrics.push(result_metrics);
    Ok(result_metrics)
}

fn invoke_stack(
    function: Builtin,
    stack: &mut Vec<Value>,
    argc: usize,
    line: usize,
) -> Result<Value, EvalError> {
    let result = match function {
        Builtin::List => construct_list(stack, argc),
        Builtin::Record => construct_record(stack, argc, line)?,
        Builtin::Len | Builtin::Get | Builtin::Contains => {
            let at = stack.len() - argc;
            let result = invoke_readonly(function, argc, |index| &stack[at + index], line);
            stack.truncate(at);
            result?
        }
        Builtin::Push => {
            let value = pop_argument(stack, line)?;
            let source = pop_argument(stack, line)?;
            let Value::List(mut values) = source else {
                return Err(execution(line, "push expects a list"));
            };
            Arc::make_mut(&mut values).push(value);
            Value::List(values)
        }
        Builtin::Put => {
            let replacement = pop_argument(stack, line)?;
            let key = pop_argument(stack, line)?;
            let source = pop_argument(stack, line)?;
            edit(source, key, Some(replacement), line)?
        }
        Builtin::Remove => {
            let key = pop_argument(stack, line)?;
            let source = pop_argument(stack, line)?;
            edit(source, key, None, line)?
        }
        Builtin::Random | Builtin::Chance => {
            return Err(execution(
                line,
                "random functions require a threaded RNG state",
            ));
        }
    };
    Ok(result)
}

/// Invokes a built-in that only reads its arguments without cloning them.
///
/// This accepts `len`, `get`, and `contains`; collection constructors and
/// mutations require owned operands and remain on [`invoke_stack_measured`].
///
/// # Errors
/// Returns the same failures as [`invoke`] and rejects non-read-only built-ins.
pub fn invoke_readonly_measured(
    function: Builtin,
    arguments: &[&Value],
    line: usize,
) -> Result<(Value, DataMetrics), EvalError> {
    if !function.accepts(arguments.len()) {
        return Err(execution(line, "invalid built-in argument count"));
    }
    let result = invoke_readonly(function, arguments.len(), |index| arguments[index], line)?;
    let metrics = result
        .data_metrics()
        .map_err(|error| execution(line, error))?;
    Ok((result, metrics))
}

fn invoke_readonly<'a>(
    function: Builtin,
    argc: usize,
    argument: impl Fn(usize) -> &'a Value,
    line: usize,
) -> Result<Value, EvalError> {
    let result = match function {
        Builtin::Len => Value::Integer(
            i64::try_from(match argument(0) {
                Value::List(values) => values.len(),
                Value::Record(values) => values.len(),
                Value::String(value) => value.chars().count(),
                _ => return Err(execution(line, "len expects a list, record or string")),
            })
            .map_err(|_| execution(line, "length overflow"))?,
        ),
        Builtin::Get => {
            let source = argument(0);
            let key = argument(1);
            let value = match (source, key) {
                (Value::List(values), Value::Integer(index)) => usize::try_from(*index)
                    .ok()
                    .and_then(|index| values.get(index)),
                (Value::Record(values), Value::String(key)) => values.get(key.as_str()),
                _ => {
                    return Err(execution(
                        line,
                        "get expects a list and integer index, or a record and string key",
                    ));
                }
            };
            value
                .or_else(|| (argc == 3).then(|| argument(2)))
                .cloned()
                .ok_or_else(|| execution(line, "missing key or list index"))?
        }
        Builtin::Contains => {
            let source = argument(0);
            let value = argument(1);
            Value::Boolean(match (source, value) {
                (Value::List(values), value) => values.contains(value),
                (Value::Record(values), Value::String(key)) => values.contains_key(key.as_str()),
                (Value::String(text), Value::String(part)) => text.contains(part.as_str()),
                _ => return Err(execution(line, "contains expects a list, record or string")),
            })
        }
        _ => return Err(execution(line, "built-in requires owned arguments")),
    };
    Ok(result)
}

fn construct_list(stack: &mut Vec<Value>, argc: usize) -> Value {
    let at = stack.len() - argc;
    let values = if at == 0 {
        std::mem::take(stack)
    } else {
        stack.drain(at..).collect()
    };
    Value::List(Arc::new(values))
}

fn construct_record(stack: &mut Vec<Value>, argc: usize, line: usize) -> Result<Value, EvalError> {
    let mut record = BTreeMap::new();
    for _ in 0..argc / 2 {
        let value = pop_argument(stack, line)?;
        let key = pop_argument(stack, line)?;
        let Value::String(key) = key else {
            return Err(execution(line, "record keys must be strings"));
        };
        if record.insert(key.into_string(), value).is_some() {
            return Err(execution(line, "duplicate record key"));
        }
    }
    Ok(Value::Record(Arc::new(record)))
}

fn pop_argument(stack: &mut Vec<Value>, line: usize) -> Result<Value, EvalError> {
    stack
        .pop()
        .ok_or_else(|| execution(line, "invalid built-in argument count"))
}

/// Invokes one of the stateful random built-ins and advances `state`.
///
/// The generator is `SplitMix64` and uses integer operations only. Its state is
/// represented as an `i64` so the VM can keep it in an ordinary Velin integer
/// slot; conversion to `u64` preserves all bits. Invalid arguments are rejected
/// before the state advances.
///
/// # Errors
/// Returns [`EvalError`] when `function` is not random/chance, the arity or
/// argument types are wrong, `random` has an inverted range, or `chance` is
/// outside `0..=100`.
pub fn invoke_random(
    function: Builtin,
    arguments: &[Value],
    state: &mut i64,
    line: usize,
) -> Result<Value, EvalError> {
    if !function.accepts(arguments.len()) {
        return Err(execution(line, "invalid built-in argument count"));
    }
    match function {
        Builtin::Random => {
            let [low, high] = arguments else {
                unreachable!("random arity checked above");
            };
            let (Value::Integer(low), Value::Integer(high)) = (low, high) else {
                return Err(execution(line, "random expects two integers"));
            };
            if low > high {
                return Err(execution(
                    line,
                    "random lower bound must not exceed upper bound",
                ));
            }
            Ok(Value::Integer(random_inclusive(state, *low, *high)))
        }
        Builtin::Chance => {
            let [percent] = arguments else {
                unreachable!("chance arity checked above");
            };
            let Value::Integer(percent) = percent else {
                return Err(execution(line, "chance expects an integer percentage"));
            };
            if !(0..=100).contains(percent) {
                return Err(execution(
                    line,
                    "chance percentage must be between 0 and 100",
                ));
            }
            let draw = uniform_below(state, 100);
            Ok(Value::Boolean(draw < percent.cast_unsigned()))
        }
        _ => Err(execution(line, "built-in is not a random function")),
    }
}

/// Draws uniformly from the inclusive `i64` interval, including the full
/// `i64::MIN..=i64::MAX` domain.
fn random_inclusive(state: &mut i64, low: i64, high: i64) -> i64 {
    let width = (i128::from(high) - i128::from(low) + 1).cast_unsigned();
    let offset = if width == (1_u128 << 64) {
        u128::from(next_u64(state))
    } else {
        u128::from(uniform_below(
            state,
            u64::try_from(width).expect("non-full i64 interval width fits u64"),
        ))
    };
    let offset = i128::try_from(offset).expect("i64 interval offset fits i128");
    i64::try_from(i128::from(low) + offset).expect("draw stays inside i64 bounds")
}

/// Lemire's multiply-high mapping with rejection, avoiding modulo bias.
fn uniform_below(state: &mut i64, bound: u64) -> u64 {
    debug_assert!(bound > 0);
    let threshold = bound.wrapping_neg() % bound;
    loop {
        let product = u128::from(next_u64(state)) * u128::from(bound);
        let low = u64::try_from(product & u128::from(u64::MAX))
            .expect("masked product low half fits u64");
        if low >= threshold {
            return u64::try_from(product >> 64).expect("product high half fits u64");
        }
    }
}

/// Advances `SplitMix64` and returns its mixed output.
fn next_u64(state: &mut i64) -> u64 {
    let next = state.cast_unsigned().wrapping_add(0x9E37_79B9_7F4A_7C15);
    *state = next.cast_signed();
    let mut value = next;
    value = (value ^ (value >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    value ^ (value >> 31)
}

fn edit(
    source: Value,
    key: Value,
    replacement: Option<Value>,
    line: usize,
) -> Result<Value, EvalError> {
    match (source, key) {
        (Value::List(mut values), Value::Integer(index)) => {
            let index = usize::try_from(index)
                .ok()
                .filter(|index| *index < values.len())
                .ok_or_else(|| execution(line, "list index out of bounds"))?;
            if let Some(value) = replacement {
                Arc::make_mut(&mut values)[index] = value;
            } else {
                Arc::make_mut(&mut values).remove(index);
            }
            Ok(Value::List(values))
        }
        (Value::Record(mut values), Value::String(key)) => {
            if let Some(value) = replacement {
                Arc::make_mut(&mut values).insert(key.into_string(), value);
            } else if Arc::make_mut(&mut values).remove(key.as_str()).is_none() {
                return Err(execution(line, "missing record key"));
            }
            Ok(Value::Record(values))
        }
        _ => Err(execution(
            line,
            "put/remove expects a list and integer index, or a record and string key",
        )),
    }
}

#[cfg(test)]
#[path = "builtins_tests.rs"]
mod tests;
