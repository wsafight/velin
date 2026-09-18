#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
baseline_file="${VELIN_PERF_BASELINE:-$repo_root/benchmarks/performance-baseline.json}"
target_dir="${CARGO_TARGET_DIR:-$repo_root/target}"
sample_size="${VELIN_PERF_SAMPLE_SIZE:-10}"
warmup_time="${VELIN_PERF_WARMUP_TIME:-0.1}"
measurement_time="${VELIN_PERF_MEASUREMENT_TIME:-0.2}"
size_output="$(mktemp "${TMPDIR:-/tmp}/velin-performance.XXXXXX")"
trap 'rm -f "$size_output"' EXIT

cd "$repo_root"

bench_args=(
    --noplot
    --sample-size "$sample_size"
    --warm-up-time "$warmup_time"
    --measurement-time "$measurement_time"
    --save-baseline ci
    --format terse
)

printf 'running pipeline benchmarks (sample=%s warm-up=%ss measurement=%ss)\n' \
    "$sample_size" "$warmup_time" "$measurement_time"
cargo bench -p velin --bench pipeline -- "${bench_args[@]}"

printf '\nrunning C ABI benchmarks\n'
cargo bench -p velin-capi --bench c_api -- "${bench_args[@]}"

printf '\nrunning Wasm runtime benchmarks\n'
cargo bench -p velin-wasm --features runtime --bench runtime -- "${bench_args[@]}"

printf '\nmeasuring release artifact sizes\n'
bash scripts/measure-embed-size.sh | tee "$size_output"

python3 - "$baseline_file" "$target_dir/criterion" "$size_output" <<'PY'
import json
import re
import sys
from pathlib import Path

baseline_path = Path(sys.argv[1])
criterion_root = Path(sys.argv[2])
size_output = Path(sys.argv[3])
baseline = json.loads(baseline_path.read_text())
factor = float(baseline["max_regression_factor"])
failures = []

benchmark_dirs = {}
for metadata in criterion_root.rglob("ci/benchmark.json"):
    data = json.loads(metadata.read_text())
    benchmark_dirs[data["full_id"]] = metadata.parent.parent

for name, expected in baseline["benchmarks"].items():
    directory = benchmark_dirs.get(name)
    estimates = directory / "ci" / "estimates.json" if directory else None
    if estimates is None or not estimates.is_file():
        failures.append(f"missing benchmark result: {name}")
        continue
    actual = json.loads(estimates.read_text())["median"]["point_estimate"]
    limit = expected * factor
    status = "ok" if actual <= limit else "FAIL"
    print(f"benchmark {name}: {actual:.2f} ns (limit {limit:.2f}) {status}")
    if actual > limit:
        failures.append(
            f"{name} is {actual / expected:.2f}x baseline (limit {factor:.2f}x)"
        )

size_pattern = re.compile(r"^(.*?)\s+([0-9]+) bytes\s+([0-9]+) gzip\s+")
observed_sizes = {}
for line in size_output.read_text().splitlines():
    match = size_pattern.match(line)
    if match:
        observed_sizes[match.group(1).strip()] = (int(match.group(2)), int(match.group(3)))

for label, limits in baseline["artifacts"].items():
    actual = observed_sizes.get(label)
    if actual is None:
        failures.append(f"missing artifact size: {label}")
        continue
    byte_limit = limits["bytes"]
    gzip_limit = limits["gzip_bytes"]
    status = "ok" if actual[0] <= byte_limit and actual[1] <= gzip_limit else "FAIL"
    print(
        f"artifact {label}: {actual[0]} bytes / {actual[1]} gzip "
        f"(limits {byte_limit} / {gzip_limit}) {status}"
    )
    if actual[0] > byte_limit or actual[1] > gzip_limit:
        failures.append(f"{label} exceeds the checked-in size ceiling")

if failures:
    print("performance gate failed:")
    for failure in failures:
        print(f"- {failure}")
    raise SystemExit(1)

print("performance and size gates passed")
PY
