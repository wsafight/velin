#!/usr/bin/env bash
set -euo pipefail

readonly max_lines=500
readonly repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
readonly candidates_file="$(mktemp)"
trap 'rm -f "$candidates_file"' EXIT
violations=0
candidate_count=0

if ! find "$repo_root" \
  -type d \( -name target -o -name node_modules -o -name .git -o -name dist \) -prune \
  -o -type f \( \
    -name '*.rs' -o -name '*.js' -o -name '*.mjs' -o -name '*.cjs' \
    -o -name '*.jsx' -o -name '*.ts' -o -name '*.tsx' -o -name '*.astro' \
    -o -name '*.vue' -o -name '*.svelte' -o -name '*.css' -o -name '*.scss' \
    -o -name '*.html' -o -name '*.sh' -o -name '*.bash' -o -name '*.yml' \
    -o -name '*.yaml' -o -name '*.toml' \) -print0 >"$candidates_file"; then
  printf 'failed to enumerate source files under %s\n' "$repo_root" >&2
  exit 1
fi

while IFS= read -r -d '' source_file; do
  candidate_count=$((candidate_count + 1))
  line_count="$(awk 'END { print NR }' "$source_file")"
  if (( line_count > max_lines )); then
    printf '%s: %s lines (maximum %s)\n' "$source_file" "$line_count" "$max_lines" >&2
    violations=1
  fi
done <"$candidates_file"

if (( candidate_count == 0 )); then
  printf 'no source files found under %s\n' "$repo_root" >&2
  exit 1
fi

if (( violations != 0 )); then
  printf 'source files must not exceed %s lines\n' "$max_lines" >&2
  exit 1
fi

printf 'all source files are within the %s-line limit\n' "$max_lines"
