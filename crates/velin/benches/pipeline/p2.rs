//! P2-specific VM benchmarks for prepared expressions and interpolation.

use criterion::Criterion;
use std::fmt::Write as _;
use std::hint::black_box;
use velin::{ScriptRunner, compile};

pub(crate) fn bench_prepared_expression_loop(c: &mut Criterion) {
    let script = compile(
        "bench.velin",
        concat!(
            "default source = 1\n",
            "default index = 0\n",
            "while index < 2000:\n",
            "    set value = source + 1\n",
            "    set index = index + 1\n",
        ),
    )
    .unwrap();
    c.bench_function("vm/prepared_expression_loop", |b| {
        b.iter(|| {
            let mut runner = ScriptRunner::new(&script).unwrap();
            black_box(runner.run().unwrap())
        });
    });
}

pub(crate) fn bench_interpolation(c: &mut Criterion) {
    let script = compile(
        "bench.velin",
        concat!(
            "default text = \"value\"\n",
            "default index = 0\n",
            "while index < 1500:\n",
            "    set rendered = \"prefix [text] item [index]\"\n",
            "    set index = index + 1\n",
        ),
    )
    .unwrap();
    c.bench_function("vm/interpolation", |b| {
        b.iter(|| {
            let mut runner = ScriptRunner::new(&script).unwrap();
            black_box(runner.run().unwrap())
        });
    });
}

pub(crate) fn bench_propagated_constants(c: &mut Criterion) {
    let mut source = String::from("set base = 1\n");
    for index in 0..512 {
        writeln!(source, "set value_{index} = base + {index}")
            .expect("writing to a string cannot fail");
    }
    let script = compile("bench.velin", &source).unwrap();
    c.bench_function("vm/propagated_constants", |b| {
        b.iter(|| {
            let mut runner = ScriptRunner::new(&script).unwrap();
            black_box(runner.run().unwrap())
        });
    });
}
