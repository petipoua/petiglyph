#!/usr/bin/env bash
set -euo pipefail

retry() {
  local attempts="$1"
  shift
  local delay="$1"
  shift
  local attempt=1
  while true; do
    if "$@"; then
      return 0
    fi
    if [[ "$attempt" -ge "$attempts" ]]; then
      return 1
    fi
    echo "Attempt $attempt failed; retrying in ${delay}s..." >&2
    sleep "$delay"
    attempt=$((attempt + 1))
  done
}

install_ffmpeg_windows() {
  resolve_ffmpeg_bin_windows() {
    powershell.exe -NoLogo -NoProfile -NonInteractive -Command '
      $ErrorActionPreference = "SilentlyContinue"
      $cmd = Get-Command ffmpeg.exe
      if ($cmd) {
        Split-Path -Parent $cmd.Source
        exit 0
      }

      $roots = @(
        "C:\ProgramData\chocolatey\lib",
        "C:\ProgramData\chocolatey\bin",
        "C:\Program Files",
        (Join-Path $env:LOCALAPPDATA "Microsoft\WinGet\Packages")
      )

      foreach ($root in $roots) {
        if (-not (Test-Path $root)) {
          continue
        }
        $match = Get-ChildItem -Path $root -Filter ffmpeg.exe -File -Recurse -ErrorAction SilentlyContinue |
          Select-Object -First 1
        if ($match) {
          Split-Path -Parent $match.FullName
          exit 0
        }
      }

      exit 1
    ' | tr -d '\r'
  }

  add_windows_path() {
    local ffmpeg_bin="$1"
    export PATH="$ffmpeg_bin:$PATH"
    if [[ -n "${GITHUB_PATH:-}" ]]; then
      printf '%s\n' "$ffmpeg_bin" >> "$GITHUB_PATH"
    fi
  }

  if command -v ffmpeg >/dev/null 2>&1; then
    return 0
  fi

  if retry 3 15 choco install ffmpeg --yes --no-progress; then
    local choco_bin=""
    choco_bin="$(resolve_ffmpeg_bin_windows || true)"
    if [[ -n "$choco_bin" ]]; then
      add_windows_path "$choco_bin"
      return 0
    fi
  fi

  echo "Chocolatey install failed; falling back to winget." >&2

  powershell.exe -NoLogo -NoProfile -NonInteractive -Command '
    $ErrorActionPreference = "Stop"
    winget install --id Gyan.FFmpeg `
      --exact `
      --accept-package-agreements `
      --accept-source-agreements `
      --silent
  '

  local winget_bin=""
  winget_bin="$(resolve_ffmpeg_bin_windows || true)"

  if [[ -n "$winget_bin" ]]; then
    add_windows_path "$winget_bin"
    return 0
  fi

  echo "ffmpeg was installed but could not be located on PATH." >&2
  return 1
}

case "${RUNNER_OS:-$(uname -s)}" in
  Linux)
    sudo apt-get update
    sudo apt-get install -y ffmpeg
    ;;
  macOS)
    brew install ffmpeg
    ;;
  Windows)
    install_ffmpeg_windows
    ;;
  *)
    echo "unsupported runner OS: ${RUNNER_OS:-unknown}" >&2
    exit 1
    ;;
esac

ffmpeg -version
