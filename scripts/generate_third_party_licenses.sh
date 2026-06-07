#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
out_file="$repo_root/THIRD_PARTY_LICENSES.md"

if ! command -v cargo >/dev/null 2>&1; then
  echo "error: cargo is required" >&2
  exit 1
fi

if ! command -v jq >/dev/null 2>&1; then
  echo "error: jq is required" >&2
  exit 1
fi

tmp_md="$(mktemp)"
tmp_rows="$(mktemp)"
trap 'rm -f "$tmp_md" "$tmp_rows"' EXIT

generated_at="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"
rustc_version="$(rustc --version 2>/dev/null || echo "unavailable")"
cargo_version="$(cargo --version 2>/dev/null || echo "unavailable")"

cargo metadata --locked --format-version 1 \
  | jq -r '
      .packages
      | map({
          name,
          version,
          license: (.license // "UNKNOWN")
        })
      | unique_by(.name, .version, .license)
      | sort_by(.name, .version)
      | .[]
      | "| `" + .name + "` | `" + .version + "` | `" + .license + "` |"
    ' > "$tmp_rows"

{
  echo "# Third-Party License Inventory"
  echo
  echo "Generated: $generated_at (UTC)"
  echo
  echo "Tooling:"
  echo "- \`$rustc_version\`"
  echo "- \`$cargo_version\`"
  echo
  echo "This inventory is generated from \`cargo metadata --locked\` and captures the transitive Rust crate dependency graph license declarations."
  echo
  echo "| Crate | Version | License (SPDX expression or crate declaration) |"
  echo "|---|---|---|"
  cat "$tmp_rows"
  echo
  echo "## Policy Gate"
  echo
  echo "Run the repository policy check:"
  echo
  echo "\`\`\`bash"
  echo "cargo deny check licenses"
  echo "\`\`\`"
} > "$tmp_md"

mv "$tmp_md" "$out_file"
echo "wrote $out_file"
