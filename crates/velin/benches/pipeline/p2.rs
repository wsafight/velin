//! P2-specific VM benchmarks for prepared expressions and interpolation.

use criterion::Criterion;
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
