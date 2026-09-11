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
};

/// A non-trivial arithmetic/boolean guard, the shape a real script branches on.
const GUARD: &str = "hp - 10 > 0 and (level * 2 + 5) <= 100 or defeated == false";

fn bench_parse(c: &mut Criterion) {
    c.bench_function("parse/guard", |b| {
        b.iter(|| parse_expression(black_box(GUARD), "bench", 1, 1).unwrap());
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

fn bench_compile(c: &mut Criterion) {
    let expr = parse_expression(GUARD, "bench", 1, 1).unwrap();
    c.bench_function("compile/guard", |b| {
        b.iter(|| {
            let mut slots = SlotTable::new();
            compile_expression(black_box(&expr), &mut slots, 1)
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
        builder.push(Op::Host {
            host_id: 1,
            args: vec![prompt],
            bind: Some(choice),
            line: 1,
        });

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
    bench_check,
    bench_check_wide_script,
    bench_compile,
    bench_growing_list,
    bench_machine_creation,
    bench_eval_tree,
    bench_vm_run
);
criterion_main!(benches);
