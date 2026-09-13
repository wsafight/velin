//! P1-specific benchmarks for the optional internal register execution path.

use criterion::Criterion;
use std::fmt::Write as _;
use std::hint::black_box;
use velin::{ScriptRunner, compile};

pub(crate) fn bench_register_expression_loop(c: &mut Criterion) {
    let mut source = String::new();
    for index in 0..16 {
        writeln!(source, "default value_{index} = 1").expect("writing to a string cannot fail");
    }
    source.push_str("default index = 0\nwhile index < 500:\n    set result = ");
    for index in 0..16 {
        if index > 0 {
            source.push_str(" + ");
        }
        write!(source, "value_{index}").expect("writing to a string cannot fail");
    }
    source.push_str("\n    set index = index + 1\n");
    let script = compile("bench.velin", &source).unwrap();
    c.bench_function("vm/register_expression_loop", |b| {
        b.iter(|| {
            let mut runner = ScriptRunner::new(&script).unwrap();
            black_box(runner.run().unwrap())
        });
    });
}
