//! P0 runtime-boundary benchmarks for snapshots, replay, and host backpressure.

use crate::sources::wide_linear_source;
use criterion::Criterion;
use std::hint::black_box;
use velin::{
    HostEvent, HostEventQueue, HostEventQueueLimits, Machine, ScriptRunner, Value, Yield, compile,
};

pub(crate) fn bench_machine_snapshot(c: &mut Criterion) {
    let script = compile("snapshot.velin", &wide_linear_source(256)).unwrap();
    let mut runner = ScriptRunner::new(&script).unwrap();
    assert_eq!(runner.run().unwrap(), velin::ScriptYield::Finished);
    let machine = runner.machine().clone();

    c.bench_function("snapshot/machine_clone", |b| {
        b.iter(|| black_box(machine.clone()));
    });
}

pub(crate) fn bench_snapshot_replay(c: &mut Criterion) {
    let script = compile(
        "replay.velin",
        concat!(
            "default roll = 0\n",
            "roll = perform ask(random(1, 100))\n",
            "set follow_up = random(1, 100)\n",
        ),
    )
    .unwrap();
    let mut machine = Machine::with_seed(script.program.clone(), 7).unwrap();
    assert!(matches!(machine.run().unwrap(), Yield::Host { .. }));
    let checkpoint = machine.clone();

    c.bench_function("snapshot/clone_and_replay", |b| {
        b.iter(|| {
            let mut replay = checkpoint.clone();
            let outcome = replay.resume(Some(Value::Integer(42))).unwrap();
            black_box((outcome, replay.rng_state(), replay.variable("follow_up")));
        });
    });
}

fn benchmark_event() -> HostEvent {
    HostEvent {
        name: "emit".into(),
        values: vec![Value::Integer(42), Value::String("payload".into())],
    }
}

pub(crate) fn bench_host_queue_push_pop(c: &mut Criterion) {
    let limits = HostEventQueueLimits {
        capacity: 128,
        max_values: 1_024,
        max_text_bytes: 16 * 1024,
    };
    let event = benchmark_event();
    let mut queue = HostEventQueue::new(limits);
    c.bench_function("queue/push_pop", |b| {
        b.iter(|| {
            for _ in 0..limits.capacity {
                queue.push(event.clone()).unwrap();
            }
            let mut popped = 0;
            while queue.pop_front().is_some() {
                popped += 1;
            }
            black_box(popped)
        });
    });
}

pub(crate) fn bench_host_queue_backpressure(c: &mut Criterion) {
    let limits = HostEventQueueLimits {
        capacity: 64,
        max_values: 512,
        max_text_bytes: 8 * 1024,
    };
    let event = benchmark_event();
    let mut queue = HostEventQueue::new(limits);
    for _ in 0..limits.capacity {
        queue.push(event.clone()).unwrap();
    }

    c.bench_function("queue/full_backpressure", |b| {
        b.iter(|| {
            let rejected = queue.push(event.clone()).is_err();
            let consumed = queue.pop_front().is_some();
            let accepted = queue.push(event.clone()).is_ok();
            black_box((rejected, consumed, accepted));
        });
    });
}
