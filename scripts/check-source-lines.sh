#!/usr/bin/env bash
set -euo pipefail

readonly max_lines=500
violations=0

while IFS= read -r -d '' source_file; do
  line_count="$(awk 'END { print NR }' "$source_file")"
  if (( line_count > max_lines )); then
    printf '%s: %s lines (maximum %s)\n' "$source_file" "$line_count" "$max_lines" >&2
    violations=1
  fi
done < <(
  git ls-files --cached --others --exclude-standard -z -- \
    '*.rs' '*.js' '*.mjs' '*.cjs' '*.jsx' '*.ts' '*.tsx' \
    '*.astro' '*.vue' '*.svelte' '*.css' '*.scss' '*.html' \
    '*.sh' '*.bash' '*.yml' '*.yaml' '*.toml'
)

if (( violations != 0 )); then
  printf 'source files must not exceed %s lines\n' "$max_lines" >&2
  exit 1
fi

printf 'all source files are within the %s-line limit\n' "$max_lines"
