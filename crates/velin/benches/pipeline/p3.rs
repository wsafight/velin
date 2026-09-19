//! VM loop and machine-creation pipeline benchmarks.

use crate::sources::{expression_heavy_source, short_circuit_heavy_source, wide_linear_source};
use criterion::Criterion;
use std::hint::black_box;
use velin::{Builtin, Machine, ScriptRunner, Value, compile};
use velin_eval::{invoke_measured_with_metrics, invoke_measured_with_metrics_reusable};

pub(crate) fn bench_wide_linear_run(c: &mut Criterion) {
    let source = wide_linear_source(512);
    let script = compile("bench.velin", &source).unwrap();
    c.bench_function("vm/wide_linear_script", |b| {
        b.iter(|| {
            let mut runner = ScriptRunner::new(&script).unwrap();
            black_box(runner.run().unwrap())
        });
    });
}

pub(crate) fn bench_growing_list(c: &mut Criterion) {
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

pub(crate) fn bench_counter_loop(c: &mut Criterion) {
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

pub(crate) fn bench_scalar_reassignment(c: &mut Criterion) {
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

pub(crate) fn bench_boolean_slot_loop(c: &mut Criterion) {
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

pub(crate) fn bench_builtin_loop(c: &mut Criterion) {
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

pub(crate) fn bench_owned_builtin_loop(c: &mut Criterion) {
    let script = compile(
        "bench.velin",
        concat!(
            "default index = 0\n",
            "while index < 1500:\n",
            "    set item = record(\"index\", index)\n",
            "    set index = index + 1\n",
        ),
    )
    .unwrap();
    c.bench_function("vm/owned_builtin_loop", |b| {
        b.iter(|| {
            let mut runner = ScriptRunner::new(&script).unwrap();
            black_box(runner.run().unwrap())
        });
    });
}

pub(crate) fn bench_owned_builtin_arguments(c: &mut Criterion) {
    let metrics = [
        Value::String("index".into()).data_metrics().unwrap(),
        Value::Integer(1).data_metrics().unwrap(),
    ];
    let mut group = c.benchmark_group("builtin/record_arguments");
    group.bench_function("fresh", |b| {
        b.iter(|| {
            black_box(
                invoke_measured_with_metrics(
                    Builtin::Record,
                    vec![Value::String("index".into()), Value::Integer(1)],
                    &metrics,
                    1,
                )
                .unwrap(),
            )
        });
    });
    group.bench_function("reused", |b| {
        let mut arguments = Vec::with_capacity(2);
        b.iter(|| {
            arguments.push(Value::String("index".into()));
            arguments.push(Value::Integer(1));
            black_box(
                invoke_measured_with_metrics_reusable(Builtin::Record, &mut arguments, &metrics, 1)
                    .unwrap(),
            )
        });
    });
    group.finish();
}

pub(crate) fn bench_growing_string(c: &mut Criterion) {
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

pub(crate) fn bench_string_reads(c: &mut Criterion) {
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

pub(crate) fn bench_machine_creation(c: &mut Criterion) {
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

pub(crate) fn bench_expression_heavy_machine_creation(c: &mut Criterion) {
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

pub(crate) fn bench_short_circuit_heavy_machine_creation(c: &mut Criterion) {
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
