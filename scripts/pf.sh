#!/usr/bin/env bash
# Local preflight checks for committed or uncommitted changes.
# Mirrors the substantive CI checks in a local fail-fast sequence.
# Purpose:
# - Provide one canonical "all checks good" command.
# - Catch code, contract, packaging, runtime, TUI, and supply-chain failures.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"
START_SECONDS=$SECONDS

log() {
  printf '[preflight] %s\n' "$*"
}

format_elapsed() {
  local elapsed="$1"
  local minutes=$((elapsed / 60))
  local seconds=$((elapsed % 60))

  if ((minutes > 0)); then
    printf '%dm %02ds' "$minutes" "$seconds"
  else
    printf '%ds' "$seconds"
  fi
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
run_step "cargo package listing" cargo package --list --allow-dirty
run_step "distribution matrix sync check" ./scripts/distribution_matrix.py --check-sync
run_step "runtime smoke (linux/macos)" ./scripts/clipboard_smoke.sh --skip-clipboard-checks
run_step "tui e2e hty journeys 1..10" ./scripts/tui_e2e_hty.sh --journey 1,2,3,4,5,6,7,8,9,10
run_step "cargo deny check" cargo deny check
run_step "cargo audit" cargo audit
run_step "cargo tree --locked -e normal" cargo tree --locked -e normal

log "All preflight checks passed in $(format_elapsed "$((SECONDS - START_SECONDS))")"
