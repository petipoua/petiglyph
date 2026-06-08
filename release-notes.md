# petiglyph v0.1.2

## Summary

- Version: `0.1.2`
- Date: `2026-06-09`
- Release type: patch

This release improves multi-registry release reliability and fixes README
rendering on PyPI and npm. There are no CLI or TUI behavior changes.

## Package Descriptions

- Fixed the demo image and video links in the PyPI description by using
  absolute GitHub asset URLs.
- Expanded the npm meta-package description to include the full repository
  README after its npm-specific installation and platform-package guidance.
- Added deterministic npm README generation from the canonical repository
  README.
- Added release hygiene checks that reject a stale generated npm README.

## Release Reliability

- Made same-version release retries resume from the immutable tagged commit
  when `main` has advanced.
- Corrected GitHub artifact verification to validate each asset's provenance
  attestation and tagged source digest.
- Added visible per-package progress while waiting for npm, PyPI, and AUR
  registry propagation.
- Made the public AUR cgit metadata authoritative during verification while
  treating delayed AUR RPC indexing as informational.
- Prevented long-running releases from reading a script modified during
  execution by running from an immutable temporary snapshot.

## Distribution

Prebuilt GitHub and npm binaries are provided for:

- Linux GNU on x86-64 and ARM64
- Linux musl on x86-64 and ARM64
- macOS on Intel and Apple Silicon
- Windows on x86-64 and ARM64

PyPI provides Linux GNU manylinux, macOS, and Windows wheels plus a source
distribution. The AUR package requires `ffmpeg` and `fontconfig`.

macOS and Windows artifacts are unsigned.
