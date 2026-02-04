#!/usr/bin/env zsh
# Pre-release validation script
# Runs dev-fast build, CLI smoke tests, and full test suite

emulate -L zsh
setopt err_exit pipe_fail

local ROOT_DIR=${0:A:h}
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-${ROOT_DIR}/target}

print "[pre-release] building CLI (dev-fast)"
cd ${ROOT_DIR}/code-rs
cargo build --locked --profile dev-fast --bin code

print "[pre-release] running CLI smokes (skip cargo tests)"
SKIP_CARGO_TESTS=1 CI_CLI_BIN="${CARGO_TARGET_DIR}/dev-fast/code" \
  zsh ${ROOT_DIR}/scripts/ci-tests.sh

print "[pre-release] running workspace tests (nextest)"
cargo nextest run --no-fail-fast --locked

print "✅ Pre-release checks passed!"
