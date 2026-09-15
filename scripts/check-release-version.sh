#!/usr/bin/env bash
set -euo pipefail

readonly repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

vsix_version="$(node -p "require('./editors/vscode-velin/package.json').version")"
cargo_version="$(cargo metadata --no-deps --format-version 1 | node -e '
  let input = "";
  process.stdin.on("data", chunk => input += chunk);
  process.stdin.on("end", () => {
    const versions = [...new Set(JSON.parse(input).packages.map(pkg => pkg.version))];
    if (versions.length !== 1) process.exit(1);
    process.stdout.write(versions[0]);
  });
')"

if [[ "$vsix_version" != "$cargo_version" ]]; then
  printf 'VSIX version %s does not match Cargo workspace version %s\n' \
    "$vsix_version" "$cargo_version" >&2
  exit 1
fi

if [[ $# -gt 0 && -n "$1" && "$1" != "v${cargo_version}" ]]; then
  printf 'release tag %s does not match workspace version v%s\n' "$1" "$cargo_version" >&2
  exit 1
fi

printf '%s\n' "$cargo_version"
