#!/usr/bin/env bash
# Preflight checks to run before pushing commits.
# Mirrors the main CI checks in a local fail-fast sequence.
# Purpose:
# - Catch contract and packaging drift early in local development.
# - Includes distribution matrix sync checks; intentionally excludes clean-tree assertion
#   so local pre-commit runs are not blocked by unrelated working-tree changes.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

log() {
  printf '[preflight] %s\n' "$*"
}

run_step() {
  local name="$1"
  shift
  log "START: ${name}"
  "$@"
  log "PASS:  ${name}"
}

run_step "cargo fmt --check" cargo fmt --check
run_step "cargo check --locked" cargo check --locked
run_step "cargo clippy --locked --all-targets --all-features -- -D warnings" \
  cargo clippy --locked --all-targets --all-features -- -D warnings
run_step "cargo test --locked" cargo test --locked
run_step "distribution matrix sync check" ./scripts/distribution_matrix.py --check-sync
run_step "runtime smoke (linux/macos)" ./scripts/clipboard_smoke.sh --skip-clipboard-checks
run_step "tui e2e hty journeys 1..10" ./scripts/tui_e2e_hty.sh --journey 1,2,3,4,5,6,7,8,9,10

log "All preflight checks passed"
