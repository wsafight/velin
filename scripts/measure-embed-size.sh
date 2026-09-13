#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
target_dir="${CARGO_TARGET_DIR:-$repo_root/target}"
export CARGO_TARGET_DIR="$target_dir"

file_size() {
    case "$(uname -s)" in
        Darwin) stat -f%z "$1" ;;
        *) stat -c%s "$1" ;;
    esac
}

report() {
    local label="$1"
    local path="$2"
    local compressed="${path}.gz"
    gzip -9 -c "$path" > "$compressed"
    printf '%-28s %10s bytes  %10s gzip  %s\n' \
        "$label" "$(file_size "$path")" "$(file_size "$compressed")" "$path"
}

cd "$repo_root"
printf 'rust: %s\n' "$(rustc --version)"
printf 'cargo target: %s\n\n' "$target_dir"

cargo build --release --no-default-features -p velin-vm --bin runtime_only
report "runtime-only example" "$target_dir/release/runtime_only"

cargo build --release -p velin-capi
report "C runtime staticlib" "$target_dir/release/libvelin_capi.a"

if rustup target list --installed | grep -qx 'wasm32-unknown-unknown'; then
    cargo build --release --target wasm32-unknown-unknown \
        -p velin-wasm --no-default-features --features runtime
    runtime_wasm="$target_dir/wasm32-unknown-unknown/release/velin_wasm.wasm"
    runtime_copy="$target_dir/wasm32-unknown-unknown/release/velin_wasm.runtime.wasm"
    cp "$runtime_wasm" "$runtime_copy"
    report "runtime-only wasm" "$runtime_copy"

    cargo build --release --target wasm32-unknown-unknown -p velin-wasm
    report "source-to-run wasm" \
        "$target_dir/wasm32-unknown-unknown/release/velin_wasm.wasm"
else
    printf '\nwasm32-unknown-unknown is not installed; skipping wasm measurement.\n'
fi

cargo build --release -p velin-cli
report "full CLI" "$target_dir/release/velin"
