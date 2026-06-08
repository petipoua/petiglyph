# petiglyph v0.1.1

## Summary

- Version: `0.1.1`
- Date: `2026-06-08`
- Release type: patch

This release reorganizes the automation CLI around explicit project selection,
adds safer batch project and font management, expands project discovery, and
hardens the GitHub, npm, PyPI, and AUR release pipelines.

## Highlights

- Added `new-project` and `use-project` as the primary automation workflow.
- Consolidated glyph, grid, and animation imports under four creation types
  matching the TUI workflows.
- Added source configuration commands for thresholds, inversion, grids,
  animation timing, and bleed.
- Made installation explicit: creation imports content by default, `--build`
  generates artifacts, and `--install` additionally installs the font.
- Added structured project and installed-font listings with warnings for stale
  or malformed state.
- Added validated batch project deletion and exact-family font uninstall.
- Expanded project discovery through two directory levels without following
  directory symlinks.
- Normalized project paths across operating systems.

## CLI Contract

The automation CLI changed substantially from v0.1.0.

Added top-level commands:

- `petiglyph new-project NAME`
- `petiglyph use-project PROJECT ...`
- `petiglyph list projects`
- `petiglyph list installed-fonts`
- `petiglyph delete-project PROJECT...`

Project-scoped operations now include:

- `create glyph`
- `create grid-glyph`
- `create animated-glyph`
- `create animated-grid-glyph`
- `configure glyph`
- `configure grid-glyph`
- `configure animation`
- `delete animation`
- `build`
- `install-font`
- `show-sample`
- `doctor`
- `tui`

Behavior changes:

- Project selection uses an exact project directory basename.
- Duplicate project basenames are rejected with candidate paths.
- `--input` accepts multiple paths for creation commands.
- Creation no longer installs automatically.
- `--install` implies `--build`.
- `show-sample` only reads the existing `build/glyph-sample.txt`; it does not
  build or install.
- Batch deletion and uninstall validate every target before modifying files.
- `list installed-fonts` reports the exact family identifier accepted by
  `uninstall-font`.

## JSON API Schema Changes

The top-level JSON envelope remains:

- `ok`
- `command`
- `version`
- `data`
- `error` on failure

Command identifiers now follow the project-oriented hierarchy, including:

- `list.projects`
- `list.installed-fonts`
- `delete-project`
- `use-project.create.glyph`
- `use-project.create.grid-glyph`
- `use-project.create.animated-glyph`
- `use-project.create.animated-grid-glyph`
- `use-project.configure.glyph`
- `use-project.configure.grid-glyph`
- `use-project.configure.animation`
- `use-project.show-sample`

Project listings now include directory name, relative path, font name, manifest
path, project ID, and malformed-manifest warnings. Installed-font listings now
include family, ownership, TTF path, manifest path, and stale metadata or
artifact warnings.

Failures continue to return a non-zero exit code and include an error message
with nested causes.

## Font Lifecycle

- Installation is always explicit.
- Exact managed font families can be uninstalled directly.
- Multiple uninstall targets are validated before any font files or metadata
  are removed.
- Managed font cache and alias state are refreshed after uninstall.

## Integrator Impact

This release contains breaking automation CLI changes.

Scripts written for v0.1.0 should migrate from manifest-scoped command families
to project-scoped commands. Typical migrations include:

```text
petiglyph create my-font
petiglyph new-project my-font

petiglyph build --manifest my-font/petiglyph.toml
petiglyph use-project my-font build

petiglyph sample --manifest my-font/petiglyph.toml
petiglyph use-project my-font show-sample

petiglyph glyph create --input logo.png --manifest my-font/petiglyph.toml
petiglyph use-project my-font create glyph --input logo.png
```

Consumers parsing JSON should update expected `command` identifiers and data
objects to the new project-oriented contract.

## Distribution And Release Reliability

- Repaired GitHub release archive creation and smoke checks.
- Preserved the expected directory structure in Windows release archives.
- Installed FFmpeg for release smoke tests with hardened Windows fallback
  handling.
- Improved npm and PyPI bootstrap publication using scoped tokens or trusted
  publishing.
- Added npm trusted-publishing setup support.
- Made TestPyPI publication tolerate files already uploaded during retries.
- Added AUR publication helpers and packaging-only `pkgrel` update support.
- Aligned AUR package metadata and dependencies.

## Supported Media

- Static imports: PNG, JPEG, WebP, BMP, GIF, SVG, and AVIF.
- Animation imports: GIF, MP4, MOV, MKV, WebM, AVI, and M4V.
- AVIF imports are converted to project-local PNG files.

## Binaries

Prebuilt GitHub and npm binaries are provided for:

- Linux GNU on x86-64 and ARM64
- Linux musl on x86-64 and ARM64
- macOS on Intel and Apple Silicon
- Windows on x86-64 and ARM64

PyPI provides Linux GNU manylinux, macOS, and Windows wheels plus a source
distribution. Musllinux wheels are not currently published.

The AUR package requires `ffmpeg` and `fontconfig`.

macOS and Windows artifacts are unsigned.
