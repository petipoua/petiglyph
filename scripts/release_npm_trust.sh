#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

usage() {
  cat <<'USAGE'
Usage:
  ./scripts/release_npm_trust.sh

Configures npm trusted publishing for the petiglyph npm packages using the
current npm login.

Prerequisites:
  - npm 11.10.0 or newer
  - package write access on npm
  - npm account 2FA enabled
  - all petiglyph npm packages already published

This sets GitHub Actions trusted publishing for:
  - repo: petipoua/petiglyph
  - workflow: npm-publish.yml
  - environment: npm
USAGE
}

if [[ "${1:-}" =~ ^(-h|--help|help)$ ]]; then
  usage
  exit 0
fi

packages=(
  petiglyph-linux-x64-gnu
  petiglyph-linux-arm64-gnu
  petiglyph-linux-x64-musl
  petiglyph-linux-arm64-musl
  petiglyph-darwin-x64
  petiglyph-darwin-arm64
  petiglyph-win32-x64-msvc
  petiglyph-win32-arm64-msvc
  petiglyph
)

for pkg in "${packages[@]}"; do
  npm trust github "$pkg" \
    --repo petipoua/petiglyph \
    --file npm-publish.yml \
    --env npm \
    --allow-publish \
    --yes
  sleep 2
done
