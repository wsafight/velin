#[cfg(feature = "runtime")]
use criterion::{Criterion, criterion_group, criterion_main};
#[cfg(feature = "runtime")]
use std::fmt::Write as _;
#[cfg(feature = "runtime")]
use std::hint::black_box;
#[cfg(feature = "runtime")]
use velin::compile;
#[cfg(feature = "runtime")]
use velin_wasm::{RuntimeMachine, RuntimeProgram};

#[cfg(feature = "runtime")]
fn bench_wasm_batch(c: &mut Criterion) {
    let mut source = String::new();
    for index in 0..128 {
        let _ = writeln!(source, "perform emit({index})");
    }
    let script = compile("wasm-bench.velin", &source).unwrap();
    let json = serde_json::to_string(&*script.program).unwrap();
    let program = RuntimeProgram::new(&json)
        .unwrap_or_else(|_| panic!("Wasm runtime program creation failed"));
    c.bench_function("host/wasm_machine_create_reused", |b| {
        b.iter(|| black_box(program.create_machine()));
    });
    c.bench_function("host/wasm_batch", |b| {
        b.iter(|| {
            let mut machine = program.create_machine();
            let first = machine.run_batch(64);
            let second = machine.run_batch(64);
            let final_batch = machine.run_batch(64);
            black_box((first, second, final_batch))
        });
    });
    c.bench_function("host/wasm_load_and_batch", |b| {
        b.iter(|| {
            let mut machine = RuntimeMachine::new(&json)
                .unwrap_or_else(|_| panic!("Wasm runtime machine creation failed"));
            let first = machine.run_batch(64);
            let second = machine.run_batch(64);
            let final_batch = machine.run_batch(64);
            black_box((first, second, final_batch))
        });
    });
}

#[cfg(feature = "runtime")]
criterion_group!(benches, bench_wasm_batch);
#[cfg(feature = "runtime")]
criterion_main!(benches);

#[cfg(not(feature = "runtime"))]
fn main() {}
