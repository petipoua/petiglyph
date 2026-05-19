#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

dist_dir="${1:-dist-release}"

if [[ ! -d "$dist_dir" ]]; then
  echo "Dist directory not found: $dist_dir" >&2
  exit 1
fi

version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n1)"
if [[ -z "$version" ]]; then
  echo "Could not read version from Cargo.toml" >&2
  exit 1
fi

tmpdir="$(mktemp -d)"
trap 'rm -rf "$tmpdir"' EXIT

extract_archive() {
  local archive="$1"
  local out_dir="$2"
  mkdir -p "$out_dir"
  case "$archive" in
    *.tar.gz) tar -xzf "$archive" -C "$out_dir" ;;
    *.zip) unzip -q "$archive" -d "$out_dir" ;;
    *)
      echo "Unsupported archive format: $archive" >&2
      return 1
      ;;
  esac
}

stage_target() {
  local target="$1"
  local package_dir="$2"
  local bin_name="$3"

  local base="petiglyph-v${version}-${target}"
  local archive=""

  if [[ -f "$dist_dir/$base.tar.gz" ]]; then
    archive="$dist_dir/$base.tar.gz"
  elif [[ -f "$dist_dir/$base.zip" ]]; then
    archive="$dist_dir/$base.zip"
  else
    archive="$(find "$dist_dir" -type f \( -name "$base.tar.gz" -o -name "$base.zip" \) | head -n1 || true)"
  fi

  if [[ -z "$archive" ]]; then
    echo "Missing release archive for $target in $dist_dir" >&2
    return 1
  fi

  local unpack_dir="$tmpdir/$target"
  extract_archive "$archive" "$unpack_dir"

  local bin_path="$(find "$unpack_dir" -type f -name "$bin_name" | head -n1 || true)"
  if [[ -z "$bin_path" ]]; then
    echo "Could not locate $bin_name in $archive" >&2
    return 1
  fi

  install -Dm755 "$bin_path" "$repo_root/$package_dir/bin/$bin_name"
  echo "Staged $target -> $package_dir/bin/$bin_name"
}

stage_target "x86_64-unknown-linux-gnu" "npm/petiglyph-linux-x64-gnu" "petiglyph"
stage_target "aarch64-unknown-linux-gnu" "npm/petiglyph-linux-arm64-gnu" "petiglyph"
stage_target "x86_64-unknown-linux-musl" "npm/petiglyph-linux-x64-musl" "petiglyph"
stage_target "aarch64-unknown-linux-musl" "npm/petiglyph-linux-arm64-musl" "petiglyph"
stage_target "x86_64-apple-darwin" "npm/petiglyph-darwin-x64" "petiglyph"
stage_target "aarch64-apple-darwin" "npm/petiglyph-darwin-arm64" "petiglyph"
stage_target "x86_64-pc-windows-msvc" "npm/petiglyph-win32-x64-msvc" "petiglyph.exe"
stage_target "aarch64-pc-windows-msvc" "npm/petiglyph-win32-arm64-msvc" "petiglyph.exe"

echo "npm platform binaries staged from $dist_dir"
