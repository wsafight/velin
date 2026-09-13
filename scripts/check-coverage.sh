#!/usr/bin/env bash
# Strict per-crate coverage: production code only, 95% line coverage minimum.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

output="${TMPDIR:-/tmp}/velin-coverage.json"
ignore='(_tests\.rs$|/tests/)'

cargo llvm-cov --workspace --json --summary-only \
  --ignore-filename-regex "$ignore" \
  --output-path "$output"

python3 - "$output" <<'PY'
import json
import sys
from collections import defaultdict
from pathlib import Path

threshold = 95.0
data = json.load(open(sys.argv[1]))
files = data["data"][0]["files"]
crates = defaultdict(lambda: [0, 0])

for file in files:
    path = Path(file["filename"])
    parts = path.parts
    if "crates" not in parts:
        continue
    crate = parts[parts.index("crates") + 1]
    summary = file["summary"]
    if "lines" not in summary:
        summary = summary["summary"]
    crates[crate][0] += summary["lines"]["covered"]
    crates[crate][1] += summary["lines"]["count"]

failed = False
print(f"{'crate':22} {'lines':>11} {'pct':>7}  status")
for name in sorted(crates):
    covered, total = crates[name]
    percent = 100.0 * covered / total if total else 100.0
    status = "ok" if percent >= threshold else "BELOW"
    if percent < threshold:
        failed = True
    print(f"{name:22} {covered:5}/{total:<5} {percent:6.2f}%  {status}")

if failed:
    sys.exit(f"each crate must have at least {threshold:.0f}% line coverage of non-test code")
print(f"all crates have at least {threshold:.0f}% line coverage of non-test code")
PY
