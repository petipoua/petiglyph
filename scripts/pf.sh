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
declare -a MISSING_TOOLS=()

log() {
  printf '[preflight] %s\n' "$*"
}

require_tool() {
  local tool_name="$1"
  local install_hint="$2"

  if ! command -v "$tool_name" >/dev/null 2>&1; then
    MISSING_TOOLS+=("${tool_name}|${install_hint}")
  fi
}

check_required_tools() {
  MISSING_TOOLS=()

  require_tool cargo "sudo pacman -S rust"
  require_tool cargo-fmt "rustup component add rustfmt"
  require_tool cargo-clippy "rustup component add clippy"
  require_tool cargo-deny "cargo install --locked cargo-deny"
  require_tool cargo-audit "cargo install --locked cargo-audit"
  require_tool python3 "sudo pacman -S python"

  if ((${#MISSING_TOOLS[@]} > 0)); then
    log "missing required tools; install them and rerun:"
    for entry in "${MISSING_TOOLS[@]}"; do
      local tool_name="${entry%%|*}"
      local install_hint="${entry#*|}"
      log "  ${tool_name}: ${install_hint}"
    done
    exit 1
  fi
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

run_quiet_step() {
  local pass_message="$1"
  shift
  local output_file
  output_file="$(mktemp)"

  if "$@" >"$output_file" 2>&1; then
    log "${pass_message}"
    rm -f "$output_file"
    return 0
  fi

  cat "$output_file" >&2
  rm -f "$output_file"
  return 1
}

check_required_tools

run_step "cargo fmt --check" cargo fmt --check
run_step "cargo check --locked" cargo check --locked
run_step "cargo clippy --locked --all-targets --all-features -- -D warnings" \
  cargo clippy --locked --all-targets --all-features -- -D warnings
run_step "cargo test --locked" cargo test --locked
run_step "cargo package listing" cargo package --list --allow-dirty
run_step "distribution matrix sync check" ./scripts/distribution_matrix.py --check-sync
run_step "runtime smoke (linux/macos)" ./scripts/clipboard_smoke.sh --skip-clipboard-checks
run_step "tui e2e hty journeys 1..10" ./scripts/tui_e2e_hty.sh --journey 1,2,3,4,5,6,7,8,9,10
run_quiet_step "cargo deny PASS" cargo deny check
run_quiet_step "cargo audit PASS" cargo audit

log "All preflight checks passed in $(format_elapsed "$((SECONDS - START_SECONDS))")"
