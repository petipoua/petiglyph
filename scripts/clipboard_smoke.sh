#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
petiglyph_bin=""
skip_cli_checks=0
skip_clipboard_checks=0
verify_readback=1

usage() {
  cat <<'EOF'
Usage:
  ./scripts/clipboard_smoke.sh [options]

Options:
  --bin <path>          Use an already-built petiglyph binary (skip cargo run)
  --skip-cli-checks     Skip petiglyph command checks; run clipboard checks only
  --skip-clipboard-checks
                        Skip clipboard checks; run petiglyph CLI checks only
  --no-readback-verify  Skip clipboard readback verification
  -h, --help            Show this help

Notes:
  - Linux/macOS only. Use scripts/clipboard_smoke.ps1 on Windows.
  - This script is non-destructive; it only checks CLI health and clipboard copy/readback.
EOF
}

log() {
  printf '[%s] %s\n' "$1" "$2"
}

fail() {
  log "FAIL" "$1" >&2
  exit 1
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --bin)
      [[ $# -ge 2 ]] || fail "--bin requires a path"
      petiglyph_bin="$2"
      shift 2
      ;;
    --skip-cli-checks)
      skip_cli_checks=1
      shift
      ;;
    --skip-clipboard-checks)
      skip_clipboard_checks=1
      shift
      ;;
    --no-readback-verify)
      verify_readback=0
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      fail "unknown argument: $1"
      ;;
  esac
done

run_petiglyph() {
  if [[ -n "$petiglyph_bin" ]]; then
    "$petiglyph_bin" "$@"
  else
    (cd "$repo_root" && cargo run --quiet -- "$@")
  fi
}

normalized_text() {
  # Normalize CRLF/LF differences for clipboard backends.
  printf '%s' "$1" | tr -d '\r'
}

detect_os() {
  local kernel
  kernel="$(uname -s)"
  case "$kernel" in
    Linux*) echo "linux" ;;
    Darwin*) echo "macos" ;;
    *) echo "unsupported" ;;
  esac
}

copy_with_provider() {
  local provider="$1"
  local payload="$2"
  case "$provider" in
    wl-copy)
      printf '%s' "$payload" | wl-copy
      ;;
    xclip)
      printf '%s' "$payload" | xclip -selection clipboard
      ;;
    pbcopy)
      printf '%s' "$payload" | pbcopy
      ;;
    *)
      return 1
      ;;
  esac
}

readback_with_provider() {
  local provider="$1"
  case "$provider" in
    wl-copy)
      if command -v wl-paste >/dev/null 2>&1; then
        wl-paste --no-newline 2>/dev/null || wl-paste 2>/dev/null
      else
        return 2
      fi
      ;;
    xclip)
      xclip -selection clipboard -o
      ;;
    pbcopy)
      pbpaste
      ;;
    *)
      return 1
      ;;
  esac
}

main() {
  local os
  os="$(detect_os)"
  [[ "$os" != "unsupported" ]] || fail "unsupported OS for this script. Use scripts/clipboard_smoke.ps1 on Windows."

  if [[ $skip_cli_checks -eq 0 ]]; then
    log "INFO" "running petiglyph CLI smoke checks"
    run_petiglyph --help >/dev/null
    log "OK" "petiglyph --help"

    local doctor_json
    doctor_json="$(run_petiglyph doctor --json)"
    if ! grep -Eq '"ok"[[:space:]]*:[[:space:]]*true' <<<"$doctor_json"; then
      fail "petiglyph doctor --json did not report ok=true"
    fi
    log "OK" "petiglyph doctor --json"

    local tui_out
    tui_out="$(mktemp)"
    if run_petiglyph tui < /dev/null >"$tui_out" 2>&1; then
      rm -f "$tui_out"
      fail "petiglyph tui should fail without a TTY"
    fi
    if ! grep -qi "requires a terminal" "$tui_out"; then
      cat "$tui_out" >&2
      rm -f "$tui_out"
      fail "non-TTY TUI failure message did not include terminal-required guidance"
    fi
    rm -f "$tui_out"
    log "OK" "petiglyph tui non-TTY guard"
  fi

  if [[ $skip_clipboard_checks -eq 1 ]]; then
    log "OK" "clipboard checks skipped"
    return 0
  fi

  local -a providers
  if [[ "$os" == "macos" ]]; then
    providers=("pbcopy")
  else
    if [[ -n "${WAYLAND_DISPLAY:-}" ]]; then
      providers=("wl-copy")
    else
      providers=("xclip" "wl-copy")
    fi
  fi

  local payload selected_provider
  payload="petiglyph-clipboard-smoke-$(date +%s)-$$"
  selected_provider=""

  log "INFO" "running clipboard provider chain: ${providers[*]}"

  local provider
  for provider in "${providers[@]}"; do
    if ! command -v "$provider" >/dev/null 2>&1; then
      log "WARN" "provider not found on PATH: $provider"
      continue
    fi

    if copy_with_provider "$provider" "$payload"; then
      selected_provider="$provider"
      log "OK" "copied payload with $provider"
      break
    else
      log "WARN" "provider failed: $provider"
    fi
  done

  [[ -n "$selected_provider" ]] || fail "no clipboard provider succeeded (${providers[*]})"

  if [[ $verify_readback -eq 1 ]]; then
    local readback
    if readback="$(readback_with_provider "$selected_provider" 2>/dev/null)"; then
      if [[ "$(normalized_text "$readback")" != "$(normalized_text "$payload")" ]]; then
        fail "clipboard readback mismatch for $selected_provider"
      fi
      log "OK" "clipboard readback matched payload"
    else
      log "WARN" "readback skipped (no compatible paste command for $selected_provider)"
    fi
  fi

  log "OK" "clipboard smoke test finished"
}

main
