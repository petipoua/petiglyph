# Dependency And Supply-Chain Notes

Last updated: 2026-05-31

This file records the dependency checks required by TODO item 8.

## Tooling Gates

- `cargo deny check` uses [`deny.toml`](../deny.toml) for:
  - advisory scanning
  - license policy
  - duplicate-version policy (`warn`)
  - source registry restrictions
- `cargo audit` checks RustSec advisories.
- `cargo tree --locked -e normal` is kept as a release input for dependency review.
- Temporary exception: `RUSTSEC-2024-0436` (`paste`) is ignored in `deny.toml` because it is transitively pulled by AVIF encoding (`rav1e` via `ravif`) with no drop-in upgrade path from `image` yet.

## Sample Asset Redistribution

- `icons/` sample assets are included as repository fixtures for local demos/tests.
- Before public release, confirm each file in `icons/` has explicit redistribution permission or replace it with an internally created equivalent.
- Do not publish a release if any `icons/` source license/provenance is unknown.

## Native/Cross-Build-Sensitive Dependency Areas

Based on `cargo tree --locked -e normal` on 2026-05-31:

- `image` is compiled with `avif`, `bmp`, `gif`, `jpeg`, `png`, and `webp` features.
- AVIF-related chain includes `dav1d`, `dav1d-sys`, `ravif`, and `rav1e`.
- SVG/raster rendering stack includes `resvg`, `usvg`, and `tiny-skia`.

Risk notes:

- AVIF native codec dependencies can be more sensitive in cross builds than PNG/JPEG/WebP paths.
- If cross-build instability appears on release targets, make AVIF support optional behind a Cargo feature and keep non-AVIF formats in default builds.
- Keep CI runtime smoke checks active across Linux/macOS/Windows after dependency updates.
