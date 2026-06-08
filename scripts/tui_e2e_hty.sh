#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

hty_bin="${HTY_BIN:-hty}"
petiglyph_bin="${PETIGLYPH_BIN:-}"
hty_runtime_dir="${HTY_RUNTIME_DIR:-}"
hty_state_home="${HTY_STATE_HOME:-}"
child_xdg_runtime_dir="${XDG_RUNTIME_DIR:-}"

rows=42
cols=140
timeout_ms=20000
startup_wait_ms=8000
step_delay_ms=80
watch_enabled=0
watch_terminal="auto"
keep_sessions=0
render_probe_enabled=0
render_probe_only=0
render_probe_term="xterm-256color"
render_probe_colorterm="truecolor"
render_probe_duration_ms=12000
journey_count=10

declare -a sessions=()
declare -a watch_pids=()
declare -a temp_dirs=()
declare -a selected_journeys=()
declare -a closed_sentinel_sessions=()
declare -a cleaned_sessions=()
declare -a hty_env=()
current_session=""

usage() {
  cat <<'USAGE'
Usage:
  ./scripts/tui_e2e_hty.sh [options]

Runs process-level TUI E2E journeys with hty:
  1. launch + quit from existing project
  2. create project from Home panel
  3. install + rescan includes new source
  4. glyph threshold override persists and clears
  5. workspace project selection builds selected project
  6. creation workflow popup: create glyph
  7. creation workflow popup: create grid
  8. creation workflow popup: create animated glyph from GIF
  9. install lifecycle via Home panel
 10. full multi-workflow lifecycle user story

Options:
  --journey N[,M...]   Run only selected journey number(s) (1-10). Repeatable.
  --watch              Auto-open a watcher terminal for each session (best effort)
  --watch-terminal T   Force watcher terminal: auto|ghostty|kitty|alacritty|foot|tmux
                       (default: auto)
  --render-probe       Run a standalone render/erase diagnostic session
  --render-probe-only  Run only the render probe (skip journeys)
  --render-probe-term T
                       TERM value for render probe session (default: xterm-256color)
  --render-probe-colorterm V
                       COLORTERM for render probe session (default: truecolor)
  --render-probe-duration-ms N
                       Render probe duration in ms (default: 12000)
  --timeout-ms N       Wait timeout in ms for waits and polling (default: 20000)
  --step-delay-ms N    Delay in ms after each input step (default: 80)
  --petiglyph-bin PATH Path to petiglyph binary (default: target/debug/petiglyph)
  --hty-bin PATH       Path to hty binary (default: hty from PATH)
  --keep-sessions      Keep hty sessions after script exits (debug)
  -h, --help           Show this help

Environment:
  HTY_BIN
  HTY_RUNTIME_DIR
  HTY_STATE_HOME
  PETIGLYPH_BIN

Examples:
  ./scripts/tui_e2e_hty.sh
  ./scripts/tui_e2e_hty.sh --journey 6,7,8
  ./scripts/tui_e2e_hty.sh --journey 5 --keep-sessions
  ./scripts/tui_e2e_hty.sh --watch --step-delay-ms 250
  ./scripts/tui_e2e_hty.sh --watch --watch-terminal alacritty
  ./scripts/tui_e2e_hty.sh --render-probe-only --watch
USAGE
}

log() {
  printf '[tui-e2e-hty] %s\n' "$*"
}

require_command() {
  local cmd="$1"
  if ! command -v "$cmd" >/dev/null 2>&1; then
    echo "Missing required command: $cmd" >&2
    exit 1
  fi
}

register_temp_dir() {
  temp_dirs+=("$1")
}

make_temp_dir() {
  local name="$1"
  local dir
  dir="$(mktemp -d "/tmp/petiglyph-tui-e2e-hty-${name}-XXXXXX")"
  register_temp_dir "$dir"
  printf '%s\n' "$dir"
}

setup_hty_env() {
  if [[ -z "$hty_runtime_dir" ]]; then
    hty_runtime_dir="$(make_temp_dir "hty-runtime")"
  else
    mkdir -p "$hty_runtime_dir"
  fi
  if [[ -z "$hty_state_home" ]]; then
    hty_state_home="$(make_temp_dir "hty-state")"
  else
    mkdir -p "$hty_state_home"
  fi
  chmod 700 "$hty_runtime_dir" "$hty_state_home" 2>/dev/null || true
  hty_env=(env XDG_RUNTIME_DIR="$hty_runtime_dir" XDG_STATE_HOME="$hty_state_home")
}

run_hty() {
  "${hty_env[@]}" "$hty_bin" "$@"
}

run_hty_timeout() {
  local duration="$1"
  shift
  timeout "$duration" "${hty_env[@]}" "$hty_bin" "$@"
}

hty_command_for_shell() {
  printf 'XDG_RUNTIME_DIR=%q XDG_STATE_HOME=%q %q' "$hty_runtime_dir" "$hty_state_home" "$hty_bin"
}

stop_hty_server() {
  local socket="$hty_runtime_dir/hty/sock"
  local pids
  pids="$(ps -eo pid=,args= | awk -v sock="$socket" '$0 ~ /hty __server__/ && index($0, sock) { print $1 }')"
  if [[ -n "$pids" ]]; then
    # shellcheck disable=SC2086
    kill $pids >/dev/null 2>&1 || true
  fi
}

append_selected_journey() {
  local id="$1"
  local existing
  for existing in "${selected_journeys[@]:-}"; do
    if [[ "$existing" == "$id" ]]; then
      return 0
    fi
  done
  selected_journeys+=("$id")
}

parse_journey_selector() {
  local raw="$1"
  local token
  local id
  IFS=',' read -r -a tokens <<<"$raw"
  for token in "${tokens[@]}"; do
    id="${token//[[:space:]]/}"
    [[ -n "$id" ]] || continue
    if [[ ! "$id" =~ ^([1-9]|10)$ ]]; then
      echo "Invalid journey id: $id (expected 1-10)" >&2
      exit 1
    fi
    append_selected_journey "$id"
  done
}

should_run_journey() {
  local id="$1"
  local selected
  if (( ${#selected_journeys[@]} == 0 )); then
    return 0
  fi
  for selected in "${selected_journeys[@]}"; do
    if [[ "$selected" == "$id" ]]; then
      return 0
    fi
  done
  return 1
}

write_test_png() {
  local out_path="$1"
  base64 -d >"$out_path" <<'PNGEOF'
iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+ip1sAAAAASUVORK5CYII=
PNGEOF
}

write_test_gif() {
  local out_path="$1"
  base64 -d >"$out_path" <<'GIFEOF'
R0lGODlhCAAIAPAAAAAAAP///yH/C05FVFNDQVBFMi4wAwEAAAAh+QQACgAAACwAAAAACAAIAAACDIwDCYfKug6ctFpaAAAh+QQACgAAACwAAAAACAAIAAACC4SPqcvhgR48Mr4CADs=
GIFEOF
}

seed_ffmpeg_prompt_state() {
  local home_dir="$1"
  local managed_dir="$home_dir/.local/share/fonts/petiglyph"
  local state_path="$managed_dir/.ffmpeg-setup-prompt-v1.json"
  mkdir -p "$managed_dir" "$home_dir/.config"
  cat >"$state_path" <<'JSON'
{"version":1,"outcome":"seeded_for_tui_e2e","at_unix_ms":0}
JSON
}

create_session_home() {
  local workspace="$1"
  local session_home="$workspace/fake-home"
  seed_ffmpeg_prompt_state "$session_home"
  printf '%s\n' "$session_home"
}

wait_for_path() {
  local path="$1"
  local timeout="$2"
  local start_ms now_ms elapsed_ms
  start_ms="$(date +%s%3N)"
  while true; do
    if [[ -e "$path" ]]; then
      return 0
    fi
    now_ms="$(date +%s%3N)"
    elapsed_ms="$((now_ms - start_ms))"
    if (( elapsed_ms >= timeout )); then
      echo "Timeout waiting for path: $path" >&2
      return 1
    fi
    sleep 0.03
  done
}

wait_for_file_contains() {
  local path="$1"
  local needle="$2"
  local timeout="$3"
  local start_ms now_ms elapsed_ms
  start_ms="$(date +%s%3N)"
  while true; do
    if [[ -f "$path" ]] && grep -Fq "$needle" "$path"; then
      return 0
    fi
    now_ms="$(date +%s%3N)"
    elapsed_ms="$((now_ms - start_ms))"
    if (( elapsed_ms >= timeout )); then
      echo "Timeout waiting for content in $path: $needle" >&2
      return 1
    fi
    sleep 0.03
  done
}

wait_for_file_not_contains() {
  local path="$1"
  local needle="$2"
  local timeout="$3"
  local start_ms now_ms elapsed_ms
  start_ms="$(date +%s%3N)"
  while true; do
    if [[ -f "$path" ]] && ! grep -Fq "$needle" "$path"; then
      return 0
    fi
    now_ms="$(date +%s%3N)"
    elapsed_ms="$((now_ms - start_ms))"
    if (( elapsed_ms >= timeout )); then
      echo "Timeout waiting for content removal in $path: $needle" >&2
      return 1
    fi
    sleep 0.03
  done
}

count_matching_files() {
  local dir="$1"
  local pattern="$2"
  if [[ ! -d "$dir" ]]; then
    echo 0
    return 0
  fi
  find "$dir" -maxdepth 1 -type f -name "$pattern" | wc -l | tr -d '[:space:]'
}

wait_for_matching_file_count_ge() {
  local dir="$1"
  local pattern="$2"
  local expected_min="$3"
  local timeout="$4"
  local start_ms now_ms elapsed_ms
  start_ms="$(date +%s%3N)"
  while true; do
    local current
    current="$(count_matching_files "$dir" "$pattern")"
    if (( current >= expected_min )); then
      return 0
    fi
    now_ms="$(date +%s%3N)"
    elapsed_ms="$((now_ms - start_ms))"
    if (( elapsed_ms >= timeout )); then
      echo "Timeout waiting for $pattern count >= $expected_min in $dir (current: $current)" >&2
      return 1
    fi
    sleep 0.03
  done
}

wait_for_matching_file_count_eq() {
  local dir="$1"
  local pattern="$2"
  local expected="$3"
  local timeout="$4"
  local start_ms now_ms elapsed_ms
  start_ms="$(date +%s%3N)"
  while true; do
    local current
    current="$(count_matching_files "$dir" "$pattern")"
    if (( current == expected )); then
      return 0
    fi
    now_ms="$(date +%s%3N)"
    elapsed_ms="$((now_ms - start_ms))"
    if (( elapsed_ms >= timeout )); then
      echo "Timeout waiting for $pattern count == $expected in $dir (current: $current)" >&2
      return 1
    fi
    sleep 0.03
  done
}

wait_for_ttf() {
  local build_dir="$1"
  local timeout="$2"
  local start_ms now_ms elapsed_ms
  start_ms="$(date +%s%3N)"
  while true; do
    local ttf
    ttf="$(find "$build_dir" -maxdepth 1 -type f -name '*.ttf' | head -n1 || true)"
    if [[ -n "$ttf" ]]; then
      printf '%s\n' "$ttf"
      return 0
    fi
    now_ms="$(date +%s%3N)"
    elapsed_ms="$((now_ms - start_ms))"
    if (( elapsed_ms >= timeout )); then
      echo "Timeout waiting for .ttf in: $build_dir" >&2
      return 1
    fi
    sleep 0.03
  done
}

start_watch_if_enabled() {
  local session="$1"
  if (( watch_enabled == 0 )); then
    return
  fi

  local watcher_cmd
  watcher_cmd="$(printf '%s watch %q' "$(hty_command_for_shell)" "$session")"
  if start_watcher_terminal "$watcher_cmd" "$session"; then
    log "watcher started for '$session'"
    sleep 0.2
    return
  fi

  echo "Unable to auto-open watcher terminal for session: $session" >&2
  echo "Install one of: ghostty, kitty, alacritty, foot, or use tmux." >&2
  exit 1
}

start_watcher_terminal() {
  local watcher_cmd="$1"
  local session="$2"
  local title="petiglyph watch: $session"
  local preferred="${watch_terminal:-auto}"

  launch_watcher_terminal() {
    local term="$1"
    case "$term" in
      ghostty)
        command -v ghostty >/dev/null 2>&1 || return 1
        ghostty --title="$title" -e env XDG_RUNTIME_DIR="$hty_runtime_dir" XDG_STATE_HOME="$hty_state_home" "$hty_bin" watch "$session" >/dev/null 2>&1 &
        watch_pids+=("$!")
        return 0
        ;;
      kitty)
        command -v kitty >/dev/null 2>&1 || return 1
        kitty --title "$title" bash -lc "$watcher_cmd; echo; read -r -p 'Press Enter to close...' _" >/dev/null 2>&1 &
        watch_pids+=("$!")
        return 0
        ;;
      alacritty)
        command -v alacritty >/dev/null 2>&1 || return 1
        alacritty --title "$title" -e bash -lc "$watcher_cmd; echo; read -r -p 'Press Enter to close...' _" >/dev/null 2>&1 &
        watch_pids+=("$!")
        return 0
        ;;
      foot)
        command -v foot >/dev/null 2>&1 || return 1
        foot --title "$title" bash -lc "$watcher_cmd; echo; read -r -p 'Press Enter to close...' _" >/dev/null 2>&1 &
        watch_pids+=("$!")
        return 0
        ;;
      tmux)
        command -v tmux >/dev/null 2>&1 || return 1
        if [[ -n "${TMUX:-}" ]]; then
          tmux split-window -v "$watcher_cmd"
          return 0
        fi

        local tmux_session_name="petiglyph-hty-watch"
        local tmux_window_name
        tmux_window_name="$(printf '%s' "$session" | tr -cd '[:alnum:]_-' | cut -c1-20)"
        [[ -n "$tmux_window_name" ]] || tmux_window_name="watch"

        if ! tmux has-session -t "$tmux_session_name" >/dev/null 2>&1; then
          tmux new-session -d -s "$tmux_session_name" -n "$tmux_window_name" "$watcher_cmd" >/dev/null 2>&1 || return 1
        else
          tmux new-window -d -t "$tmux_session_name:" -n "$tmux_window_name" "$watcher_cmd" >/dev/null 2>&1 || \
            tmux new-window -d -t "$tmux_session_name:" "$watcher_cmd" >/dev/null 2>&1 || return 1
        fi

        if command -v ghostty >/dev/null 2>&1; then
          ghostty --title="$title (tmux)" -e tmux attach -t "$tmux_session_name" >/dev/null 2>&1 &
          watch_pids+=("$!")
          return 0
        fi
        if command -v kitty >/dev/null 2>&1; then
          kitty --title "$title (tmux)" bash -lc "tmux attach -t $(printf '%q' "$tmux_session_name")" >/dev/null 2>&1 &
          watch_pids+=("$!")
          return 0
        fi
        if command -v alacritty >/dev/null 2>&1; then
          alacritty --title "$title (tmux)" -e bash -lc "tmux attach -t $(printf '%q' "$tmux_session_name")" >/dev/null 2>&1 &
          watch_pids+=("$!")
          return 0
        fi
        if command -v foot >/dev/null 2>&1; then
          foot --title "$title (tmux)" bash -lc "tmux attach -t $(printf '%q' "$tmux_session_name")" >/dev/null 2>&1 &
          watch_pids+=("$!")
          return 0
        fi

        log "tmux watcher session created: $tmux_session_name"
        log "attach manually with: tmux attach -t $tmux_session_name"
        return 0
        ;;
      *)
        return 1
        ;;
    esac
  }

  if [[ "$preferred" != "auto" ]]; then
    launch_watcher_terminal "$preferred"
    return $?
  fi

  launch_watcher_terminal ghostty && return 0
  launch_watcher_terminal kitty && return 0
  launch_watcher_terminal alacritty && return 0
  launch_watcher_terminal foot && return 0
  launch_watcher_terminal tmux && return 0
  return 1
}

run_session() {
  local session="$1"
  local cwd="$2"
  shift 2

  current_session="$session"
  sessions+=("$session")

  run_hty run \
    --name "$session" \
    --cwd "$cwd" \
    --rows "$rows" \
    --cols "$cols" \
    -- "$@" >/dev/null

  if ! wait_for_session_contains "$session" "petiglyph" "$startup_wait_ms"; then
    run_hty wait "$session" --idle 500 --timeout "$startup_wait_ms" >/dev/null 2>/dev/null || true
  fi
  start_watch_if_enabled "$session"
}

run_petiglyph_session() {
  local session="$1"
  local cwd="$2"
  local session_home="$3"
  local -a child_env=(
    env
    HOME="$session_home"
    XDG_CONFIG_HOME="$session_home/.config"
    PETIGLYPH_TUI_HTY_FULL_REPAINT=1
  )
  if [[ -n "$child_xdg_runtime_dir" ]]; then
    child_env+=(XDG_RUNTIME_DIR="$child_xdg_runtime_dir")
  fi
  run_session \
    "$session" \
    "$cwd" \
    "${child_env[@]}" "$petiglyph_bin"
}

wait_exit() {
  local session="$1"
  if run_hty wait "$session" --exit --timeout "$timeout_ms" >/dev/null 2>/dev/null; then
    return 0
  fi

  # hty v0.7.0 can keep a session marked as running after the TUI has already
  # restored the terminal and printed its shutdown sentinel.
  if session_snapshot_text "$session" | grep -Fq "tui session closed"; then
    if run_hty wait "$session" --exit --timeout 3000 >/dev/null 2>/dev/null; then
      return 0
    fi
    closed_sentinel_sessions+=("$session")
    log "session rendered TUI close sentinel before hty reported process exit; cleaning recorder session"
    return 0
  fi

  run_hty wait "$session" --exit --timeout "$timeout_ms" >/dev/null
}

session_rendered_close_sentinel() {
  local session="$1"
  local closed_session
  for closed_session in "${closed_sentinel_sessions[@]:-}"; do
    if [[ "$closed_session" == "$session" ]]; then
      return 0
    fi
  done
  return 1
}

session_already_cleaned() {
  local session="$1"
  local cleaned_session
  for cleaned_session in "${cleaned_sessions[@]:-}"; do
    if [[ "$cleaned_session" == "$session" ]]; then
      return 0
    fi
  done
  return 1
}

session_cleanup() {
  local session="$1"
  if session_already_cleaned "$session"; then
    return
  fi
  cleaned_sessions+=("$session")
  if (( keep_sessions == 1 )); then
    return
  fi
  if session_rendered_close_sentinel "$session"; then
    run_hty_timeout 3s kill "$session" >/dev/null 2>&1 || true
    stop_hty_server
    return
  fi
  run_hty_timeout 5s delete "$session" >/dev/null 2>&1 || {
    log "warning: hty delete timed out for '$session'"
  }
}

session_snapshot_text() {
  local session="$1"
  run_hty snapshot "$session" --ansi 2>/dev/null \
    | perl -pe 's/\e\[[0-9;?]*[ -\/]*[@-~]//g' || true
}

wait_for_session_contains() {
  local session="$1"
  local needle="$2"
  local timeout="$3"
  local start_ms now_ms elapsed_ms
  start_ms="$(date +%s%3N)"
  while true; do
    if session_snapshot_text "$session" | grep -Fq "$needle"; then
      return 0
    fi
    now_ms="$(date +%s%3N)"
    elapsed_ms="$((now_ms - start_ms))"
    if (( elapsed_ms >= timeout )); then
      echo "Timeout waiting for session content [$session]: $needle" >&2
      return 1
    fi
    sleep 0.05
  done
}

wait_for_session_not_contains() {
  local session="$1"
  local needle="$2"
  local timeout="$3"
  local start_ms now_ms elapsed_ms
  start_ms="$(date +%s%3N)"
  while true; do
    if ! session_snapshot_text "$session" | grep -Fq "$needle"; then
      return 0
    fi
    now_ms="$(date +%s%3N)"
    elapsed_ms="$((now_ms - start_ms))"
    if (( elapsed_ms >= timeout )); then
      echo "Timeout waiting for session content removal [$session]: $needle" >&2
      return 1
    fi
    sleep 0.05
  done
}

wait_for_session_contains_any() {
  local session="$1"
  local timeout="$2"
  shift 2
  local start_ms now_ms elapsed_ms
  start_ms="$(date +%s%3N)"
  while true; do
    local snap
    snap="$(session_snapshot_text "$session")"
    local needle
    for needle in "$@"; do
      if grep -Fq "$needle" <<<"$snap"; then
        return 0
      fi
    done
    now_ms="$(date +%s%3N)"
    elapsed_ms="$((now_ms - start_ms))"
    if (( elapsed_ms >= timeout )); then
      echo "Timeout waiting for any session content [$session]: $*" >&2
      return 1
    fi
    sleep 0.05
  done
}

send_key() {
  local session="$1"
  local key="$2"
  local label="$3"
  log "send [$session] $label"
  run_hty send "$session" --key "$key" >/dev/null
  sleep "$(awk "BEGIN { printf \"%.3f\", ${step_delay_ms}/1000 }")"
}

send_key_nowait() {
  local session="$1"
  local key="$2"
  local label="$3"
  log "send [$session] $label"
  run_hty send "$session" --key "$key" >/dev/null
}

send_raw_text() {
  local session="$1"
  local text="$2"
  local label="$3"
  log "send [$session] $label"
  run_hty send "$session" --raw-text "$text" >/dev/null
  sleep "$(awk "BEGIN { printf \"%.3f\", ${step_delay_ms}/1000 }")"
}

send_literal_keys() {
  local session="$1"
  local text="$2"
  local label="$3"
  local i
  local ch
  log "send [$session] $label"
  for ((i = 0; i < ${#text}; i++)); do
    ch="${text:i:1}"
    run_hty send "$session" --key "$ch" >/dev/null
  done
  sleep "$(awk "BEGIN { printf \"%.3f\", ${step_delay_ms}/1000 }")"
}

send_bracketed_paste() {
  local session="$1"
  local payload="$2"
  local label="$3"
  local payload_hex
  local wrapper_hex
  log "send [$session] $label"
  if command -v xxd >/dev/null 2>&1; then
    payload_hex="$(printf '%s' "$payload" | xxd -p -c 999999 | tr -d '\n')"
  else
    payload_hex="$(printf '%s' "$payload" | od -An -tx1 -v | tr -d ' \n')"
  fi
  wrapper_hex="1b5b3230307e${payload_hex}1b5b3230317e"
  run_hty send "$session" --bytes-hex "$wrapper_hex" >/dev/null
  sleep "$(awk "BEGIN { printf \"%.3f\", ${step_delay_ms}/1000 }")"
}

go_home_panel() {
  local session="$1"
  if session_snapshot_text "$session" | grep -Fq "Petiglyph projects"; then
    return 0
  fi
  send_raw_text "$session" "1" "normalize to Home panel"
  wait_for_session_contains "$session" "Petiglyph projects" "$timeout_ms"
}

go_glyphs_panel() {
  local session="$1"
  local snapshot

  glyphs_panel_visible() {
    snapshot="$(session_snapshot_text "$session")"
    grep -Eq '^[^[:alnum:]]+Glyphs[[:space:]]' <<<"$snapshot"
  }

  if glyphs_panel_visible; then
    return 0
  fi
  send_raw_text "$session" "2" "normalize to Glyphs panel"
  wait_for_session_contains "$session" "Glyphs" "$timeout_ms"
}

focus_home_create_glyph_button() {
  local session="$1"
  local prefix="$2"
  local i
  go_home_panel "$session"
  for i in 1 2 3 4 5 6; do
    send_key "$session" "up" "$prefix: normalize focus upward ($i/6)"
  done
  send_key "$session" "down" "$prefix: move to active project"
  send_key "$session" "right" "$prefix: move to install action"
  send_key "$session" "down" "$prefix: move to Create glyph button"
}

focus_create_input_with_active_project() {
  local session="$1"
  local prefix="$2"
  local i
  go_home_panel "$session"
  for i in 1 2 3 4 5 6; do
    send_key "$session" "up" "$prefix: normalize focus upward ($i/6)"
  done
  send_key "$session" "down" "$prefix: move to active project"
  send_key "$session" "right" "$prefix: move to install action"
  send_key "$session" "left" "$prefix: move to create input"
}

focus_installed_fonts_list() {
  local session="$1"
  local prefix="$2"
  local i
  go_home_panel "$session"
  for i in 1 2 3 4 5 6; do
    send_key "$session" "up" "$prefix: normalize focus upward ($i/6)"
  done
  send_key "$session" "down" "$prefix: move to active project"
  send_key "$session" "right" "$prefix: move to install action"
  send_key "$session" "left" "$prefix: move to create input"
  send_key "$session" "down" "$prefix: move to installed fonts list"
}

focus_home_install_action() {
  local session="$1"
  local prefix="$2"
  local i
  go_home_panel "$session"
  for i in 1 2 3 4 5 6; do
    send_key "$session" "up" "$prefix: normalize focus upward ($i/6)"
  done
  send_key "$session" "down" "$prefix: move to active project"
  send_key "$session" "right" "$prefix: move to install action"
}

open_first_project_from_home() {
  local session="$1"
  local prefix="$2"
  local i
  go_home_panel "$session"
  for i in 1 2 3 4 5 6 7 8 9 10; do
    send_key "$session" "up" "$prefix: normalize focus upward ($i/10)"
  done
  send_key "$session" "down" "$prefix: move to first project row"
  send_key "$session" "enter" "$prefix: open selected project"
  run_hty wait "$session" --idle 300 --timeout 2000 >/dev/null 2>/dev/null || true
}

delete_active_project() {
  local session="$1"
  local prefix="$2"
  local timeout="$3"
  local attempt
  local confirm_open_timeout=1800
  for attempt in 1 2 3; do
    focus_create_input_with_active_project "$session" "$prefix attempt ${attempt}"
    send_key "$session" "up" "$prefix: move to project list"
    send_key "$session" "right" "$prefix: move to install action"
    send_key "$session" "right" "$prefix: move to delete project action"
    send_key "$session" "enter" "$prefix: open delete confirmation"
    if wait_for_session_contains "$session" "Confirm Deletion" "$confirm_open_timeout"; then
      break
    fi
    if (( attempt == 3 )); then
      echo "Failed to open delete confirmation for active project" >&2
      return 1
    fi
  done
  send_key "$session" "right" "$prefix: select DELETE"
  send_key "$session" "enter" "$prefix: confirm delete"
  wait_for_session_not_contains "$session" "Confirm Deletion" "$timeout"
}

workflow_tweak_threshold_then_continue() {
  local session="$1"
  local prefix="$2"
  local continue_label="$3"
  wait_for_session_contains "$session" "Tweak grayscale / threshold / preview" "$timeout_ms"
  send_key "$session" "left" "$prefix: focus export test image"
  send_key "$session" "left" "$prefix: focus threshold"
  send_key "$session" "up" "$prefix: increase threshold"
  send_key "$session" "right" "$prefix: return to export test image"
  send_key "$session" "right" "$prefix: return to continue"
  send_key "$session" "enter" "$continue_label"
}

continue_from_tweak_until_animation_config() {
  local session="$1"
  local prefix="$2"
  local max_attempts="${3:-4}"
  local attempt
  local transition_timeout_ms=1200
  local snapshot

  animation_config_visible() {
    snapshot="$(session_snapshot_text "$session")"
    [[ "$snapshot" == *"Create Animation"* ]] && [[ "$snapshot" == *"FPS:"* ]]
  }

  for attempt in $(seq 1 "$max_attempts"); do
    if animation_config_visible; then
      return 0
    fi
    send_key "$session" "enter" "$prefix: continue to animation config (attempt ${attempt}/${max_attempts})"
    if wait_for_session_contains "$session" "Create Animation" "$transition_timeout_ms" && animation_config_visible; then
      return 0
    fi
  done

  wait_for_session_contains "$session" "Create Animation" "$timeout_ms"
  wait_for_session_contains "$session" "FPS:" "$timeout_ms"
}

tweak_in_glyph_panel() {
  local session="$1"
  local prefix="$2"
  go_glyphs_panel "$session"
  send_key "$session" "right" "$prefix: focus preview controls"
  send_key "$session" "up" "$prefix: adjust first knob"
  send_key "$session" "right" "$prefix: focus next knob"
  send_key "$session" "up" "$prefix: adjust second knob"
  send_key "$session" "left" "$prefix: move back one knob"
}

maybe_dismiss_first_install_notice() {
  local session="$1"
  local timeout_ms_local=3000
  local start_ms now_ms elapsed_ms
  start_ms="$(date +%s%3N)"
  while true; do
    if session_snapshot_text "$session" | grep -Fq "First Install Guidance"; then
      send_key "$session" "enter" "dismiss first-install guidance"
      wait_for_session_not_contains "$session" "First Install Guidance" "$timeout_ms"
      return 0
    fi
    now_ms="$(date +%s%3N)"
    elapsed_ms="$((now_ms - start_ms))"
    if (( elapsed_ms >= timeout_ms_local )); then
      return 0
    fi
    sleep 0.05
  done
}

create_empty_project() {
  local workspace="$1"
  local name="$2"
  (
    cd "$workspace"
    "$petiglyph_bin" new-project "$name" >/dev/null
  )
  printf '%s\n' "$workspace/$name"
}

create_project_with_icon() {
  local workspace="$1"
  local name="$2"
  local icon_name="$3"
  local project_dir
  project_dir="$(create_empty_project "$workspace" "$name")"
  write_test_png "$project_dir/images/$icon_name"
  printf '%s\n' "$project_dir"
}

journey_launch_and_quit() {
  log "journey 1/${journey_count}: launch and quit from existing project"
  local workspace project_dir session session_home
  workspace="$(make_temp_dir "launch-quit")"
  project_dir="$(create_project_with_icon "$workspace" "launch-quit-demo" "alpha.png")"
  session_home="$(create_session_home "$workspace")"
  session="petiglyph-e2e-launch-quit-$$-$(date +%s%N)"

  run_petiglyph_session "$session" "$project_dir" "$session_home"
  send_key "$session" "q" "quit"
  wait_exit "$session"
  session_cleanup "$session"
  current_session=""
  log "journey 1/${journey_count} passed"
}

journey_create_project_from_home() {
  log "journey 2/${journey_count}: create project from Home panel"
  local workspace session session_home project_name
  workspace="$(make_temp_dir "create-home")"
  session_home="$(create_session_home "$workspace")"
  project_name="fromtuie2e"
  session="petiglyph-e2e-create-home-$$-$(date +%s%N)"

  run_petiglyph_session "$session" "$workspace" "$session_home"
  go_home_panel "$session"
  send_key "$session" "enter" "focus create input"
  send_literal_keys "$session" "$project_name" "type project name"
  send_key "$session" "enter" "submit create"

  wait_for_path "$workspace/$project_name/petiglyph.toml" "$timeout_ms"
  wait_for_path "$workspace/$project_name/images" "$timeout_ms"

  send_key "$session" "q" "quit"
  wait_exit "$session"
  session_cleanup "$session"
  current_session=""
  log "journey 2/${journey_count} passed"
}

journey_build_and_rescan() {
  log "journey 3/${journey_count}: install + rescan includes new source"
  local workspace project_dir session session_home build_dir mapping sample ttf
  workspace="$(make_temp_dir "build-rescan")"
  project_dir="$(create_project_with_icon "$workspace" "build-rescan-demo" "alpha.png")"
  session_home="$(create_session_home "$workspace")"
  build_dir="$project_dir/build"
  mapping="$build_dir/glyph-map.json"
  sample="$build_dir/glyph-sample.txt"
  session="petiglyph-e2e-build-rescan-$$-$(date +%s%N)"

  run_petiglyph_session "$session" "$project_dir" "$session_home"

  send_key_nowait "$session" "i" "install (build + install)"
  wait_for_session_not_contains "$session" "Installing..." "$timeout_ms"
  wait_for_path "$mapping" "$timeout_ms"
  wait_for_path "$sample" "$timeout_ms"
  ttf="$(wait_for_ttf "$build_dir" "$timeout_ms")"
  [[ -n "$ttf" ]] || {
    echo "Expected .ttf output missing in $build_dir" >&2
    return 1
  }
  send_key "$session" "space" "dismiss first-install guidance"

  write_test_png "$project_dir/images/beta.png"
  wait_for_path "$project_dir/images/beta.png" "$timeout_ms"
  send_key "$session" "R" "rescan"
  send_key_nowait "$session" "i" "reinstall (build + install)"
  wait_for_session_not_contains "$session" "Installing..." "$timeout_ms"
  wait_for_path "$mapping" "$timeout_ms"
  send_key "$session" "space" "dismiss first-install guidance after reinstall"

  send_key "$session" "q" "quit"
  wait_exit "$session"
  session_cleanup "$session"
  current_session=""
  log "journey 3/${journey_count} passed"
}

journey_threshold_roundtrip() {
  log "journey 4/${journey_count}: glyph threshold override persists and clears"
  local workspace project_dir session session_home manifest_path
  workspace="$(make_temp_dir "threshold")"
  project_dir="$(create_project_with_icon "$workspace" "threshold-demo" "alpha.png")"
  session_home="$(create_session_home "$workspace")"
  manifest_path="$project_dir/petiglyph.toml"
  session="petiglyph-e2e-threshold-$$-$(date +%s%N)"

  run_petiglyph_session "$session" "$project_dir" "$session_home"
  for i in 1 2 3 4 5 6; do
    send_key "$session" "up" "focus panel tabs ($i/6)"
  done
  send_key "$session" "right" "select Glyphs panel tab"
  send_key "$session" "enter" "open selected Glyphs panel"
  wait_for_session_contains "$session" "Glyphs" "$timeout_ms"
  send_key "$session" "down" "focus Glyphs install action"
  send_key "$session" "down" "focus glyph list"
  send_key "$session" "right" "focus preview threshold"
  send_key "$session" "up" "increase threshold"

  wait_for_file_contains "$manifest_path" "threshold_overrides" "$timeout_ms"
  wait_for_file_contains "$manifest_path" "alpha.png" "$timeout_ms"

  send_key "$session" "r" "clear threshold override"
  wait_for_file_not_contains "$manifest_path" "alpha.png" "$timeout_ms"
  wait_for_file_not_contains "$manifest_path" "threshold_overrides" "$timeout_ms"

  send_key "$session" "q" "quit"
  wait_exit "$session"
  session_cleanup "$session"
  current_session=""
  log "journey 4/${journey_count} passed"
}

journey_workspace_selection() {
  log "journey 5/${journey_count}: workspace selection installs selected project"
  local workspace project_one project_two session session_home map_one map_two
  workspace="$(make_temp_dir "workspace-select")"
  project_one="$(create_project_with_icon "$workspace" "project-one" "only-one.png")"
  project_two="$(create_project_with_icon "$workspace" "project-two" "only-two.png")"
  session_home="$(create_session_home "$workspace")"
  map_one="$project_one/build/glyph-map.json"
  map_two="$project_two/build/glyph-map.json"
  session="petiglyph-e2e-workspace-select-$$-$(date +%s%N)"

  run_petiglyph_session "$session" "$workspace" "$session_home"
  go_home_panel "$session"
  send_key "$session" "down" "select second project"
  send_key "$session" "enter" "open selected project"
  run_hty wait "$session" --idle 300 --timeout "$timeout_ms" >/dev/null 2>/dev/null || true

  send_key_nowait "$session" "i" "install selected project"
  wait_for_session_not_contains "$session" "Installing..." "$timeout_ms"
  wait_for_path "$map_two" "$timeout_ms"
  wait_for_file_contains "$map_two" '"source_file": "only-two.png"' "$timeout_ms"

  if [[ -f "$map_one" ]]; then
    echo "Unexpected install/build output in unselected project: $map_one" >&2
    return 1
  fi

  send_key "$session" "q" "quit"
  wait_exit "$session"
  session_cleanup "$session"
  current_session=""
  log "journey 5/${journey_count} passed"
}

journey_creation_glyph() {
  log "journey 6/${journey_count}: creation workflow popup create glyph"
  local workspace project_dir session session_home source_png imported_png
  workspace="$(make_temp_dir "home-glyph")"
  project_dir="$(create_empty_project "$workspace" "home-glyph-demo")"
  session_home="$(create_session_home "$workspace")"
  source_png="$workspace/popup-glyph-source.png"
  imported_png="$project_dir/images/popup-glyph-source.png"
  write_test_png "$source_png"
  session="petiglyph-e2e-home-glyph-$$-$(date +%s%N)"

  run_petiglyph_session "$session" "$project_dir" "$session_home"
  focus_home_create_glyph_button "$session" "home glyph"
  send_key "$session" "enter" "start Create glyph workflow"
  wait_for_session_contains "$session" "Creation Workflow In Progress" "$timeout_ms"
  send_bracketed_paste "$session" "$source_png" "paste source path"
  wait_for_path "$imported_png" "$timeout_ms"
  send_key "$session" "enter" "advance to tweak step"
  wait_for_session_contains "$session" "Tweak grayscale / threshold / preview" "$timeout_ms"
  send_key "$session" "enter" "continue to Glyphs"
  wait_for_session_not_contains "$session" "Creation Workflow In Progress" "$timeout_ms"
  wait_for_session_contains "$session" "File: popup-glyph-source.png" "$timeout_ms"

  send_key "$session" "q" "quit"
  wait_exit "$session"
  session_cleanup "$session"
  current_session=""
  log "journey 6/${journey_count} passed"
}

journey_creation_grid() {
  log "journey 7/${journey_count}: creation workflow popup create grid"
  local workspace project_dir session session_home source_png imported_png manifest_path
  workspace="$(make_temp_dir "home-grid")"
  project_dir="$(create_empty_project "$workspace" "home-grid-demo")"
  session_home="$(create_session_home "$workspace")"
  source_png="$workspace/popup-grid-source.png"
  imported_png="$project_dir/images/popup-grid-source.png"
  manifest_path="$project_dir/petiglyph.toml"
  write_test_png "$source_png"
  session="petiglyph-e2e-home-grid-$$-$(date +%s%N)"

  run_petiglyph_session "$session" "$project_dir" "$session_home"
  focus_home_create_glyph_button "$session" "home grid"
  send_key "$session" "right" "focus Create grid button"
  send_key "$session" "enter" "start Create grid workflow"
  wait_for_session_contains "$session" "Creation Workflow In Progress" "$timeout_ms"
  send_bracketed_paste "$session" "$source_png" "paste source path"
  wait_for_path "$imported_png" "$timeout_ms"
  send_key "$session" "enter" "advance to tweak step"
  wait_for_session_contains "$session" "Tweak grayscale / threshold / preview" "$timeout_ms"
  send_key "$session" "enter" "continue to grid config"
  wait_for_session_contains "$session" "Create grid: adjust rows/columns/bleed here" "$timeout_ms"

  send_key "$session" "up" "rows 2 -> 3"
  send_key "$session" "right" "focus cols"
  send_key "$session" "up" "cols 2 -> 3"
  send_key "$session" "up" "cols 3 -> 4"
  send_key "$session" "right" "focus horizontal bleed"
  send_key "$session" "down" "horizontal weak -> off"
  send_key "$session" "right" "focus vertical bleed"
  send_key "$session" "up" "vertical off -> weak"
  send_key "$session" "up" "vertical weak -> strong"
  send_key "$session" "right" "focus create"
  send_key "$session" "enter" "create grid and switch to Glyphs"

  wait_for_file_contains "$manifest_path" '"popup-grid-source.png"' "$timeout_ms"
  wait_for_file_contains "$manifest_path" 'rows = 3' "$timeout_ms"
  wait_for_file_contains "$manifest_path" 'cols = 4' "$timeout_ms"
  wait_for_file_contains "$manifest_path" 'horizontal_bleed = "off"' "$timeout_ms"
  wait_for_file_contains "$manifest_path" 'vertical_bleed = "strong"' "$timeout_ms"

  send_key "$session" "q" "quit"
  wait_exit "$session"
  session_cleanup "$session"
  current_session=""
  log "journey 7/${journey_count} passed"
}

journey_creation_animated_glyph() {
  log "journey 8/${journey_count}: creation workflow popup create animated glyph"
  local workspace project_dir session session_home source_gif manifest_path
  workspace="$(make_temp_dir "home-animated")"
  project_dir="$(create_empty_project "$workspace" "home-animated-demo")"
  session_home="$(create_session_home "$workspace")"
  source_gif="$workspace/spinner.gif"
  manifest_path="$project_dir/petiglyph.toml"
  write_test_gif "$source_gif"
  session="petiglyph-e2e-home-animated-$$-$(date +%s%N)"

  run_petiglyph_session "$session" "$project_dir" "$session_home"
  focus_home_create_glyph_button "$session" "home animated"
  send_key "$session" "down" "focus Create animated glyph button"
  send_key "$session" "enter" "start Create animated glyph workflow"
  wait_for_session_contains "$session" "Creation Workflow In Progress" "$timeout_ms"
  send_bracketed_paste "$session" "$source_gif" "paste GIF path"
  wait_for_matching_file_count_ge "$project_dir/images" "spinner--pgf-*.png" 2 "$timeout_ms"

  send_key "$session" "enter" "advance to tweak step"
  wait_for_session_contains "$session" "Tweak grayscale / threshold / preview" "$timeout_ms"
  continue_from_tweak_until_animation_config "$session" "home animated"
  send_key "$session" "enter" "create animation and switch to Glyphs"

  wait_for_file_contains "$manifest_path" "[[animations]]" "$timeout_ms"
  wait_for_file_contains "$manifest_path" 'type = "standard"' "$timeout_ms"
  wait_for_file_contains "$manifest_path" "spinner--pgf-" "$timeout_ms"

  send_key "$session" "q" "quit"
  wait_exit "$session"
  session_cleanup "$session"
  current_session=""
  log "journey 8/${journey_count} passed"
}

journey_install_lifecycle() {
  log "journey 9/${journey_count}: install lifecycle"
  local workspace project_dir session session_home install_dir
  workspace="$(make_temp_dir "install-uninstall")"
  project_dir="$(create_project_with_icon "$workspace" "install-demo" "alpha.png")"
  session_home="$(create_session_home "$workspace")"
  install_dir="$session_home/.local/share/fonts/petiglyph"
  session="petiglyph-e2e-install-uninstall-$$-$(date +%s%N)"

  run_petiglyph_session "$session" "$project_dir" "$session_home"
  send_key_nowait "$session" "i" "install font"
  wait_for_matching_file_count_ge "$install_dir" "*.ttf" 1 "$timeout_ms"
  maybe_dismiss_first_install_notice "$session"

  send_key "$session" "q" "quit"
  wait_exit "$session"
  session_cleanup "$session"
  current_session=""
  log "journey 9/${journey_count} passed"
}

journey_full_user_story() {
  log "journey 10/${journey_count}: full multi-workflow lifecycle user story"

  local workspace session session_home install_dir
  local project_one_name project_two_name project_one_dir project_two_dir
  local project_one_manifest project_two_manifest
  local source_std source_grid source_anim source_anim_grid
  local map_path ttf_count_before ttf_count_after

  workspace="$(make_temp_dir "journey10-full-story")"
  session_home="$(create_session_home "$workspace")"
  install_dir="$session_home/.local/share/fonts/petiglyph"
  project_one_name="full-story-alpha"
  project_two_name="full-story-empty"
  project_one_dir="$workspace/$project_one_name"
  project_two_dir="$workspace/$project_two_name"
  project_one_manifest="$project_one_dir/petiglyph.toml"
  project_two_manifest="$project_two_dir/petiglyph.toml"
  source_std="$workspace/fullstory-standard.png"
  source_grid="$workspace/fullstory-grid.png"
  source_anim="$workspace/fullstory-anim.gif"
  source_anim_grid="$workspace/fullstory-anim-grid.gif"
  map_path="$project_one_dir/build/glyph-map.json"
  write_test_png "$source_std"
  write_test_png "$source_grid"
  write_test_gif "$source_anim"
  write_test_gif "$source_anim_grid"

  session="petiglyph-e2e-journey10-full-story-$$-$(date +%s%N)"
  run_petiglyph_session "$session" "$workspace" "$session_home"

  # Create first project.
  go_home_panel "$session"
  send_key "$session" "enter" "j10: focus create input"
  send_literal_keys "$session" "$project_one_name" "j10: type first project name"
  send_key "$session" "enter" "j10: submit first project create"
  wait_for_path "$project_one_manifest" "$timeout_ms"
  wait_for_path "$project_one_dir/images" "$timeout_ms"

  # Standard glyph creation + tweaks.
  focus_home_create_glyph_button "$session" "j10 standard glyph"
  send_key "$session" "enter" "j10: start standard glyph workflow"
  wait_for_session_contains "$session" "Creation Workflow In Progress" "$timeout_ms"
  send_bracketed_paste "$session" "$source_std" "j10: paste standard glyph source"
  wait_for_path "$project_one_dir/images/$(basename "$source_std")" "$timeout_ms"
  send_key "$session" "enter" "j10: standard glyph import -> tweak"
  workflow_tweak_threshold_then_continue "$session" "j10 standard glyph" "j10: complete standard glyph creation"
  wait_for_session_not_contains "$session" "Creation Workflow In Progress" "$timeout_ms"
  tweak_in_glyph_panel "$session" "j10 standard glyph"

  # Standard grid glyph creation + tweaks.
  focus_home_create_glyph_button "$session" "j10 standard grid"
  send_key "$session" "right" "j10: focus create grid button"
  send_key "$session" "enter" "j10: start standard grid workflow"
  wait_for_session_contains "$session" "Creation Workflow In Progress" "$timeout_ms"
  send_bracketed_paste "$session" "$source_grid" "j10: paste standard grid source"
  wait_for_path "$project_one_dir/images/$(basename "$source_grid")" "$timeout_ms"
  send_key "$session" "enter" "j10: standard grid import -> tweak"
  workflow_tweak_threshold_then_continue "$session" "j10 standard grid" "j10: continue to grid config"
  wait_for_session_contains "$session" "Create grid: adjust rows/columns/bleed here" "$timeout_ms"
  send_key "$session" "up" "j10: grid rows +1"
  send_key "$session" "right" "j10: grid focus cols"
  send_key "$session" "up" "j10: grid cols +1"
  send_key "$session" "right" "j10: grid focus horizontal bleed"
  send_key "$session" "down" "j10: grid horizontal bleed tweak"
  send_key "$session" "right" "j10: grid focus vertical bleed"
  send_key "$session" "up" "j10: grid vertical bleed tweak"
  send_key "$session" "right" "j10: grid focus create"
  send_key "$session" "enter" "j10: create standard grid glyph"
  wait_for_file_contains "$project_one_manifest" "\"$(basename "$source_grid")\"" "$timeout_ms"
  tweak_in_glyph_panel "$session" "j10 standard grid"

  # Animated glyph creation + tweaks.
  focus_home_create_glyph_button "$session" "j10 animated glyph"
  send_key "$session" "down" "j10: focus create animated glyph button"
  send_key "$session" "enter" "j10: start animated glyph workflow"
  wait_for_session_contains "$session" "Creation Workflow In Progress" "$timeout_ms"
  send_bracketed_paste "$session" "$source_anim" "j10: paste animated glyph GIF"
  wait_for_matching_file_count_ge "$project_one_dir/images" "fullstory-anim--pgf-*.png" 2 "$timeout_ms"
  send_key "$session" "enter" "j10: animated glyph import -> tweak"
  workflow_tweak_threshold_then_continue "$session" "j10 animated glyph" "j10: continue to animation config (attempt 1/1)"
  continue_from_tweak_until_animation_config "$session" "j10 animated glyph"
  send_key "$session" "up" "j10: animation fps +1"
  send_key "$session" "right" "j10: animation focus create"
  send_key "$session" "enter" "j10: create animated glyph"
  wait_for_file_contains "$project_one_manifest" "[[animations]]" "$timeout_ms"
  wait_for_file_contains "$project_one_manifest" "fullstory-anim--pgf-" "$timeout_ms"
  tweak_in_glyph_panel "$session" "j10 animated glyph"

  # Animated grid glyph creation + tweaks.
  focus_home_create_glyph_button "$session" "j10 animated grid glyph"
  send_key "$session" "down" "j10: focus create animated glyph row"
  send_key "$session" "right" "j10: focus create animated grid glyph button"
  send_key "$session" "enter" "j10: start animated grid glyph workflow"
  wait_for_session_contains "$session" "Creation Workflow In Progress" "$timeout_ms"
  send_bracketed_paste "$session" "$source_anim_grid" "j10: paste animated grid GIF"
  wait_for_matching_file_count_ge "$project_one_dir/images" "fullstory-anim-grid--pgf-*.png" 2 "$timeout_ms"
  send_key "$session" "enter" "j10: animated grid import -> tweak"
  workflow_tweak_threshold_then_continue "$session" "j10 animated grid glyph" "j10: continue to animated grid config (attempt 1/1)"
  continue_from_tweak_until_animation_config "$session" "j10 animated grid glyph"
  send_key "$session" "up" "j10: animated grid fps +1"
  send_key "$session" "right" "j10: animated grid focus rows"
  send_key "$session" "up" "j10: animated grid rows +1"
  send_key "$session" "right" "j10: animated grid focus cols"
  send_key "$session" "up" "j10: animated grid cols +1"
  send_key "$session" "right" "j10: animated grid focus horizontal bleed"
  send_key "$session" "down" "j10: animated grid horizontal bleed tweak"
  send_key "$session" "right" "j10: animated grid focus vertical bleed"
  send_key "$session" "up" "j10: animated grid vertical bleed tweak"
  send_key "$session" "right" "j10: animated grid focus create"
  send_key "$session" "enter" "j10: create animated grid glyph"
  wait_for_file_contains "$project_one_manifest" "fullstory-anim-grid--pgf-" "$timeout_ms"
  tweak_in_glyph_panel "$session" "j10 animated grid glyph"

  # Build + install via Home action row using arrows + Enter.
  focus_home_install_action "$session" "j10 install everything"
  send_key "$session" "enter" "j10: run install action from Home"
  wait_for_path "$map_path" "$timeout_ms"
  wait_for_matching_file_count_ge "$install_dir" "*.ttf" 1 "$timeout_ms"
  maybe_dismiss_first_install_notice "$session"

  # Sample area copy behavior via Enter on path + sample rows.
  focus_installed_fonts_list "$session" "j10 copy samples"
  send_key "$session" "enter" "j10: copy installed font path"
  wait_for_session_contains_any "$session" "$timeout_ms" "copied to clipboard" "clipboard copy failed:"
  send_key "$session" "down" "j10: move to sample row 1"
  send_key "$session" "enter" "j10: copy sample row 1"
  wait_for_session_contains_any "$session" "$timeout_ms" "copied to clipboard" "clipboard copy failed:"
  send_key "$session" "down" "j10: move to sample row 2"
  send_key "$session" "enter" "j10: copy sample row 2"
  wait_for_session_contains_any "$session" "$timeout_ms" "copied to clipboard" "clipboard copy failed:"

  # Create second empty project.
  focus_create_input_with_active_project "$session" "j10 second project"
  send_key "$session" "enter" "j10: focus create input for second project"
  send_literal_keys "$session" "$project_two_name" "j10: type second project name"
  send_key "$session" "enter" "j10: submit second project create"
  wait_for_path "$project_two_manifest" "$timeout_ms"
  wait_for_path "$project_two_dir/images" "$timeout_ms"

  # Delete each installed font from sample area.
  while true; do
    ttf_count_before="$(count_matching_files "$install_dir" "*.ttf")"
    if (( ttf_count_before == 0 )); then
      break
    fi
    focus_installed_fonts_list "$session" "j10 uninstall installed fonts"
    send_key "$session" "right" "j10: focus uninstall button"
    send_key "$session" "enter" "j10: uninstall selected installed font"
    ttf_count_after="$((ttf_count_before - 1))"
    wait_for_matching_file_count_eq "$install_dir" "*.ttf" "$ttf_count_after" "$timeout_ms"
  done

  # Delete project two (active), then reopen and delete project one.
  delete_active_project "$session" "j10 delete second project" "$timeout_ms"
  wait_for_session_contains_any "$session" "$timeout_ms" "deleted project" "opened project" "Petiglyph projects"
  [[ ! -d "$project_two_dir" ]] || {
    echo "Expected second project directory to be deleted: $project_two_dir" >&2
    return 1
  }

  open_first_project_from_home "$session" "j10 reopen first project"
  delete_active_project "$session" "j10 delete first project" "$timeout_ms"
  [[ ! -d "$project_one_dir" ]] || {
    echo "Expected first project directory to be deleted: $project_one_dir" >&2
    return 1
  }

  # Quit.
  send_key "$session" "q" "j10: quit application"
  wait_exit "$session"
  session_cleanup "$session"
  current_session=""
  log "journey 10/${journey_count} passed"
}

journey_render_probe() {
  log "render probe: exercise erase/default-background repaint behavior"
  local workspace session probe_script
  workspace="$(make_temp_dir "render-probe")"
  probe_script="$workspace/render_probe.sh"

  cat >"$probe_script" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

duration_ms="${1:-12000}"
start_ms="$(date +%s%3N)"
frame=0

pad_line() {
  local text="$1"
  printf '%-130s' "$text"
}

trap 'printf "\033[0m\033[?25h\n"' EXIT
printf '\033[?25l'

while true; do
  now_ms=""
  elapsed_ms=0
  bg=""
  now_ms="$(date +%s%3N)"
  elapsed_ms="$((now_ms - start_ms))"
  if (( elapsed_ms >= duration_ms )); then
    break
  fi

  frame=$((frame + 1))
  case $((frame % 6)) in
    0) bg="48;5;52" ;;
    1) bg="48;5;22" ;;
    2) bg="48;5;17" ;;
    3) bg="48;5;53" ;;
    4) bg="48;5;94" ;;
    *) bg="48;5;236" ;;
  esac

  printf '\033[H\033[2J'
  printf 'petiglyph hty render probe  frame=%03d  elapsed=%dms\n' "$frame" "$elapsed_ms"
  printf 'TERM=%s  COLORTERM=%s\n' "${TERM:-}" "${COLORTERM:-}"
  printf 'If black blocks vanish only when text overwrites them, suspect erase/default-bg mismatch.\n'
  printf 'Rows: A=full fill, B=EOL erase, C=default-bg rewrite, D+=checker rewrite.\n\n'

  printf '\033[%sm' "$bg"
  pad_line "A FULL-LINE FILL (colored background)"
  printf '\033[0m\n'

  printf '\033[%sm' "$bg"
  printf 'B EOL-ERASE TEST >>>>>>>>>>>>>>> frame=%03d ' "$frame"
  printf '\033[K\033[0m\n'

  printf '\033[49m'
  pad_line "C DEFAULT-BG REWRITE frame=$frame"
  printf '\033[0m\n'

  for ((row = 0; row < 10; row++)); do
    if (((row + frame) % 2 == 0)); then
      printf '\033[48;5;236m'
    else
      printf '\033[49m'
    fi
    pad_line "D CHECKER row=$row frame=$frame"
    printf '\033[0m\n'
  done

  printf '\n'
  printf 'Probe auto-exits after %dms.\n' "$duration_ms"
  sleep 0.12
done
EOF
  chmod +x "$probe_script"

  session="petiglyph-e2e-render-probe-$$-$(date +%s%N)"
  run_session \
    "$session" \
    "$workspace" \
    env TERM="$render_probe_term" COLORTERM="$render_probe_colorterm" \
    bash "$probe_script" "$render_probe_duration_ms"
  wait_exit "$session"
  session_cleanup "$session"
  current_session=""
  log "render probe passed"
}

cleanup() {
  local ec=$?
  set +e

  for pid in "${watch_pids[@]:-}"; do
    kill "$pid" >/dev/null 2>&1 || true
  done

  if (( ec != 0 )) && [[ -n "$current_session" ]]; then
    local hty_debug_cmd
    hty_debug_cmd="$(hty_command_for_shell)"
    echo >&2
    echo "Last active session: $current_session" >&2
    echo "Debug commands:" >&2
    echo "  $hty_debug_cmd snapshot $current_session --ansi" >&2
    echo "  $hty_debug_cmd logs $current_session | tail -n 120" >&2
    echo "  $hty_debug_cmd replay $current_session" >&2
  fi

  if (( ec == 0 )); then
    for session in "${sessions[@]:-}"; do
      session_cleanup "$session"
    done

    for dir in "${temp_dirs[@]:-}"; do
      rm -rf "$dir" >/dev/null 2>&1 || true
    done
  fi
}
trap cleanup EXIT

while [[ $# -gt 0 ]]; do
  case "$1" in
    --journey|-j)
      shift
      if [[ -z "${1:-}" ]]; then
        echo "Missing value for --journey" >&2
        exit 1
      fi
      parse_journey_selector "$1"
      ;;
    --watch)
      watch_enabled=1
      ;;
    --watch-terminal)
      shift
      if [[ -z "${1:-}" ]]; then
        echo "Missing value for --watch-terminal" >&2
        exit 1
      fi
      watch_terminal="${1:-}"
      ;;
    --watch-auto)
      watch_enabled=1
      log "warning: --watch-auto is deprecated; use --watch"
      ;;
    --render-probe)
      render_probe_enabled=1
      ;;
    --render-probe-only)
      render_probe_enabled=1
      render_probe_only=1
      ;;
    --render-probe-term)
      shift
      if [[ -z "${1:-}" ]]; then
        echo "Missing value for --render-probe-term" >&2
        exit 1
      fi
      render_probe_term="${1:-}"
      ;;
    --render-probe-colorterm)
      shift
      if [[ -z "${1:-}" ]]; then
        echo "Missing value for --render-probe-colorterm" >&2
        exit 1
      fi
      render_probe_colorterm="${1:-}"
      ;;
    --render-probe-duration-ms)
      shift
      if [[ -z "${1:-}" ]]; then
        echo "Missing value for --render-probe-duration-ms" >&2
        exit 1
      fi
      render_probe_duration_ms="${1:-}"
      ;;
    --timeout-ms)
      shift
      timeout_ms="${1:-}"
      ;;
    --step-delay-ms)
      shift
      step_delay_ms="${1:-}"
      ;;
    --petiglyph-bin)
      shift
      petiglyph_bin="${1:-}"
      ;;
    --hty-bin)
      shift
      hty_bin="${1:-}"
      ;;
    --keep-sessions)
      keep_sessions=1
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage
      exit 1
      ;;
  esac
  shift
done

case "$watch_terminal" in
  auto|ghostty|kitty|alacritty|foot|tmux)
    ;;
  *)
    echo "Invalid --watch-terminal value: $watch_terminal (expected auto|ghostty|kitty|alacritty|foot|tmux)" >&2
    exit 1
    ;;
esac

require_command "$hty_bin"
setup_hty_env

if [[ -z "$petiglyph_bin" ]]; then
  petiglyph_bin="$repo_root/target/debug/petiglyph"
fi
if [[ ! -x "$petiglyph_bin" ]]; then
  log "building petiglyph binary at $petiglyph_bin"
  (cd "$repo_root" && cargo build --quiet --bin petiglyph)
fi
if [[ ! -x "$petiglyph_bin" ]]; then
  echo "petiglyph binary is not executable: $petiglyph_bin" >&2
  exit 1
fi

# CI runners can have bursty scheduling around PTY rendering/input delivery.
# Apply slightly safer timing defaults only when not explicitly overridden.
if [[ "${GITHUB_ACTIONS:-}" == "true" ]]; then
  if [[ -z "${PETIGLYPH_E2E_TIMEOUT_MS_OVERRIDE:-}" && "$timeout_ms" -lt 30000 ]]; then
    timeout_ms=30000
  fi
  if [[ -z "${PETIGLYPH_E2E_STEP_DELAY_MS_OVERRIDE:-}" && "$step_delay_ms" -lt 100 ]]; then
    step_delay_ms=100
  fi
fi

log "hty binary: $hty_bin"
log "petiglyph binary: $petiglyph_bin"
log "timeout: ${timeout_ms}ms, startup-wait: ${startup_wait_ms}ms, step-delay: ${step_delay_ms}ms, watch: ${watch_enabled}, watch-terminal: ${watch_terminal}, keep sessions: ${keep_sessions}"
if (( render_probe_enabled == 1 )); then
  log "render probe: enabled (only=${render_probe_only}, TERM=${render_probe_term}, COLORTERM=${render_probe_colorterm}, duration=${render_probe_duration_ms}ms)"
fi
if (( ${#selected_journeys[@]} > 0 )); then
  log "selected journeys: ${selected_journeys[*]}"
fi

if (( render_probe_enabled == 1 )); then
  journey_render_probe
fi

if (( render_probe_only == 1 )); then
  log "render probe only mode complete"
  exit 0
fi

if should_run_journey 1; then
  journey_launch_and_quit
fi
if should_run_journey 2; then
  journey_create_project_from_home
fi
if should_run_journey 3; then
  journey_build_and_rescan
fi
if should_run_journey 4; then
  journey_threshold_roundtrip
fi
if should_run_journey 5; then
  journey_workspace_selection
fi
if should_run_journey 6; then
  journey_creation_glyph
fi
if should_run_journey 7; then
  journey_creation_grid
fi
if should_run_journey 8; then
  journey_creation_animated_glyph
fi
if should_run_journey 9; then
  journey_install_lifecycle
fi
if should_run_journey 10; then
  journey_full_user_story
fi

log "all journeys passed"
