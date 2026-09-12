# Performance

[简体中文](PERFORMANCE.zh-CN.md)

This page records the relative effect of the recent checker and VM optimizations. The numbers are local measurements, not universal hardware promises. They are useful as regression baselines: rerun the same workload on the same machine when comparing changes.

## Read the numbers

The CLI measurements include process startup, parsing, lowering, and checking. They are useful for the experience of checking a complete script, but they include more than the algorithm under test. Criterion measurements run in-process and isolate one path more closely.

### Results

The static-checker workload contains `N` defaults followed by `N` assignments. The release CLI measurements show the change in shape clearly:

| `N` | Before | After |
| ---: | ---: | ---: |
| 250 | ~13 ms | ~6.15 ms |
| 500 | ~35 ms | ~4.43 ms |
| 1,000 | ~117 ms | ~6.31 ms |
| 2,000 | ~452 ms | ~9.10 ms |
| 4,000 | ~1,765 ms | ~15.38 ms |

The largest case is roughly 115x faster. The small cases are dominated by CLI overhead, so they should not be read as a perfectly monotonic curve. A fixed one-slot comparison stays near-linear after the same change: 4,000 assignments take ~12.93 ms and 8,000 take ~20.50 ms.

The focused in-process measurements are:

| Benchmark | Measurement | Relative result |
| --- | ---: | --- |
| `check/wide_linear_script` (512 variables) | ~305.29 µs | static checking baseline |
| `machine/create_wide/validate` | ~146.54 µs | validates a program each time |
| `machine/create_wide/reuse_validation` | ~720.53 ns | ~203x less validation work |
| `vm/counter_loop` (2,000 iterations) | ~120.97 µs | ~36% faster than ~189.06 µs |
| `vm/growing_list` (2,000 pushes) | ~10.265 ms | ~2% faster than ~10.494 ms |

Reusing value footprints also reduced a separate CLI run of the 2,000-step list-growth script from ~17.0 ms to ~14.2 ms, about 17%. This is an end-to-end comparison rather than a Criterion result.

## What changed

### Static analysis propagates state by basic block

The old checker cloned a complete slot-type vector and rebuilt a `BTreeMap` environment at every control-flow instruction. That made a wide linear script behave quadratically as the number of slots grew.

The checker now propagates the state belonging to a basic block and merges only at block boundaries. Linear code therefore does work proportional to the code and slot count instead of repeatedly copying the whole environment. The conservative type rules and diagnostics are unchanged.

### Value footprints are reused

Expression evaluation now returns the `Value` together with its `DataFootprint`. Assignment and host-payload accounting reuse that footprint instead of traversing the same value a second time. Scalar values use a no-allocation fast path.

Persistent lists and records still use copy-on-write semantics. The optimization removes duplicate accounting; it does not turn collection updates into in-place mutation.

### Validated programs are reused safely

`ValidatedProgram` is a proof that wraps the shared `Arc<Program>`. `Machine::new` and `Machine::with_seed` continue to validate arbitrary external programs. The `ScriptRunner` path reuses the validation result from the original compiled script, while replacing the public program `Arc` falls back to validation when pointer identity no longer matches. The safety boundary remains explicit.

### The VM reuses its expression stack

The hot loop now reuses the VM expression stack and avoids cloning instruction data while executing. This helps tight scalar loops substantially. The smaller improvement for growing lists is expected: persistent collection updates still copy their path, so that cost remains the dominant part of the workload.

## What remains intentionally expensive

Persistent collections are designed for snapshots, replay, and alias-safe values. Replacing their copy-on-write behavior with in-place mutation would change those semantics. Host effects, serialization, process startup, and allocation behavior are also outside the `vm/*` microbenchmarks, so application-level performance depends on the host integration as well as the VM.

## Reproduce the measurements

Run the full workspace checks first:

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Run the Criterion suite without opening plots:

```sh
cargo bench -p velin --bench pipeline -- --noplot
```

Useful focused filters are:

```sh
cargo bench -p velin --bench pipeline -- check/wide_linear_script --noplot
cargo bench -p velin --bench pipeline -- machine/create_wide --noplot
cargo bench -p velin --bench pipeline -- 'vm/(counter_loop|growing_list)' --noplot
```

For CLI comparisons, build the release binary and run the same generated script repeatedly. Record the machine, Rust toolchain, and whether the timing includes process startup before comparing results.
