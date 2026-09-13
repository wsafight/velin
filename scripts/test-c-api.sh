#!/bin/sh
set -eu

cargo build -p velin-capi

cc -std=c11 -Wall -Wextra -Werror \
  -I crates/velin-capi/include \
  crates/velin-capi/tests/c_api_smoke.c \
  target/debug/libvelin_capi.a \
  -ldl -lpthread -lm \
  -o target/velin-c-api-smoke

target/velin-c-api-smoke
