# petiglyph

Rust CLI tool to transform SVG icons into terminal-usable font glyphs.

## Current status

Initial Rust CLI scaffolding is in place. Core conversion logic is not implemented yet.

## Quick start

```bash
cargo run -- init
cargo run -- build-font
```

## Planned pipeline

1. Load and validate SVG icons.
2. Normalize paths and viewport sizing.
3. Map icons to PUA codepoints.
4. Generate font files (TTF/OTF).
5. Emit mapping metadata (JSON/TOML).
