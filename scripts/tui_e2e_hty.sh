#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

hty_bin="${HTY_BIN:-hty}"
petiglyph_bin="${PETIGLYPH_BIN:-}"
timeout_ms=10000
startup_wait_ms=1500
step_delay_ms=0
watch_enabled=0
watch_auto_enabled=0
keep_sessions=0

declare -a sessions=()
declare -a watch_pids=()
declare -a temp_dirs=()
declare -a selected_journeys=()
current_session=""

usage() {
  cat <<'EOF'
Usage:
  ./scripts/tui_e2e_hty.sh [options]

Runs process-level TUI E2E journeys with hty:
  1. launch + quit from existing project
  2. create project from Home panel
  3. build shortcut writes artifacts
  4. glyph threshold override persists and clears
  5. workspace multi-project selection builds chosen project
  6. rescan picks up new image and rebuild includes it
  7. multi-project create/build/install/uninstall lifecycle

Options:
  --journey N[,M...]      Run only selected journey number(s) (1-7). Repeatable.
  --watch                 Pause before each journey step so you can run `hty watch <session>`
                          or `hty attach <session>` in another terminal
  --watch-auto            Auto-open a watcher terminal for each session (best effort)
  --keep-sessions         Keep hty sessions/logs after script exits (no hty delete)
  --timeout-ms N          Wait timeout in ms for hty waits and file polling (default: 10000)
  --step-delay-ms N       Delay in ms after each send step (default: 0)
  --petiglyph-bin PATH    Path to petiglyph binary (default: target/debug/petiglyph, auto-build if missing)
  --hty-bin PATH          Path to hty binary (default: hty from PATH)
  -h, --help              Show this help

Environment:
  HTY_BIN         Same as --hty-bin
  PETIGLYPH_BIN   Same as --petiglyph-bin

Example:
  ./scripts/tui_e2e_hty.sh --journey 7
  ./scripts/tui_e2e_hty.sh --journey 2,5 --journey 7
  ./scripts/tui_e2e_hty.sh --watch --step-delay-ms 250
  ./scripts/tui_e2e_hty.sh --watch-auto --step-delay-ms 250
EOF
}

log() {
  printf '[tui-e2e-hty] %s\n' "$*"
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
    if [[ ! "$id" =~ ^[1-7]$ ]]; then
      echo "Invalid journey id: $id (expected 1-7)" >&2
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

require_command() {
  local cmd="$1"
  if ! command -v "$cmd" >/dev/null 2>&1; then
    echo "Missing required command: $cmd" >&2
    exit 1
  fi
}

sleep_step_delay() {
  if (( step_delay_ms > 0 )); then
    sleep "$(awk "BEGIN { printf \"%.3f\", ${step_delay_ms}/1000 }")"
  fi
}

register_temp_dir() {
  local dir="$1"
  temp_dirs+=("$dir")
}

make_temp_dir() {
  local name="$1"
  local dir
  dir="$(mktemp -d "/tmp/petiglyph-tui-e2e-hty-${name}-XXXXXX")"
  register_temp_dir "$dir"
  printf '%s\n' "$dir"
}

write_test_png() {
  local out_path="$1"
  # 1x1 black PNG
  base64 -d >"$out_path" <<'EOF'
iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+ip1sAAAAASUVORK5CYII=
EOF
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
    if [[ "$current" == "$expected" ]]; then
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

start_watch_if_enabled() {
  local session="$1"
  if (( watch_enabled == 0 && watch_auto_enabled == 0 )); then
    return
  fi

  if (( watch_auto_enabled == 1 )); then
    local watcher_cmd
    watcher_cmd="$(printf '%q watch %q' "$hty_bin" "$session")"
    if start_watcher_terminal "$watcher_cmd" "$session"; then
      log "auto watcher started for '$session'"
      sleep 0.2
      return
    fi
    log "failed to auto-open watcher terminal; falling back to manual watch mode"
  fi

  echo
  log "watch mode for session '$session'"
  echo "  In another terminal, run one of:"
  echo "    $hty_bin watch $session    # read-only live view"
  echo "    $hty_bin attach $session   # live view + interactive takeover"
  if [[ -t 0 ]]; then
    read -r -p "Press Enter here after attaching watcher... " _
  else
    log "non-interactive shell: continuing without pause"
  fi
}

start_watcher_terminal() {
  local watcher_cmd="$1"
  local session="$2"
  local title="petiglyph watch: $session"

  if command -v kitty >/dev/null 2>&1; then
    kitty --title "$title" bash -lc "$watcher_cmd; echo; read -r -p 'Press Enter to close...' _" >/dev/null 2>&1 &
    watch_pids+=("$!")
    return 0
  fi

  if command -v ghostty >/dev/null 2>&1; then
    ghostty --title="$title" -e "$hty_bin" watch "$session" >/dev/null 2>&1 &
    watch_pids+=("$!")
    return 0
  fi

  if command -v alacritty >/dev/null 2>&1; then
    alacritty --title "$title" -e bash -lc "$watcher_cmd; echo; read -r -p 'Press Enter to close...' _" >/dev/null 2>&1 &
    watch_pids+=("$!")
    return 0
  fi

  if command -v foot >/dev/null 2>&1; then
    foot --title "$title" bash -lc "$watcher_cmd; echo; read -r -p 'Press Enter to close...' _" >/dev/null 2>&1 &
    watch_pids+=("$!")
    return 0
  fi

  if [[ -n "${TMUX:-}" ]] && command -v tmux >/dev/null 2>&1; then
    tmux split-window -v "$watcher_cmd"
    return 0
  fi

  return 1
}

session_cleanup() {
  local session="$1"
  if (( keep_sessions == 1 )); then
    return
  fi
  "$hty_bin" delete "$session" >/dev/null 2>&1 || true
}

send_text() {
  local session="$1"
  local text="$2"
  local label="$3"
  log "send [$session] $label"
  "$hty_bin" send "$session" --text "$text" >/dev/null
  sleep_step_delay
}

send_key() {
  local session="$1"
  local key="$2"
  local label="$3"
  log "send [$session] $label"
  "$hty_bin" send "$session" --key "$key" >/dev/null
  sleep_step_delay
}

run_session() {
  local session="$1"
  local cwd="$2"
  shift 2
  current_session="$session"
  sessions+=("$session")
  "$hty_bin" run \
    --name "$session" \
    --cwd "$cwd" \
    --rows 42 \
    --cols 140 \
    -- \
    "$@"
  # Wait briefly for the first render. Use explicit wait command because some
  # hty builds parse --wait-duration differently.
  "$hty_bin" wait "$session" --idle 120 --timeout "$startup_wait_ms" >/dev/null 2>/dev/null || true
  "$hty_bin" snapshot "$session" >/dev/null || true
  start_watch_if_enabled "$session"
}

wait_exit() {
  local session="$1"
  "$hty_bin" wait "$session" --exit --timeout "$timeout_ms" >/dev/null
}

wait_session_idle() {
  local session="$1"
  local idle_ms="${2:-250}"
  local timeout="${3:-$timeout_ms}"
  "$hty_bin" wait "$session" --idle "$idle_ms" --timeout "$timeout" >/dev/null 2>/dev/null || true
}

session_snapshot_text() {
  local session="$1"
  "$hty_bin" snapshot "$session" --ansi 2>/dev/null \
    | awk '{ gsub(/\033\[[0-9;?]*[ -\/]*[@-~]/, ""); print }' || true
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

dismiss_first_install_notice() {
  local session="$1"
  local timeout="$2"
  wait_for_session_contains "$session" "First Install Guidance" "$timeout"
  send_key "$session" "space" "dismiss first-install guidance popup"
  wait_for_session_not_contains "$session" "First Install Guidance" "$timeout"
}

dismiss_delete_project_confirmation_if_open() {
  local session="$1"
  if session_snapshot_text "$session" | grep -Fq "Confirm Deletion"; then
    send_key "$session" "esc" "dismiss delete-project confirmation popup"
    wait_for_session_not_contains "$session" "Confirm Deletion" "$timeout_ms"
  fi
}

focus_installed_fonts_list() {
  local session="$1"
  local prefix="$2"
  local project_count="$3"
  local down_steps=$((project_count + 2))
  local idx

  # Traverse vertically to avoid accidentally entering the delete-project slot puzzle.
  # Normalize to project list first, then take exact steps into installed-font list.
  for idx in 1 2 3 4 5; do
    send_key "$session" "up" "$prefix: normalize to project list ($idx/5)"
  done
  for ((idx = 1; idx <= down_steps; idx++)); do
    send_key "$session" "down" "$prefix: move toward installed fonts list ($idx/$down_steps)"
  done
}

focus_create_input_for_single_project_workspace() {
  local session="$1"
  local prefix="$2"
  # Journey 7 has exactly one project at this point.
  # Normalize to project list, then one step down reaches create input.
  send_key "$session" "up" "$prefix: normalize to project list (1/5)"
  send_key "$session" "up" "$prefix: normalize to project list (2/5)"
  send_key "$session" "up" "$prefix: normalize to project list (3/5)"
  send_key "$session" "up" "$prefix: normalize to project list (4/5)"
  send_key "$session" "up" "$prefix: normalize to project list (5/5)"
  send_key "$session" "down" "$prefix: focus create input"
}

wait_for_session_background_tasks_done() {
  local session="$1"
  local timeout="$2"
  wait_for_session_not_contains "$session" "Building..." "$timeout"
  wait_for_session_not_contains "$session" "Installing..." "$timeout"
  wait_for_session_not_contains "$session" "Removing..." "$timeout"
  wait_for_session_not_contains "$session" "background task is in progress" "$timeout"
}

select_jpg_fixture() {
  local icons_dir="$1"
  local png_fallback="$2"
  local out_dir="$3"

  local candidate
  for candidate in "$icons_dir"/*.jpg "$icons_dir"/*.jpeg; do
    [[ -f "$candidate" ]] || continue
    if command -v file >/dev/null 2>&1; then
      if file --brief --mime-type "$candidate" 2>/dev/null | grep -Fxq "image/jpeg"; then
        printf '%s\n' "$candidate"
        return 0
      fi
      continue
    fi
    # If `file` is unavailable, optimistically try the first .jpg/.jpeg found.
    printf '%s\n' "$candidate"
    return 0
  done

  local generated="$out_dir/derived-from-icons.jpg"
  if command -v magick >/dev/null 2>&1; then
    magick "$png_fallback" -quality 95 "$generated"
    printf '%s\n' "$generated"
    return 0
  fi
  if command -v convert >/dev/null 2>&1; then
    convert "$png_fallback" -quality 95 "$generated"
    printf '%s\n' "$generated"
    return 0
  fi

  echo "No decodable JPG found in $icons_dir and no magick/convert available to derive one" >&2
  return 1
}

assert_current_project_outputs_built() {
  local session="$1"
  local timeout="$2"
  wait_for_session_contains "$session" "TTF: built:" "$timeout"
  wait_for_session_not_contains "$session" "TTF: pending:" "$timeout"
}

create_project_with_icon() {
  local workspace="$1"
  local project_name="$2"
  (
    cd "$workspace"
    "$petiglyph_bin" create "$project_name" --no-launch >/dev/null
  )
  local project_dir="$workspace/$project_name"
  mkdir -p "$project_dir/icons"
  write_test_png "$project_dir/icons/alpha.png"
  printf '%s\n' "$project_dir"
}

journey_launch_and_quit() {
  log "journey 1/7: launch and quit from existing project"
  local workspace project_dir session
  workspace="$(make_temp_dir "launch-quit")"
  project_dir="$(create_project_with_icon "$workspace" "launch-quit-demo")"
  session="petiglyph-e2e-launch-quit-$$-$(date +%s%N)"

  run_session "$session" "$project_dir" "$petiglyph_bin" >/dev/null
  send_text "$session" "q" "quit"
  wait_exit "$session"
  session_cleanup "$session"
  current_session=""
  log "journey 1/7 passed"
}

journey_create_project_from_home() {
  log "journey 2/7: create project from home panel"
  local workspace session project_name
  workspace="$(make_temp_dir "create-home")"
  project_name="from-tui-e2e"
  session="petiglyph-e2e-create-home-$$-$(date +%s%N)"

  run_session "$session" "$workspace" "$petiglyph_bin" >/dev/null

  send_key "$session" "enter" "enter typing mode"
  send_text "$session" "$project_name" "type project name"
  send_key "$session" "enter" "focus create button"
  send_key "$session" "enter" "submit create"
  send_text "$session" "q" "quit"
  wait_exit "$session"

  [[ -d "$workspace/$project_name" ]] || {
    echo "Expected project directory missing: $workspace/$project_name" >&2
    return 1
  }
  [[ -f "$workspace/$project_name/petiglyph.toml" ]] || {
    echo "Expected manifest missing: $workspace/$project_name/petiglyph.toml" >&2
    return 1
  }
  [[ -d "$workspace/$project_name/icons" ]] || {
    echo "Expected icons dir missing: $workspace/$project_name/icons" >&2
    return 1
  }

  session_cleanup "$session"
  current_session=""
  log "journey 2/7 passed"
}

journey_build_shortcut() {
  log "journey 3/7: build shortcut writes artifacts"
  local workspace project_dir session build_dir mapping sample ttf
  workspace="$(make_temp_dir "build")"
  project_dir="$(create_project_with_icon "$workspace" "build-demo")"
  build_dir="$project_dir/build"
  mapping="$build_dir/glyph-map.json"
  sample="$build_dir/glyph-sample.txt"
  session="petiglyph-e2e-build-$$-$(date +%s%N)"

  run_session "$session" "$project_dir" "$petiglyph_bin" >/dev/null
  send_text "$session" "b" "trigger build"

  wait_for_path "$mapping" "$timeout_ms"
  wait_for_path "$sample" "$timeout_ms"
  ttf="$(wait_for_ttf "$build_dir" "$timeout_ms")"
  [[ -n "$ttf" ]] || {
    echo "Expected .ttf output missing in $build_dir" >&2
    return 1
  }

  send_text "$session" "q" "quit"
  wait_exit "$session"
  session_cleanup "$session"
  current_session=""
  log "journey 3/7 passed (ttf: $ttf)"
}

journey_glyph_threshold_override_roundtrip() {
  log "journey 4/7: glyph threshold override persists and clears"
  local workspace project_dir session manifest_path
  workspace="$(make_temp_dir "threshold")"
  project_dir="$(create_project_with_icon "$workspace" "threshold-demo")"
  manifest_path="$project_dir/petiglyph.toml"
  session="petiglyph-e2e-threshold-$$-$(date +%s%N)"

  run_session "$session" "$project_dir" "$petiglyph_bin" >/dev/null
  send_text "$session" "2" "switch to glyphs panel"
  send_key "$session" "right" "increase threshold by 1"
  wait_for_file_contains "$manifest_path" "threshold_overrides" "$timeout_ms"
  wait_for_file_contains "$manifest_path" "alpha.png" "$timeout_ms"
  send_text "$session" "r" "clear threshold override"
  wait_for_file_not_contains "$manifest_path" "alpha.png" "$timeout_ms"
  wait_for_file_not_contains "$manifest_path" "threshold_overrides" "$timeout_ms"
  send_text "$session" "q" "quit"
  wait_exit "$session"
  session_cleanup "$session"
  current_session=""
  log "journey 4/7 passed"
}

journey_workspace_multi_project_selection() {
  log "journey 5/7: workspace multi-project selection builds chosen project"
  local workspace project_a project_b session project_a_build project_b_build ttf_b
  workspace="$(make_temp_dir "workspace-select")"
  project_a="$(create_project_with_icon "$workspace" "project-one")"
  project_b="$(create_project_with_icon "$workspace" "project-two")"
  project_a_build="$project_a/build/glyph-map.json"
  project_b_build="$project_b/build/glyph-map.json"
  session="petiglyph-e2e-workspace-select-$$-$(date +%s%N)"

  run_session "$session" "$workspace" "$petiglyph_bin" >/dev/null
  send_key "$session" "down" "select second project in list"
  send_key "$session" "enter" "open selected project"
  send_text "$session" "b" "build selected project"
  wait_for_path "$project_b_build" "$timeout_ms"
  ttf_b="$(wait_for_ttf "$project_b/build" "$timeout_ms")"
  [[ ! -f "$project_a_build" ]] || {
    echo "Unexpected build output in unselected project: $project_a_build" >&2
    return 1
  }
  [[ -n "$ttf_b" ]] || {
    echo "Expected .ttf output missing in selected project: $project_b/build" >&2
    return 1
  }
  send_text "$session" "q" "quit"
  wait_exit "$session"
  session_cleanup "$session"
  current_session=""
  log "journey 5/7 passed (ttf: $ttf_b)"
}

journey_rescan_new_image_and_rebuild() {
  log "journey 6/7: rescan picks up new image and rebuild includes it"
  local workspace project_dir session build_dir mapping sample initial_ttf ttf
  workspace="$(make_temp_dir "rescan")"
  project_dir="$(create_project_with_icon "$workspace" "rescan-demo")"
  build_dir="$project_dir/build"
  mapping="$build_dir/glyph-map.json"
  sample="$build_dir/glyph-sample.txt"
  session="petiglyph-e2e-rescan-$$-$(date +%s%N)"

  run_session "$session" "$project_dir" "$petiglyph_bin" >/dev/null
  send_text "$session" "b" "initial build"
  wait_for_path "$mapping" "$timeout_ms"
  wait_for_path "$sample" "$timeout_ms"
  initial_ttf="$(wait_for_ttf "$build_dir" "$timeout_ms")"
  [[ -n "$initial_ttf" ]] || {
    echo "Expected initial .ttf output missing in $build_dir" >&2
    return 1
  }
  "$hty_bin" wait "$session" --idle 200 --timeout "$timeout_ms" >/dev/null 2>/dev/null || true
  wait_for_file_contains "$mapping" "\"source_file\": \"alpha.png\"" "$timeout_ms"

  write_test_png "$project_dir/icons/beta.png"
  send_text "$session" "R" "rescan project icons"
  send_text "$session" "b" "rebuild after rescan"
  wait_for_file_contains "$mapping" "\"source_file\": \"beta.png\"" "$timeout_ms"
  ttf="$(wait_for_ttf "$build_dir" "$timeout_ms")"
  [[ -n "$ttf" ]] || {
    echo "Expected .ttf output missing after rescan in $build_dir" >&2
    return 1
  }

  send_text "$session" "q" "quit"
  wait_exit "$session"
  session_cleanup "$session"
  current_session=""
  log "journey 6/7 passed (ttf: $ttf)"
}

journey_multi_project_lifecycle() {
  log "journey 7/7: single-session multi-project lifecycle with real-image rescan"
  local workspace session_home install_dir
  local project_one project_two project_one_dir project_two_dir manifest_one manifest_two
  local project_one_build project_two_build
  local icons_dir png_source svg_source jpg_source
  local session ttf_count_one ttf_count_two
  workspace="$(make_temp_dir "mega-lifecycle")"
  session_home="$workspace/fake-home"
  install_dir="$session_home/.local/share/fonts/petiglyph"
  mkdir -p "$session_home/.config"

  icons_dir="$repo_root/icons"
  png_source="$icons_dir/codex.png"
  svg_source="$icons_dir/copilot.svg"
  jpg_source="$(select_jpg_fixture "$icons_dir" "$png_source" "$workspace")"
  [[ -f "$png_source" ]] || {
    echo "Expected PNG test fixture missing: $png_source" >&2
    return 1
  }
  [[ -f "$svg_source" ]] || {
    echo "Expected SVG test fixture missing: $svg_source" >&2
    return 1
  }
  [[ -f "$jpg_source" ]] || {
    echo "Expected JPG test fixture missing or not derivable from repo icons" >&2
    return 1
  }
  log "journey 7 jpg fixture: $jpg_source"

  project_one="nav-only-project"
  project_two="mixed-nav-shortcuts-project"
  project_one_dir="$workspace/$project_one"
  project_two_dir="$workspace/$project_two"
  manifest_one="$project_one_dir/petiglyph.toml"
  manifest_two="$project_two_dir/petiglyph.toml"
  project_one_build="$project_one_dir/build/glyph-map.json"
  project_two_build="$project_two_dir/build/glyph-map.json"

  session="petiglyph-e2e-mega-lifecycle-$$-$(date +%s%N)"
  run_session "$session" "$workspace" env HOME="$session_home" XDG_CONFIG_HOME="$session_home/.config" "$petiglyph_bin" >/dev/null
  wait_for_session_contains "$session" "Petiglyph projects" "$timeout_ms"
  wait_for_session_contains "$session" "Current project" "$timeout_ms"

  send_key "$session" "enter" "project one: enter typing mode"
  send_text "$session" "$project_one" "project one: type project name"
  send_key "$session" "enter" "project one: focus create button"
  send_key "$session" "enter" "project one: submit create"
  wait_for_path "$manifest_one" "$timeout_ms"
  wait_for_path "$project_one_dir/icons" "$timeout_ms"

  cp "$png_source" "$project_one_dir/icons/a-png.png"
  cp "$svg_source" "$project_one_dir/icons/b-svg.svg"
  send_key "$session" "up" "project one: focus project list"
  send_key "$session" "enter" "project one: reopen selected project to load images"

  send_key "$session" "tab" "project one: switch to glyphs"
  send_key "$session" "right" "project one: increase first glyph threshold"
  send_key "$session" "down" "project one: move to second glyph"
  wait_for_file_contains "$manifest_one" "threshold_overrides" "$timeout_ms"
  wait_for_file_contains "$manifest_one" "a-png.png" "$timeout_ms"
  wait_for_file_not_contains "$manifest_one" "b-svg.svg" "$timeout_ms"
  send_key "$session" "tab" "project one: switch back to home"

  send_key "$session" "enter" "project one: build via focused action (nav-only)"
  wait_for_path "$project_one_build" "$timeout_ms"
  wait_for_file_contains "$project_one_build" "\"source_file\": \"a-png.png\"" "$timeout_ms"
  wait_for_file_contains "$project_one_build" "\"source_file\": \"b-svg.svg\"" "$timeout_ms"
  wait_session_idle "$session" 300 "$timeout_ms"
  assert_current_project_outputs_built "$session" "$timeout_ms"

  send_key "$session" "right" "project one: move to install action"
  send_key "$session" "enter" "project one: install via focused action (nav-only)"
  wait_for_matching_file_count_ge "$install_dir" "*.ttf" 1 "$timeout_ms"
  wait_session_idle "$session" 300 "$timeout_ms"
  wait_for_session_background_tasks_done "$session" "$timeout_ms"
  dismiss_first_install_notice "$session" "$timeout_ms"
  ttf_count_one="$(count_matching_files "$install_dir" "*.ttf")"

  cp "$jpg_source" "$project_one_dir/icons/c-jpg.jpg"
  send_text "$session" "R" "project one: rescan after adding third image"
  wait_session_idle "$session" 300 "$timeout_ms"
  wait_for_session_background_tasks_done "$session" "$timeout_ms"

  send_key "$session" "left" "project one: move back to build action"
  send_key "$session" "enter" "project one: rebuild via focused action (nav-only)"
  wait_for_file_contains "$project_one_build" "\"source_file\": \"c-jpg.jpg\"" "$timeout_ms"
  wait_session_idle "$session" 300 "$timeout_ms"
  wait_for_session_background_tasks_done "$session" "$timeout_ms"
  assert_current_project_outputs_built "$session" "$timeout_ms"

  send_key "$session" "right" "project one: move to reinstall action"
  send_key "$session" "enter" "project one: reinstall via focused action (nav-only)"
  wait_for_matching_file_count_ge "$install_dir" "*.ttf" 1 "$timeout_ms"
  wait_session_idle "$session" 300 "$timeout_ms"
  wait_for_session_background_tasks_done "$session" "$timeout_ms"

  focus_installed_fonts_list "$session" "project one" 1
  send_key "$session" "enter" "project one: uninstall selected font from list"
  dismiss_delete_project_confirmation_if_open "$session"
  wait_for_matching_file_count_eq "$install_dir" "*.ttf" 0 "$timeout_ms"
  wait_session_idle "$session" 300 "$timeout_ms"
  wait_for_session_background_tasks_done "$session" "$timeout_ms"

  send_key "$session" "right" "project one: move to install action again"
  send_key "$session" "enter" "project one: install again via focused action (nav-only)"
  wait_for_matching_file_count_ge "$install_dir" "*.ttf" 1 "$timeout_ms"
  wait_session_idle "$session" 300 "$timeout_ms"
  wait_for_session_background_tasks_done "$session" "$timeout_ms"

  focus_create_input_for_single_project_workspace "$session" "project two"

  send_key "$session" "enter" "project two: enter typing mode"
  send_text "$session" "$project_two" "project two: type project name"
  send_key "$session" "enter" "project two: focus create button"
  send_key "$session" "enter" "project two: submit create"
  wait_for_path "$manifest_two" "$timeout_ms"
  wait_for_path "$project_two_dir/icons" "$timeout_ms"

  cp "$project_one_dir/icons/a-png.png" "$project_two_dir/icons/shared-png.png"
  cp "$svg_source" "$project_two_dir/icons/fresh-svg.svg"
  cp "$jpg_source" "$project_two_dir/icons/fresh-jpg.jpg"

  send_text "$session" "R" "project two: rescan icons after adding 3 images"
  send_text "$session" "2" "project two: switch to glyphs view"
  send_key "$session" "right" "project two: increase one glyph threshold"
  wait_for_file_contains "$manifest_two" "threshold_overrides" "$timeout_ms"
  wait_for_file_contains "$manifest_two" "fresh-jpg.jpg" "$timeout_ms"
  wait_for_file_not_contains "$manifest_two" "fresh-svg.svg" "$timeout_ms"
  wait_for_file_not_contains "$manifest_two" "shared-png.png" "$timeout_ms"

  send_text "$session" "b" "project two: build shortcut"
  wait_for_path "$project_two_build" "$timeout_ms"
  wait_for_file_contains "$project_two_build" "\"source_file\": \"fresh-jpg.jpg\"" "$timeout_ms"
  wait_for_file_contains "$project_two_build" "\"source_file\": \"fresh-svg.svg\"" "$timeout_ms"
  wait_for_file_contains "$project_two_build" "\"source_file\": \"shared-png.png\"" "$timeout_ms"
  wait_session_idle "$session" 300 "$timeout_ms"
  send_text "$session" "1" "project two: return to home for render assertion"
  assert_current_project_outputs_built "$session" "$timeout_ms"

  send_text "$session" "i" "project two: install shortcut"
  wait_for_matching_file_count_ge "$install_dir" "*.ttf" 2 "$timeout_ms"
  wait_session_idle "$session" 300 "$timeout_ms"
  wait_for_session_background_tasks_done "$session" "$timeout_ms"
  send_text "$session" "1" "project two: return to home view"

  focus_installed_fonts_list "$session" "project two" 2
  send_key "$session" "enter" "project two: uninstall selected font from list"
  dismiss_delete_project_confirmation_if_open "$session"
  wait_for_matching_file_count_eq "$install_dir" "*.ttf" 1 "$timeout_ms"
  wait_session_idle "$session" 300 "$timeout_ms"
  wait_for_session_background_tasks_done "$session" "$timeout_ms"

  send_text "$session" "b" "project two: rebuild shortcut"
  wait_for_path "$project_two_build" "$timeout_ms"
  wait_session_idle "$session" 300 "$timeout_ms"
  wait_for_session_background_tasks_done "$session" "$timeout_ms"
  assert_current_project_outputs_built "$session" "$timeout_ms"
  send_text "$session" "i" "project two: install again shortcut"
  wait_for_matching_file_count_ge "$install_dir" "*.ttf" 2 "$timeout_ms"
  wait_session_idle "$session" 300 "$timeout_ms"
  wait_for_session_background_tasks_done "$session" "$timeout_ms"
  ttf_count_two="$(count_matching_files "$install_dir" "*.ttf")"

  send_text "$session" "q" "quit journey 7 session"
  wait_exit "$session"
  session_cleanup "$session"
  current_session=""

  (( ttf_count_one >= 1 )) || {
    echo "Expected at least one installed TTF after project one install cycle (got $ttf_count_one)" >&2
    return 1
  }
  (( ttf_count_two >= 2 )) || {
    echo "Expected at least two installed TTFs after project two final install (got $ttf_count_two)" >&2
    return 1
  }

  log "journey 7/7 passed (isolated install dir: $install_dir, final ttf count: $ttf_count_two)"
}

cleanup() {
  local ec=$?
  set +e

  for pid in "${watch_pids[@]:-}"; do
    kill "$pid" >/dev/null 2>&1 || true
  done

  for session in "${sessions[@]:-}"; do
    session_cleanup "$session"
  done

  for dir in "${temp_dirs[@]:-}"; do
    rm -rf "$dir" >/dev/null 2>&1 || true
  done

  if (( ec != 0 )) && [[ -n "$current_session" ]]; then
    echo >&2
    echo "Last active session: $current_session" >&2
    echo "Debug commands:" >&2
    echo "  $hty_bin snapshot $current_session --ansi" >&2
    echo "  $hty_bin logs $current_session | tail -n 100" >&2
    echo "  $hty_bin replay $current_session" >&2
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
    --watch-auto)
      watch_auto_enabled=1
      ;;
    --keep-sessions)
      keep_sessions=1
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

require_command "$hty_bin"
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

log "hty binary: $hty_bin"
log "petiglyph binary: $petiglyph_bin"
log "timeout: ${timeout_ms}ms, startup-wait: ${startup_wait_ms}ms, step delay: ${step_delay_ms}ms, watch: ${watch_enabled}, watch-auto: ${watch_auto_enabled}, keep sessions: ${keep_sessions}"
if (( ${#selected_journeys[@]} > 0 )); then
  log "selected journeys: ${selected_journeys[*]}"
fi

if should_run_journey 1; then
  journey_launch_and_quit
fi
if should_run_journey 2; then
  journey_create_project_from_home
fi
if should_run_journey 3; then
  journey_build_shortcut
fi
if should_run_journey 4; then
  journey_glyph_threshold_override_roundtrip
fi
if should_run_journey 5; then
  journey_workspace_multi_project_selection
fi
if should_run_journey 6; then
  journey_rescan_new_image_and_rebuild
fi
if should_run_journey 7; then
  journey_multi_project_lifecycle
fi

log "all journeys passed"
