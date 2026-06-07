# petiglyph Release Notes Template

## Summary

- Version: `<version>`
- Date: `<YYYY-MM-DD>`
- Release type: `<patch|minor|major>`

## Highlights

- `<high-level item 1>`
- `<high-level item 2>`

## CLI Contract

- Added commands: `<none|list>`
- Removed commands: `<none|list>`
- Changed command behavior: `<none|list>`

## JSON API Schema Changes

Canonical command coverage for `--json` (including nested command families like `glyph`, `grid`, `composition`, and `animation`) is defined in:

- `README.md` -> `Automation API Contract`

Release notes should not duplicate that command list. Instead, document only what changed in this release relative to the canonical contract.

Top-level envelope fields (must remain):

- `ok`
- `command`
- `version`
- `data`
- `error` (failure only)

Schema changes this release:

- Added fields in `data`: `<none|list>`
- Removed fields in `data`: `<none|list>`
- Semantic changes in existing fields: `<none|list>`

## Font Lifecycle

- Install behavior changes: `<none|details>`
- Uninstall behavior changes: `<none|details>`
- Platform-specific behavior changes: `<none|details>`

## Integrator Impact

- Breaking changes: `<none|list>`
- Migration steps required: `<none|steps>`

## Binaries

Attach prebuilt binaries for:

- Linux GNU (`x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`)
- Linux musl (`x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`)
- macOS (`x86_64-apple-darwin`, `aarch64-apple-darwin`)
- Windows (`x86_64-pc-windows-msvc`, `aarch64-pc-windows-msvc`)

Distribution callouts:

- AUR package requires `ffmpeg` and `fontconfig`.
- npm publishes `petiglyph` plus all `petiglyph-*` platform packages.
- PyPI publishes Linux GNU manylinux, macOS, and Windows wheels plus sdist.
- macOS and Windows artifacts are unsigned unless this release explicitly states otherwise.

## Verification Checklist

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo check`
- [ ] `cargo clippy --all-targets --all-features -- -D warnings`
- [ ] `cargo test`
- [ ] README command section matches implementation
- [ ] JSON samples still parse and match envelope contract
