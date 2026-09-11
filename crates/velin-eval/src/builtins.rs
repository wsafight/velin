//! The closed set of deterministic built-in functions.
//!
//! Built-ins are pure value operations except for deterministic random draws,
//! whose state is supplied explicitly by the evaluator. Every result is
//! validated against the shared data budget in [`Value::validate_data`].

use crate::eval::{EvalError, execution};
use std::collections::BTreeMap;
use std::sync::Arc;
use velin_syntax::{Builtin, Value};

/// Invokes `function` with already-evaluated `arguments`.
///
/// # Errors
/// Returns [`EvalError`] for an invalid argument count, a type mismatch, an
/// out-of-range index or key, or a result that exceeds the data budget.
///
/// # Panics
/// Panics if a well-formed built-in call is internally inconsistent (e.g. a
/// missing argument the arity check should have rejected); unreachable for
/// calls that passed [`Builtin::accepts`].
pub fn invoke(
    function: Builtin,
    mut arguments: Vec<Value>,
    line: usize,
) -> Result<Value, EvalError> {
    if !function.accepts(arguments.len()) {
        return Err(execution(line, "invalid built-in argument count"));
    }
    let result = match function {
        Builtin::List => Value::List(Arc::new(arguments)),
        Builtin::Record => {
            let mut record = BTreeMap::new();
            let mut arguments = arguments.into_iter();
            while let Some(key) = arguments.next() {
                let Value::String(key) = key else {
                    return Err(execution(line, "record keys must be strings"));
                };
                if record.insert(key, arguments.next().unwrap()).is_some() {
                    return Err(execution(line, "duplicate record key"));
                }
            }
            Value::Record(Arc::new(record))
        }
        Builtin::Len => Value::Integer(
            i64::try_from(match &arguments[0] {
                Value::List(values) => values.len(),
                Value::Record(values) => values.len(),
                Value::String(value) => value.chars().count(),
                _ => return Err(execution(line, "len expects a list, record or string")),
            })
            .map_err(|_| execution(line, "length overflow"))?,
        ),
        Builtin::Get => {
            let value = match (&arguments[0], &arguments[1]) {
                (Value::List(values), Value::Integer(index)) => usize::try_from(*index)
                    .ok()
                    .and_then(|index| values.get(index)),
                (Value::Record(values), Value::String(key)) => values.get(key),
                _ => {
                    return Err(execution(
                        line,
                        "get expects a list and integer index, or a record and string key",
                    ));
                }
            };
            value
                .or(arguments.get(2))
                .cloned()
                .ok_or_else(|| execution(line, "missing key or list index"))?
        }
        Builtin::Contains => Value::Boolean(match (&arguments[0], &arguments[1]) {
            (Value::List(values), value) => values.contains(value),
            (Value::Record(values), Value::String(key)) => values.contains_key(key),
            (Value::String(text), Value::String(part)) => text.contains(part),
            _ => return Err(execution(line, "contains expects a list, record or string")),
        }),
        Builtin::Push => {
            let value = arguments.pop().unwrap();
            let Value::List(mut values) = arguments.pop().unwrap() else {
                return Err(execution(line, "push expects a list"));
            };
            Arc::make_mut(&mut values).push(value);
            Value::List(values)
        }
        Builtin::Put | Builtin::Remove => edit(function, arguments, line)?,
        Builtin::Random | Builtin::Chance => {
            return Err(execution(
                line,
                "random functions require a threaded RNG state",
            ));
        }
    };
    result
        .validate_data()
        .map_err(|error| execution(line, error))?;
    Ok(result)
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

fn edit(function: Builtin, mut arguments: Vec<Value>, line: usize) -> Result<Value, EvalError> {
    let replacement = (function == Builtin::Put).then(|| arguments.pop().unwrap());
    let key = arguments.pop().unwrap();
    match (arguments.pop().unwrap(), key) {
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
                Arc::make_mut(&mut values).insert(key, value);
            } else if Arc::make_mut(&mut values).remove(&key).is_none() {
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
mod tests {
    use super::*;

    #[test]
    fn list_and_record_construct_and_read_back() {
        let list = invoke(Builtin::List, vec![Value::Integer(1), Value::Integer(2)], 1).unwrap();
        assert_eq!(
            invoke(Builtin::Len, vec![list.clone()], 1).unwrap(),
            Value::Integer(2)
        );
        let first = invoke(Builtin::Get, vec![list, Value::Integer(0)], 1).unwrap();
        assert_eq!(first, Value::Integer(1));

        let record = invoke(
            Builtin::Record,
            vec![Value::String("done".into()), Value::Boolean(true)],
            1,
        )
        .unwrap();
        let got = invoke(Builtin::Get, vec![record, Value::String("done".into())], 1).unwrap();
        assert_eq!(got, Value::Boolean(true));
    }

    #[test]
    fn push_put_and_remove_are_copy_on_write() {
        let list = Value::List(Arc::new(vec![Value::String("key".into())]));
        let pushed = invoke(
            Builtin::Push,
            vec![list.clone(), Value::String("map".into())],
            1,
        )
        .unwrap();
        // Original is untouched (structural sharing).
        assert_eq!(
            invoke(Builtin::Len, vec![list], 1).unwrap(),
            Value::Integer(1)
        );
        assert_eq!(
            invoke(Builtin::Len, vec![pushed.clone()], 1).unwrap(),
            Value::Integer(2)
        );
        let removed = invoke(Builtin::Remove, vec![pushed, Value::Integer(0)], 1).unwrap();
        assert_eq!(
            invoke(Builtin::Len, vec![removed], 1).unwrap(),
            Value::Integer(1)
        );
    }

    #[test]
    fn invalid_calls_and_collection_growth_are_bounded() {
        assert!(
            invoke(
                Builtin::Get,
                vec![Value::List(Arc::new(vec![])), Value::Integer(-1)],
                1
            )
            .is_err()
        );
        let list = Value::List(Arc::new(vec![Value::Integer(0); 4095]));
        assert!(invoke(Builtin::Push, vec![list, Value::Integer(1)], 1).is_err());
        assert!(
            invoke(
                Builtin::Record,
                vec![Value::Integer(1), Value::Boolean(true)],
                1
            )
            .is_err()
        );
    }

    #[test]
    fn random_calls_are_deterministic_bounded_and_advance_state() {
        let mut first_state = 7;
        let mut second_state = 7;
        let mut first = Vec::new();
        let mut second = Vec::new();
        for _ in 0..32 {
            first.push(
                invoke_random(
                    Builtin::Random,
                    &[Value::Integer(-3), Value::Integer(4)],
                    &mut first_state,
                    1,
                )
                .unwrap(),
            );
            second.push(
                invoke_random(
                    Builtin::Random,
                    &[Value::Integer(-3), Value::Integer(4)],
                    &mut second_state,
                    1,
                )
                .unwrap(),
            );
        }
        assert_eq!(first, second);
        assert_eq!(first_state, second_state);
        assert!(
            first
                .iter()
                .all(|value| matches!(value, Value::Integer(-3..=4)))
        );
        assert_ne!(first_state, 7);
    }

    #[test]
    fn invalid_random_calls_do_not_advance_state() {
        let mut state = 11;
        assert!(
            invoke_random(
                Builtin::Random,
                &[Value::Integer(3), Value::Integer(2)],
                &mut state,
                1,
            )
            .is_err()
        );
        assert_eq!(state, 11);
        assert!(invoke_random(Builtin::Chance, &[Value::Integer(101)], &mut state, 1,).is_err());
        assert_eq!(state, 11);
    }

    #[test]
    fn random_supports_full_i64_range_and_chance_boundaries() {
        let mut state = i64::MIN;
        let value = invoke_random(
            Builtin::Random,
            &[Value::Integer(i64::MIN), Value::Integer(i64::MAX)],
            &mut state,
            1,
        )
        .unwrap();
        assert!(matches!(value, Value::Integer(_)));
        assert_ne!(state, i64::MIN);

        assert_eq!(
            invoke_random(Builtin::Chance, &[Value::Integer(0)], &mut state, 1,).unwrap(),
            Value::Boolean(false)
        );
        assert_eq!(
            invoke_random(Builtin::Chance, &[Value::Integer(100)], &mut state, 1,).unwrap(),
            Value::Boolean(true)
        );
    }

    #[test]
    fn len_get_contains_and_duplicate_keys_error() {
        assert!(invoke(Builtin::Len, Vec::new(), 1).is_err());
        assert!(invoke(Builtin::Len, vec![Value::Integer(1)], 1).is_err());
        assert_eq!(
            invoke(Builtin::Len, vec![Value::String("ab".into())], 1).unwrap(),
            Value::Integer(2)
        );
        assert!(
            invoke(
                Builtin::Record,
                vec![
                    Value::String("a".into()),
                    Value::Integer(1),
                    Value::String("a".into()),
                    Value::Integer(2),
                ],
                1
            )
            .is_err()
        );

        let list = Value::List(Arc::new(vec![Value::Integer(1)]));
        let record = invoke(
            Builtin::Record,
            vec![Value::String("k".into()), Value::Integer(1)],
            1,
        )
        .unwrap();
        assert!(invoke(Builtin::Get, vec![Value::Integer(1), Value::Integer(0)], 1).is_err());
        assert_eq!(
            invoke(
                Builtin::Get,
                vec![record.clone(), Value::String("k".into())],
                1
            )
            .unwrap(),
            Value::Integer(1)
        );
        assert!(
            invoke(
                Builtin::Contains,
                vec![Value::Integer(1), Value::Integer(0)],
                1
            )
            .is_err()
        );
        assert_eq!(
            invoke(
                Builtin::Contains,
                vec![record, Value::String("k".into())],
                1
            )
            .unwrap(),
            Value::Boolean(true)
        );
        assert_eq!(
            invoke(
                Builtin::Contains,
                vec![Value::String("abc".into()), Value::String("b".into())],
                1
            )
            .unwrap(),
            Value::Boolean(true)
        );
        assert_eq!(
            invoke(Builtin::Contains, vec![list, Value::Integer(1)], 1).unwrap(),
            Value::Boolean(true)
        );
        assert!(invoke(Builtin::Push, vec![Value::Integer(1), Value::Integer(2)], 1).is_err());
        assert!(
            invoke(
                Builtin::Random,
                vec![Value::Integer(1), Value::Integer(2)],
                1
            )
            .is_err()
        );
    }

    #[test]
    fn record_edits_and_random_arity_errors() {
        let record = invoke(
            Builtin::Record,
            vec![Value::String("k".into()), Value::Integer(1)],
            1,
        )
        .unwrap();
        let updated = invoke(
            Builtin::Put,
            vec![record.clone(), Value::String("k".into()), Value::Integer(9)],
            1,
        )
        .unwrap();
        assert_eq!(
            invoke(
                Builtin::Get,
                vec![updated.clone(), Value::String("k".into())],
                1
            )
            .unwrap(),
            Value::Integer(9)
        );
        let removed = invoke(Builtin::Remove, vec![updated, Value::String("k".into())], 1).unwrap();
        assert!(invoke(Builtin::Remove, vec![removed, Value::String("k".into())], 1).is_err());
        assert!(
            invoke(
                Builtin::Put,
                vec![Value::Integer(1), Value::Integer(0), Value::Integer(2)],
                1
            )
            .is_err()
        );

        let mut state = 1;
        assert!(invoke_random(Builtin::Random, &[], &mut state, 1).is_err());
        assert!(
            invoke_random(
                Builtin::Random,
                &[Value::Boolean(true), Value::Integer(1)],
                &mut state,
                1
            )
            .is_err()
        );
        assert!(invoke_random(Builtin::Chance, &[], &mut state, 1).is_err());
        assert!(invoke_random(Builtin::Chance, &[Value::Boolean(true)], &mut state, 1).is_err());
        assert!(invoke_random(Builtin::Len, &[Value::Integer(1)], &mut state, 1).is_err());
    }
}
