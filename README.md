# petiglyph

Rust CLI tool to convert image folders into monochrome font glyphs.

## Current status

Initial Rust CLI scaffolding is in place. Core conversion and font generation logic is not implemented yet.

## Quick start

```bash
cargo run -- init
cargo run -- build-font
```

## Interface strategy

- `clap` remains the primary interface for command parsing and automation-friendly subcommands.
- An optional interactive mode (TUI) will be added for visual editing and preview workflows.
- Planned TUI stack: `ratatui` (UI rendering), while command-line entrypoints and flags stay in `clap`.

## Intended input/output

- Input: one folder containing source images, one file per glyph.
- Accepted formats: common raster and vector formats (for example PNG, JPG/JPEG, WEBP, BMP, and SVG).
- Output: a generated font where each source file maps to one glyph, plus optional mapping metadata.

## Intended monochrome behavior

- All sources are converted to a single-color (monochrome) glyph mask.
- If an input image has alpha transparency, alpha is preserved and used to shape the glyph.
- If an input does not include transparency (for example plain JPEG), white is treated as background.
- Source filename and/or manifest rules determine glyph naming and codepoint assignment.

## Planned pipeline

1. Load and validate all supported image files from an input folder.
2. Normalize image bounds, sizing, and baseline alignment.
3. Convert each source image to monochrome glyph data.
4. Apply transparency rules (alpha-aware; white fallback when no alpha exists).
5. Map files to Unicode codepoints (typically PUA).
6. Generate font files (TTF/OTF/WOFF as configured).
7. Emit mapping metadata (JSON/TOML) for downstream usage.

## Planned interactive mode (TUI)

- Browse input images and generated monochrome previews.
- Assign/reassign glyph codepoints interactively.
- Tune conversion parameters (threshold/invert/scale/offset) with immediate preview.
- Save resulting manifest/settings and run the same build pipeline as non-interactive CLI mode.
