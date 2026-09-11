//! Differential test: the bytecode expression evaluator must agree with the
//! `velin-eval` tree-walker on every input, value for value and error for
//! error. This is the safety net that lets the VM replace the tree-walk.

use std::sync::Arc;
use velin_compile::{SlotTable, compile_expression};
use velin_eval::{Variables, evaluate, evaluate_with_rng};
use velin_parse::parse_expression;
use velin_syntax::Value;
use velin_vm::eval_chunk;

/// Evaluates `source` both ways against the same environment and asserts the
/// results (or errors) match.
fn agree(source: &str, vars: &Variables) {
    let expr = parse_expression(source, "diff", 1, 1).expect("parses");

    // Tree-walk reference.
    let walked = evaluate(&expr, vars, 1);

    // Bytecode path: intern the same variables into a frame.
    let mut slots = SlotTable::new();
    let chunk = compile_expression(&expr, &mut slots, 1);
    let slot_count = u32::try_from(slots.len()).expect("slot count fits in u32");
    let mut frame: Vec<Option<Value>> = (0..slot_count)
        .map(|slot| slots.name(slot).and_then(|name| vars.get(name).cloned()))
        .collect();
    let compiled = eval_chunk(&chunk, &mut frame, |slot| {
        slots.name(slot).unwrap_or("?").to_owned()
    });

    match (walked, compiled) {
        (Ok(a), Ok(b)) => assert_eq!(a, b, "value mismatch for `{source}`"),
        (Err(a), Err(b)) => {
            assert_eq!(
                a.message, b.message,
                "error message mismatch for `{source}`"
            );
        }
        (a, b) => panic!("outcome mismatch for `{source}`: walk={a:?} bytecode={b:?}"),
    }
}

/// Evaluates a stateful random expression through both engines from the same
/// seed, asserting both the value/error and the advanced state agree.
fn agree_rng(source: &str, vars: &Variables, seed: i64) {
    let expr = parse_expression(source, "diff", 1, 1).expect("parses");

    let mut walked_state = seed;
    let walked = evaluate_with_rng(&expr, vars, &mut walked_state, 1);

    let mut slots = SlotTable::new();
    let chunk = compile_expression(&expr, &mut slots, 1);
    let rng_slot = slots.rng_state().expect("random expression reserves state");
    let slot_count = u32::try_from(slots.len()).expect("slot count fits in u32");
    let mut frame: Vec<Option<Value>> = (0..slot_count)
        .map(|slot| {
            if slot == rng_slot {
                Some(Value::Integer(seed))
            } else {
                slots.name(slot).and_then(|name| vars.get(name).cloned())
            }
        })
        .collect();
    let compiled = eval_chunk(&chunk, &mut frame, |slot| {
        slots.name(slot).unwrap_or("?").to_owned()
    });

    match (walked, compiled) {
        (Ok(a), Ok(b)) => assert_eq!(a, b, "value mismatch for `{source}`"),
        (Err(a), Err(b)) => assert_eq!(
            a.message, b.message,
            "error message mismatch for `{source}`"
        ),
        (a, b) => panic!("outcome mismatch for `{source}`: walk={a:?} bytecode={b:?}"),
    }
    assert_eq!(
        frame[rng_slot as usize],
        Some(Value::Integer(walked_state)),
        "RNG state mismatch for `{source}`"
    );
}

fn sample_env() -> Variables {
    Variables::from([
        ("hp".into(), Value::Integer(30)),
        ("name".into(), Value::String("Mira".into())),
        ("flag".into(), Value::Boolean(true)),
        (
            "bag".into(),
            Value::List(Arc::new(vec![
                Value::String("key".into()),
                Value::String("map".into()),
            ])),
        ),
    ])
}

#[test]
fn arithmetic_and_precedence_match() {
    let vars = Variables::new();
    for source in [
        "1 + 2 * 3",
        "(1 + 2) * 3",
        "-4 + 10",
        "10 / 3",
        "10 - 2 - 3",
        "2 * 3 + 4 * 5",
    ] {
        agree(source, &vars);
    }
}

#[test]
fn comparisons_and_booleans_match() {
    let vars = sample_env();
    for source in [
        "hp >= 30",
        "hp < 10 or flag",
        "flag and hp == 30",
        "not flag",
        "\"a\" + \"b\" == \"ab\"",
        "hp > 5 and hp < 100 and flag",
    ] {
        agree(source, &vars);
    }
}

#[test]
fn non_boolean_right_operands_match_errors() {
    let vars = Variables::from([("number".into(), Value::Integer(1))]);
    for source in ["true and number", "false or number"] {
        agree(source, &vars);
    }
}

#[test]
fn builtins_match() {
    let vars = sample_env();
    for source in [
        "len(bag)",
        "contains(bag, \"map\")",
        "get(bag, 0)",
        "get(push(bag, \"gem\"), 2)",
        "len(record(\"a\", 1, \"b\", 2))",
    ] {
        agree(source, &vars);
    }
}

#[test]
fn errors_match_value_for_value() {
    let vars = sample_env();
    for source in [
        "1 / 0",        // division by zero
        "hp + name",    // integer + string
        "not hp",       // not on integer
        "missing + 1",  // unassigned variable
        "get(bag, 99)", // list index out of bounds -> missing
        "\"a\" < 1",    // comparison type mismatch
    ] {
        agree(source, &vars);
    }
}

#[test]
fn short_circuit_avoids_unassigned_both_ways() {
    // `missing` is deliberately absent from the environment; both paths must
    // return the left operand without touching it.
    let vars = Variables::from([("flag".into(), Value::Boolean(false))]);
    agree("flag and missing", &vars);
    let vars = Variables::from([("flag".into(), Value::Boolean(true))]);
    agree("flag or missing", &vars);
}

#[test]
fn interpolation_matches_value_for_value() {
    let vars = sample_env();
    for source in [
        r#""hp is [hp] now""#,
        r#""[name] has [hp] hp""#,
        r#""[hp + 1] and [hp * 2]""#,
        r#""bag: [bag], flag: [flag]""#,
        r#""plain, no holes""#,
        r#""escaped [[bracket]] only""#,
    ] {
        agree(source, &vars);
    }
}

#[test]
fn interpolation_errors_match() {
    // An error inside a hole must surface identically on both paths.
    let vars = sample_env();
    agree(r#""value is [hp + name]""#, &vars); // integer + string
    agree(r#""[missing] here""#, &Variables::new()); // unassigned in a hole
}

#[test]
fn random_and_chance_match_value_and_final_state() {
    let vars = sample_env();
    for source in [
        "random(1, 6)",
        "random(-10, 10) + random(20, 30)",
        "chance(25)",
        r#""roll [random(1, 100)], hit [chance(70)]""#,
        "false and chance(50)",
    ] {
        agree_rng(source, &vars, 0x1234_5678);
    }
}

#[test]
fn invalid_random_calls_match_without_consuming_state() {
    let vars = Variables::new();
    for source in [
        "random(9, 1)",
        "random(false, 3)",
        "chance(-1)",
        "chance(101)",
    ] {
        agree_rng(source, &vars, 77);
    }
}
