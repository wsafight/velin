//! End-to-end pipeline microbenchmarks.
//!
//! These are intentionally small and deterministic: they measure the cost of
//! each stage (parse, check, compile, run) on representative expressions so
//! regressions show up as relative shifts, not absolute wall-clock claims.

use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;
use velin::{
    Environment, ScriptRunner, SlotTable, check_expression, check_script, compile,
    compile_expression, parse_expression, parse_program,
};

#[path = "pipeline/p0.rs"]
mod pipeline_p0;
#[path = "pipeline/p1.rs"]
mod pipeline_p1;
#[path = "pipeline/p2.rs"]
mod pipeline_p2;
#[path = "pipeline/p3.rs"]
mod pipeline_p3;
#[path = "pipeline/p4.rs"]
mod pipeline_p4;
#[path = "pipeline/sources.rs"]
mod sources;

/// A non-trivial arithmetic/boolean guard, the shape a real script branches on.
pub(crate) const GUARD: &str = "hp - 10 > 0 and (level * 2 + 5) <= 100 or defeated == false";

fn bench_parse(c: &mut Criterion) {
    c.bench_function("parse/guard", |b| {
        b.iter(|| parse_expression(black_box(GUARD), "bench", 1, 1).unwrap());
    });
}

fn bench_parse_wide_script(c: &mut Criterion) {
    let source = sources::wide_linear_source(512);
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
    let source = sources::wide_linear_source(512);
    let script = compile("bench.velin", &source).unwrap();
    c.bench_function("check/wide_linear_script", |b| {
        b.iter(|| check_script("bench.velin", black_box(&script)));
    });
}

fn bench_check_expression_heavy_script(c: &mut Criterion) {
    let source = sources::expression_heavy_source(512);
    let script = compile("bench.velin", &source).unwrap();
    c.bench_function("check/expression_heavy_script", |b| {
        b.iter(|| check_script("bench.velin", black_box(&script)));
    });
}

fn bench_check_builtin_heavy_script(c: &mut Criterion) {
    let source = sources::builtin_heavy_source(512);
    let script = compile("bench.velin", &source).unwrap();
    c.bench_function("check/builtin_heavy_script", |b| {
        b.iter(|| check_script("bench.velin", black_box(&script)));
    });
}

fn bench_check_short_circuit_heavy_script(c: &mut Criterion) {
    let source = sources::short_circuit_heavy_source(512);
    let script = compile("bench.velin", &source).unwrap();
    c.bench_function("check/short_circuit_heavy_script", |b| {
        b.iter(|| check_script("bench.velin", black_box(&script)));
    });
}
fn bench_parse_host_calls(c: &mut Criterion) {
    let source = sources::host_call_source(512);
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
    let source = sources::wide_linear_source(512);
    c.bench_function("compile/wide_linear_script", |b| {
        b.iter(|| compile("bench.velin", black_box(&source)).unwrap());
    });
}

fn bench_compile_expression_heavy_script(c: &mut Criterion) {
    let source = sources::expression_heavy_source(512);
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
criterion_group!(
    benches,
    pipeline_p0::bench_machine_snapshot,
    pipeline_p0::bench_snapshot_replay,
    pipeline_p0::bench_host_queue_push_pop,
    pipeline_p0::bench_host_queue_backpressure,
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
    pipeline_p3::bench_counter_loop,
    pipeline_p1::bench_long_register_expression_loop,
    pipeline_p2::bench_small_register_expression_loop,
    pipeline_p2::bench_propagated_constants,
    pipeline_p3::bench_scalar_reassignment,
    pipeline_p3::bench_boolean_slot_loop,
    pipeline_p3::bench_builtin_loop,
    pipeline_p3::bench_owned_builtin_loop,
    pipeline_p3::bench_owned_builtin_arguments,
    pipeline_p3::bench_growing_list,
    pipeline_p3::bench_growing_string,
    pipeline_p2::bench_interpolation,
    pipeline_p3::bench_string_reads,
    pipeline_p3::bench_wide_linear_run,
    pipeline_p3::bench_machine_creation,
    pipeline_p3::bench_expression_heavy_machine_creation,
    pipeline_p3::bench_short_circuit_heavy_machine_creation,
    pipeline_p4::bench_eval_tree,
    pipeline_p4::bench_vm_run,
    pipeline_p4::bench_host_roundtrips,
    pipeline_p4::bench_artifact_pipeline,
    pipeline_p4::bench_machine_restart,
    pipeline_p4::bench_pure_module_invocation,
    pipeline_p4::bench_profile_overhead,
    pipeline_p4::bench_execution_image,
    pipeline_p4::bench_real_workloads,
    pipeline_p4::bench_vm_additional_shapes
);
criterion_main!(benches);
