//! End-to-end pipeline microbenchmarks.
//!
//! These are intentionally small and deterministic: they measure the cost of
//! each stage (parse, check, compile, run) on representative expressions so
//! regressions show up as relative shifts, not absolute wall-clock claims.

use criterion::{Criterion, criterion_group, criterion_main};
use std::fmt::Write as _;
use std::hint::black_box;
use velin::{
    Environment, Expr, Machine, Op, ProgramBuilder, ScriptRunner, SlotTable, Value, Variables,
    Yield, check_expression, check_script, compile, compile_expression, evaluate, parse_expression,
    parse_program,
};

#[path = "pipeline/p1.rs"]
mod pipeline_p1;
#[path = "pipeline/p2.rs"]
mod pipeline_p2;

/// A non-trivial arithmetic/boolean guard, the shape a real script branches on.
const GUARD: &str = "hp - 10 > 0 and (level * 2 + 5) <= 100 or defeated == false";

fn bench_parse(c: &mut Criterion) {
    c.bench_function("parse/guard", |b| {
        b.iter(|| parse_expression(black_box(GUARD), "bench", 1, 1).unwrap());
    });
}

fn bench_parse_wide_script(c: &mut Criterion) {
    let source = wide_linear_source(512);
    c.bench_function("parse/wide_linear_script", |b| {
        b.iter(|| parse_program(black_box(&source)).unwrap());
    });
}

fn bench_check(c: &mut Criterion) {
    let expr = parse_expression(GUARD, "bench", 1, 1).unwrap();
    let mut env = Environment::new();
    env.insert("hp".into(), velin::Type::Integer);
    env.insert("level".into(), velin::Type::Integer);
    env.insert("defeated".into(), velin::Type::Boolean);
    c.bench_function("check/guard", |b| {
        b.iter(|| check_expression(black_box(&expr), &env, "bench", 1, 1));
    });
}

fn bench_check_wide_script(c: &mut Criterion) {
    let source = wide_linear_source(512);
    let script = compile("bench.velin", &source).unwrap();
    c.bench_function("check/wide_linear_script", |b| {
        b.iter(|| check_script("bench.velin", black_box(&script)));
    });
}

fn bench_check_expression_heavy_script(c: &mut Criterion) {
    let source = expression_heavy_source(512);
    let script = compile("bench.velin", &source).unwrap();
    c.bench_function("check/expression_heavy_script", |b| {
        b.iter(|| check_script("bench.velin", black_box(&script)));
    });
}

fn bench_check_builtin_heavy_script(c: &mut Criterion) {
    let source = builtin_heavy_source(512);
    let script = compile("bench.velin", &source).unwrap();
    c.bench_function("check/builtin_heavy_script", |b| {
        b.iter(|| check_script("bench.velin", black_box(&script)));
    });
}

fn bench_check_short_circuit_heavy_script(c: &mut Criterion) {
    let source = short_circuit_heavy_source(512);
    let script = compile("bench.velin", &source).unwrap();
    c.bench_function("check/short_circuit_heavy_script", |b| {
        b.iter(|| check_script("bench.velin", black_box(&script)));
    });
}

fn wide_linear_source(variables: usize) -> String {
    let mut source = String::new();
    for index in 0..variables {
        writeln!(source, "default value_{index} = 0").expect("writing to a string cannot fail");
    }
    for index in 0..variables {
        writeln!(source, "set value_{index} = value_{index} + 1")
            .expect("writing to a string cannot fail");
    }
    source
}

fn expression_heavy_source(expressions: usize) -> String {
    let mut source = String::from("default source = 1\n");
    for index in 0..expressions {
        writeln!(source, "set value_{index} = source * 2 + {index}")
            .expect("writing to a string cannot fail");
    }
    source
}

fn builtin_heavy_source(expressions: usize) -> String {
    let mut source = String::from("default source = 1\n");
    for index in 0..expressions {
        writeln!(
            source,
            "set value_{index} = len(list(source, {index}, source + 1))"
        )
        .expect("writing to a string cannot fail");
    }
    source
}

fn short_circuit_heavy_source(expressions: usize) -> String {
    let mut source = String::from("default flag = true\n");
    for index in 0..expressions {
        writeln!(
            source,
            "set value_{index} = flag and flag or flag and flag or flag"
        )
        .expect("writing to a string cannot fail");
    }
    source
}

fn host_call_source(calls: usize) -> String {
    let mut source = String::new();
    for index in 0..calls {
        writeln!(source, "perform emit({index}, {index} + 1)")
            .expect("writing to a string cannot fail");
    }
    source
}

fn bench_parse_host_calls(c: &mut Criterion) {
    let source = host_call_source(512);
    c.bench_function("parse/host_calls", |b| {
        b.iter(|| parse_program(black_box(&source)).unwrap());
    });
}

fn bench_compile(c: &mut Criterion) {
    let expr = parse_expression(GUARD, "bench", 1, 1).unwrap();
    c.bench_function("compile/guard", |b| {
        b.iter(|| {
            let mut slots = SlotTable::new();
            compile_expression(black_box(&expr), &mut slots, 1)
        });
    });
}

fn bench_compile_wide_script(c: &mut Criterion) {
    let source = wide_linear_source(512);
    c.bench_function("compile/wide_linear_script", |b| {
        b.iter(|| compile("bench.velin", black_box(&source)).unwrap());
    });
}

fn bench_compile_expression_heavy_script(c: &mut Criterion) {
    let source = expression_heavy_source(512);
    c.bench_function("compile/expression_heavy_script", |b| {
        b.iter(|| compile("bench.velin", black_box(&source)).unwrap());
    });
}

fn bench_constant_folding(c: &mut Criterion) {
    let folded = compile(
        "bench.velin",
        concat!(
            "default result = 0\n",
            "default index = 0\n",
            "while index < 1500:\n",
            "    set result = (1 + 2) * (3 + 4) - (8 / 2)\n",
            "    set index = index + 1\n",
        ),
    )
    .unwrap();
    let dynamic = compile(
        "bench.velin",
        concat!(
            "default seed = 1\n",
            "default result = 0\n",
            "default index = 0\n",
            "while index < 1500:\n",
            "    set result = (seed + 2) * (3 + 4) - (8 / 2)\n",
            "    set index = index + 1\n",
        ),
    )
    .unwrap();
    let mut group = c.benchmark_group("vm/constant_folding");
    group.bench_function("folded", |b| {
        b.iter(|| {
            let mut runner = ScriptRunner::new(&folded).unwrap();
            black_box(runner.run().unwrap())
        });
    });
    group.bench_function("runtime_expression", |b| {
        b.iter(|| {
            let mut runner = ScriptRunner::new(&dynamic).unwrap();
            black_box(runner.run().unwrap())
        });
    });
    group.finish();
}

fn bench_wide_linear_run(c: &mut Criterion) {
    let source = wide_linear_source(512);
    let script = compile("bench.velin", &source).unwrap();
    c.bench_function("vm/wide_linear_script", |b| {
        b.iter(|| {
            let mut runner = ScriptRunner::new(&script).unwrap();
            black_box(runner.run().unwrap())
        });
    });
}

fn bench_growing_list(c: &mut Criterion) {
    let script = compile(
        "bench.velin",
        concat!(
            "default items = list()\n",
            "default index = 0\n",
            "while index < 2000:\n",
            "    set items = push(items, index)\n",
            "    set index = index + 1\n",
        ),
    )
    .unwrap();
    c.bench_function("vm/growing_list", |b| {
        b.iter(|| {
            let mut runner = ScriptRunner::new(&script).unwrap();
            black_box(runner.run().unwrap())
        });
    });
}

fn bench_counter_loop(c: &mut Criterion) {
    let script = compile(
        "bench.velin",
        concat!(
            "default index = 0\n",
            "while index < 2000:\n",
            "    set index = index + 1\n",
        ),
    )
    .unwrap();
    c.bench_function("vm/counter_loop", |b| {
        b.iter(|| {
            let mut runner = ScriptRunner::new(&script).unwrap();
            black_box(runner.run().unwrap())
        });
    });
}

fn bench_scalar_reassignment(c: &mut Criterion) {
    let script = compile(
        "bench.velin",
        concat!(
            "default value = 0\n",
            "default index = 0\n",
            "while index < 2000:\n",
            "    set value = 7\n",
            "    set index = index + 1\n",
        ),
    )
    .unwrap();
    c.bench_function("vm/scalar_reassignment", |b| {
        b.iter(|| {
            let mut runner = ScriptRunner::new(&script).unwrap();
            black_box(runner.run().unwrap())
        });
    });
}

fn bench_boolean_slot_loop(c: &mut Criterion) {
    let script = compile(
        "bench.velin",
        concat!(
            "default enabled = true\n",
            "default index = 0\n",
            "while index < 1500:\n",
            "    if enabled:\n",
            "        set index = index + 1\n",
        ),
    )
    .unwrap();
    c.bench_function("vm/boolean_slot_loop", |b| {
        b.iter(|| {
            let mut runner = ScriptRunner::new(&script).unwrap();
            black_box(runner.run().unwrap())
        });
    });
}

fn bench_builtin_loop(c: &mut Criterion) {
    let script = compile(
        "bench.velin",
        concat!(
            "default items = list(1, 2, 3)\n",
            "default index = 0\n",
            "while index < 1500:\n",
            "    set present = contains(items, 2)\n",
            "    set size = len(items)\n",
            "    set index = index + 1\n",
        ),
    )
    .unwrap();
    c.bench_function("vm/builtin_loop", |b| {
        b.iter(|| {
            let mut runner = ScriptRunner::new(&script).unwrap();
            black_box(runner.run().unwrap())
        });
    });
}

fn bench_growing_string(c: &mut Criterion) {
    let script = compile(
        "bench.velin",
        concat!(
            "default text = \"\"\n",
            "default index = 0\n",
            "while index < 2000:\n",
            "    set text = text + \"x\"\n",
            "    set index = index + 1\n",
        ),
    )
    .unwrap();
    c.bench_function("vm/growing_string", |b| {
        b.iter(|| {
            let mut runner = ScriptRunner::new(&script).unwrap();
            black_box(runner.run().unwrap())
        });
    });
}

fn bench_string_reads(c: &mut Criterion) {
    let source = format!(
        "default text = \"{}\"\ndefault index = 0\nwhile index < 1500:\n    set present = contains(text, \"z\")\n    set index = index + 1\n",
        "x".repeat(16 * 1024)
    );
    let script = compile("bench.velin", &source).unwrap();
    c.bench_function("vm/string_reads", |b| {
        b.iter(|| {
            let mut runner = ScriptRunner::new(&script).unwrap();
            black_box(runner.run().unwrap())
        });
    });
}

fn bench_machine_creation(c: &mut Criterion) {
    let source = wide_linear_source(512);
    let script = compile("bench.velin", &source).unwrap();
    let mut group = c.benchmark_group("machine/create_wide");
    group.bench_function("validate", |b| {
        b.iter(|| Machine::new(black_box(script.program.clone())).unwrap());
    });
    group.bench_function("reuse_validation", |b| {
        b.iter(|| Machine::from_validated(black_box(script.validated_program())));
    });
    group.finish();
}

fn bench_expression_heavy_machine_creation(c: &mut Criterion) {
    let source = expression_heavy_source(512);
    let script = compile("bench.velin", &source).unwrap();
    let mut group = c.benchmark_group("machine/create_expression_heavy");
    group.bench_function("validate", |b| {
        b.iter(|| Machine::new(black_box(script.program.clone())).unwrap());
    });
    group.bench_function("reuse_validation", |b| {
        b.iter(|| Machine::from_validated(black_box(script.validated_program())));
    });
    group.finish();
}

fn bench_short_circuit_heavy_machine_creation(c: &mut Criterion) {
    let source = short_circuit_heavy_source(512);
    let script = compile("bench.velin", &source).unwrap();
    let mut group = c.benchmark_group("machine/create_short_circuit_heavy");
    group.bench_function("validate", |b| {
        b.iter(|| Machine::new(black_box(script.program.clone())).unwrap());
    });
    group.bench_function("reuse_validation", |b| {
        b.iter(|| Machine::from_validated(black_box(script.validated_program())));
    });
    group.finish();
}

fn bench_eval_tree(c: &mut Criterion) {
    let expr = parse_expression(GUARD, "bench", 1, 1).unwrap();
    let mut vars = Variables::new();
    vars.insert("hp".into(), Value::Integer(30));
    vars.insert("level".into(), Value::Integer(12));
    vars.insert("defeated".into(), Value::Boolean(false));
    c.bench_function("eval/tree_walk", |b| {
        b.iter(|| evaluate(black_box(&expr), &vars, 1).unwrap());
    });
}

/// Runs a small program end to end through the VM, including one host yield,
/// the way an embedder actually drives it.
fn bench_vm_run(c: &mut Criterion) {
    let guard = parse_expression(GUARD, "bench", 1, 1).unwrap();
    let program = {
        let mut builder = ProgramBuilder::new();
        let hp = builder.slot("hp");
        let choice = builder.slot("choice");

        let prompt = builder.expr(&Expr::Value(Value::String("continue?".into())), 1);
        builder.push(Op::host(1, vec![prompt], Some(choice), 1));

        let condition = builder.expr(&guard, 2);
        let skip = builder.push(Op::JumpIfFalse {
            condition,
            target: u32::MAX,
        });
        let heal = builder.expr(
            &Expr::Binary {
                left: Box::new(Expr::Variable("hp".into())),
                op: velin::BinaryOp::Add,
                right: Box::new(Expr::Value(Value::Integer(10))),
            },
            3,
        );
        builder.push(Op::Set {
            slot: hp,
            value: heal,
        });
        let after = builder.here();
        builder.patch(
            skip,
            Op::JumpIfFalse {
                condition,
                target: after,
            },
        );
        builder.build()
    };

    c.bench_function("vm/run_with_host_yield", |b| {
        b.iter(|| {
            let mut machine = Machine::new(program.clone()).unwrap();
            machine.set_variable("hp", Value::Integer(30));
            machine.set_variable("level", Value::Integer(12));
            machine.set_variable("defeated", Value::Boolean(false));
            let mut outcome = machine.run().unwrap();
            loop {
                match outcome {
                    Yield::Finished => break,
                    Yield::Host { .. } => {
                        outcome = machine.resume(Some(Value::Integer(1))).unwrap();
                    }
                }
            }
            black_box(machine.variable("hp").cloned())
        });
    });
}

criterion_group!(
    benches,
    bench_parse,
    bench_parse_wide_script,
    bench_parse_host_calls,
    bench_check,
    bench_check_wide_script,
    bench_check_expression_heavy_script,
    bench_check_builtin_heavy_script,
    bench_check_short_circuit_heavy_script,
    bench_compile,
    bench_compile_wide_script,
    bench_compile_expression_heavy_script,
    bench_constant_folding,
    bench_counter_loop,
    pipeline_p1::bench_register_expression_loop,
    pipeline_p2::bench_prepared_expression_loop,
    pipeline_p2::bench_propagated_constants,
    bench_scalar_reassignment,
    bench_boolean_slot_loop,
    bench_builtin_loop,
    bench_growing_list,
    bench_growing_string,
    pipeline_p2::bench_interpolation,
    bench_string_reads,
    bench_wide_linear_run,
    bench_machine_creation,
    bench_expression_heavy_machine_creation,
    bench_short_circuit_heavy_machine_creation,
    bench_eval_tree,
    bench_vm_run
);
criterion_main!(benches);
