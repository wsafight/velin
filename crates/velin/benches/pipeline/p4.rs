//! Host, artifact, restart, and mixed-workload pipeline benchmarks.

use crate::GUARD;
use crate::sources::{expression_heavy_source, host_call_source, wide_linear_source};
use criterion::Criterion;
use std::collections::BTreeMap;
use std::hint::black_box;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use velin::{
    Expr, Machine, Op, ProgramBuilder, PureModule, ScriptRunner, ScriptYield, Type, Value,
    Variables, Yield, artifact_cache_path, compile, decode_artifact, encode_artifact, evaluate,
    load_artifact_cache, parse_expression, store_artifact_cache,
};

pub(crate) fn bench_eval_tree(c: &mut Criterion) {
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
pub(crate) fn bench_vm_run(c: &mut Criterion) {
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

fn run_unbound_effects_one_at_a_time(runner: &mut ScriptRunner) -> usize {
    let mut effects = 0;
    let mut outcome = runner.run().unwrap();
    loop {
        match outcome {
            ScriptYield::Host { .. } => {
                effects += 1;
                outcome = runner.resume(None).unwrap();
            }
            ScriptYield::Finished => return effects,
        }
    }
}

fn run_unbound_effects_in_batches(runner: &mut ScriptRunner, limit: usize) -> usize {
    let mut effects = 0;
    loop {
        let batch = runner.run_effect_batch(limit).unwrap();
        effects += batch.len();
        if batch.is_empty() {
            return effects;
        }
    }
}

pub(crate) fn bench_host_roundtrips(c: &mut Criterion) {
    let script = compile("bench.velin", &host_call_source(256)).unwrap();
    c.bench_function("host/single_effect_roundtrip", |b| {
        b.iter(|| {
            let mut runner = ScriptRunner::new(&script).unwrap();
            black_box(run_unbound_effects_one_at_a_time(&mut runner))
        });
    });
    c.bench_function("host/batched_effect_roundtrip", |b| {
        b.iter(|| {
            let mut runner = ScriptRunner::new(&script).unwrap();
            black_box(run_unbound_effects_in_batches(&mut runner, 64))
        });
    });
}

fn benchmark_temp_path(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is after the Unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "velin-bench-{label}-{}-{nonce}",
        std::process::id()
    ))
}

pub(crate) fn bench_artifact_pipeline(c: &mut Criterion) {
    let source = wide_linear_source(256);
    let script = compile("bench.velin", &source).unwrap();
    let artifact = encode_artifact("bench.velin", &script).unwrap();
    let cache_dir = benchmark_temp_path("cache");
    let cache_path = artifact_cache_path(&cache_dir, "hit.velinc");
    store_artifact_cache(&cache_path, "bench.velin", &script).unwrap();
    let miss_path = artifact_cache_path(&cache_dir, "miss.velinc");

    c.bench_function("artifact/source_compile", |b| {
        b.iter(|| black_box(compile("bench.velin", black_box(&source)).unwrap()));
    });
    c.bench_function("artifact/binary_decode", |b| {
        b.iter(|| black_box(decode_artifact(black_box(&artifact)).unwrap()));
    });
    c.bench_function("artifact/cache_hit", |b| {
        b.iter(|| black_box(load_artifact_cache(black_box(&cache_path)).unwrap()));
    });
    c.bench_function("artifact/cache_miss", |b| {
        b.iter(|| black_box(load_artifact_cache(black_box(&miss_path)).unwrap()));
    });
    eprintln!(
        "artifact baseline: source_bytes={} artifact_bytes={} cache={}",
        source.len(),
        artifact.len(),
        cache_path.display()
    );
    let _ = std::fs::remove_dir_all(cache_dir);
}

pub(crate) fn bench_machine_restart(c: &mut Criterion) {
    let source = expression_heavy_source(256);
    let script = compile("bench.velin", &source).unwrap();
    c.bench_function("machine/restart", |b| {
        let mut runner = ScriptRunner::new(&script).unwrap();
        b.iter(|| {
            runner.restart(0).unwrap();
            black_box(runner.run().unwrap())
        });
    });
}

pub(crate) fn bench_pure_module_invocation(c: &mut Criterion) {
    let module = PureModule::compile(
        "bench.velin",
        "perform return(value * 2 + 1)\n",
        BTreeMap::from([(String::from("value"), Type::Integer)]),
    )
    .unwrap();
    let mut group = c.benchmark_group("pure/invoke");
    group.bench_function("map_fresh", |b| {
        b.iter(|| {
            black_box(
                module
                    .invoke(BTreeMap::from([(
                        String::from("value"),
                        Value::Integer(21),
                    )]))
                    .unwrap(),
            )
        });
    });
    let mut map_invoker = module.invoker().unwrap();
    group.bench_function("map_reused", |b| {
        b.iter(|| {
            black_box(
                map_invoker
                    .invoke(BTreeMap::from([(
                        String::from("value"),
                        Value::Integer(21),
                    )]))
                    .unwrap(),
            )
        });
    });
    let mut one_invoker = module.invoker().unwrap();
    group.bench_function("one_reused", |b| {
        b.iter(|| black_box(one_invoker.invoke_one(Value::Integer(21)).unwrap()));
    });
    group.finish();
}

pub(crate) fn bench_profile_overhead(c: &mut Criterion) {
    let script = compile(
        "bench.velin",
        "set value = 0\nset index = 0\nwhile index < 256:\n    set value = value + index\n    set index = index + 1\nperform return(value)\n",
    )
    .unwrap();
    let mut group = c.benchmark_group("machine/profile");
    group.bench_function("enabled", |b| {
        b.iter(|| {
            let mut machine = Machine::from_validated(script.validated_program());
            black_box(machine.run().unwrap())
        });
    });
    group.bench_function("disabled", |b| {
        b.iter(|| {
            let mut machine =
                Machine::from_validated_with_seed_without_profile(script.validated_program(), 0);
            black_box(machine.run().unwrap())
        });
    });
    group.finish();
}

pub(crate) fn bench_execution_image(c: &mut Criterion) {
    let source = wide_linear_source(256);
    let script = compile("bench.velin", &source).unwrap();
    let program = script.program.clone();
    let image = (*program).clone().into_execution_image();
    let full_bytes = serde_json::to_vec(&*program).unwrap();
    let image_bytes = serde_json::to_vec(image.program()).unwrap();
    eprintln!(
        "execution image baseline: program_json_bytes={} image_json_bytes={} debug_locations={}",
        full_bytes.len(),
        image_bytes.len(),
        image.debug().len()
    );
    c.bench_function("memory/execution_image", |b| {
        b.iter(|| black_box((*program).clone().into_execution_image()));
    });
}

pub(crate) fn bench_real_workloads(c: &mut Criterion) {
    let dialogue = compile(
        "dialogue.velin",
        concat!(
            "default hp = 30\n",
            "perform say(\"start\")\n",
            "choice = perform ask(\"continue?\")\n",
            "if choice == 1:\n",
            "    set hp = hp + 10\n",
            "else:\n",
            "    set hp = hp - 5\n",
            "perform say(\"done [hp]\")\n",
        ),
    )
    .unwrap();
    c.bench_function("workload/dialogue", |b| {
        b.iter(|| {
            let mut runner = ScriptRunner::new(&dialogue).unwrap();
            let mut outcome = runner.run().unwrap();
            loop {
                outcome = match outcome {
                    ScriptYield::Host { name, .. } if name == "ask" => {
                        runner.resume(Some(Value::Integer(1))).unwrap()
                    }
                    ScriptYield::Host { .. } => runner.resume(None).unwrap(),
                    ScriptYield::Finished => break,
                };
            }
            black_box(runner.host_effects())
        });
    });

    let inventory = compile(
        "inventory.velin",
        concat!(
            "default items = list()\n",
            "default index = 0\n",
            "while index < 150:\n",
            "    set items = push(items, index)\n",
            "    set present = contains(items, index)\n",
            "    set rendered = \"item [index] present [present]\"\n",
            "    set index = index + 1\n",
        ),
    )
    .unwrap();
    c.bench_function("workload/inventory", |b| {
        b.iter(|| {
            let mut runner = ScriptRunner::new(&inventory).unwrap();
            black_box(runner.run().unwrap())
        });
    });

    let mixed = compile(
        "mixed.velin",
        concat!(
            "default index = 0\n",
            "default total = 0\n",
            "while index < 100:\n",
            "    set total = total + index\n",
            "    if index < 50:\n",
            "        perform emit(index, total)\n",
            "    set index = index + 1\n",
        ),
    )
    .unwrap();
    c.bench_function("workload/mixed", |b| {
        b.iter(|| {
            let mut runner = ScriptRunner::new(&mixed).unwrap();
            black_box(run_unbound_effects_in_batches(&mut runner, 32))
        });
    });
}

pub(crate) fn bench_vm_additional_shapes(c: &mut Criterion) {
    let short = compile(
        "short.velin",
        "default source = 1\nset result = source + 2\n",
    )
    .unwrap();
    c.bench_function("vm/short_scalar_expression", |b| {
        b.iter(|| {
            let mut runner = ScriptRunner::new(&short).unwrap();
            black_box(runner.run().unwrap())
        });
    });

    let branched = compile(
        "branched.velin",
        concat!(
            "default index = 0\n",
            "default total = 0\n",
            "while index < 500:\n",
            "    if index < 250:\n",
            "        set total = total + 1\n",
            "    else:\n",
            "        set total = total + 2\n",
            "    set index = index + 1\n",
        ),
    )
    .unwrap();
    c.bench_function("vm/branched_scalar_loop", |b| {
        b.iter(|| {
            let mut runner = ScriptRunner::new(&branched).unwrap();
            black_box(runner.run().unwrap())
        });
    });

    let interpolation = compile(
        "interpolation-mixed.velin",
        concat!(
            "default text = \"value\"\n",
            "default enabled = true\n",
            "default index = 0\n",
            "while index < 500:\n",
            "    set rendered = \"prefix [text] item [index] enabled [enabled]\"\n",
            "    set index = index + 1\n",
        ),
    )
    .unwrap();
    c.bench_function("vm/interpolation_mixed_holes", |b| {
        b.iter(|| {
            let mut runner = ScriptRunner::new(&interpolation).unwrap();
            black_box(runner.run().unwrap())
        });
    });
}
