# petiglyph v0.1.3

## Summary

- Version: `0.1.3`
- Date: `2026-06-09`
- Release type: patch

This release is primarily a documentation and wording update on top of
`v0.1.2`. It adds a detailed tuning reference to the README, syncs agent-facing
repo instructions with the current CLI/TUI contract, and replaces the `Th`
abbreviation with `Threshold` in the Home creation workflow popup. There are no
behavioral CLI, JSON API, build-output, install, or font-runtime changes.

## Changes

- Added a new README tuning reference that explains threshold, invert,
  grayscale preprocessing, rows, columns, bleed, FPS, and project-level
  rendering knobs such as `glyph_size`, `codepoint_start`, `input_dir`, and
  `out_dir`.
- Documented the actual defaults, accepted ranges, and practical visual effect
  of each image-processing control so users can predict how a change will alter
  imported imagery before rebuilding.
- Updated agent-facing repo guidance to match the current project terminology,
  CLI/TUI contract, code layout, generated paths, TUI guardrails, and `hty`
  E2E workflow expectations.
- Consolidated dependency supply-chain guidance into the current docs layout.
- Replaced the abbreviated `Th` label with `Threshold` in the Home creation
  workflow popup for wording consistency.

## Compatibility

- CLI/API breaking changes: none
- Migration required: none

## Distribution

Prebuilt GitHub and npm binaries are provided for:

- Linux GNU on x86-64 and ARM64
- Linux musl on x86-64 and ARM64
- macOS on Intel and Apple Silicon
- Windows on x86-64 and ARM64

PyPI provides Linux GNU manylinux, macOS, and Windows wheels plus a source
distribution. The AUR package requires `ffmpeg` and `fontconfig`.

macOS and Windows artifacts are unsigned.
