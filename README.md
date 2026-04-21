# petiglyph

Rust CLI tool to convert a folder of images into monochrome font glyph assets.

## v0.0.1 status

Implemented:

- Folder input (`one image file = one glyph`).
- Supported inputs: `png`, `jpg/jpeg`, `webp`, `bmp`, `gif`, `svg`.
- Monochrome conversion with transparency-aware behavior.
- Output font artifacts as bitmap `BDF` and outline `TTF`.
- Per-glyph preview PNG generation.
- Glyph mapping export as JSON.
- Glyph sample text export for terminal/editor testing.
- Interactive terminal preview mode (TUI) using `ratatui`.

Not in v0.0.1 yet:

- OTF/WOFF output.
- Interactive codepoint editing persistence.

## Quick start

```bash
cargo run -- init
# edit petiglyph.toml
cargo run -- build-font
```

## Interface strategy (`clap` + `ratatui`)

- `clap` is the primary command parser and automation interface.
- `ratatui` powers an optional interactive mode: `cargo run -- interactive`.

## Commands

```bash
# create starter config
cargo run -- init --output petiglyph.toml

# build from manifest
cargo run -- build-font --manifest petiglyph.toml

# build with overrides
cargo run -- build-font --manifest petiglyph.toml --input-dir ./icons --out-dir ./dist --threshold 72 --glyph-size 64 --codepoint-start U+100000

# interactive preview mode
cargo run -- interactive --manifest petiglyph.toml

# glyph demo using the generated TTF/sample
cargo run -- demo --manifest petiglyph.toml --input-dir ./icons --out-dir ./dist
```

## Manifest (`petiglyph.toml`)

```toml
input_dir = "icons"
out_dir = "dist"
font_name = "Petiglyph"
glyph_size = 64
threshold = 64
codepoint_start = "U+100000"

[threshold_overrides]
"codex.png" = 72
"factory.svg" = 51
```

`threshold` is the base value for every glyph. `threshold_overrides` is optional and lets you override specific icons by their path relative to `input_dir`. These per-icon overrides are used by `build-font`, `demo`, and `interactive`.

## Output files

By default in `dist/`:

- `*.bdf`: bitmap font file (monochrome glyphs).
- `*.ttf`: TrueType font file generated from the thresholded glyphs.
- `glyph-map.json`: glyph-to-file and codepoint mapping.
- `glyph-sample.txt`: copyable private-use sample string for testing the font.
- `previews/*.png`: generated monochrome previews, one file per glyph.

## Monochrome and transparency behavior

- If source alpha exists, alpha defines glyph coverage. This preserves white-on-transparent logos and anti-aliased edges.
- If source has no alpha, border color is treated as the background and pixel contrast against that background becomes glyph coverage.
- Monochrome threshold then turns coverage into on/off glyph pixels.

## Interactive mode keys

- `q` / `Esc`: quit.
- `j` / `k` or arrow keys: select glyph.
- `+` / `-`: adjust the selected glyph threshold by 1 and save it immediately to `petiglyph.toml`.
- `PgUp` / `PgDn`: adjust the selected glyph threshold by 10 and save it immediately to `petiglyph.toml`.
- `r`: remove the selected glyph override and fall back to the base `threshold`.

## Terminal demo

Use the demo when you want the real private-use glyph string for the generated font. The default starter manifest uses `U+100000` to avoid common Nerd Font collisions in the BMP private-use area:

```bash
cargo run -- demo --manifest ./petiglyph.toml --input-dir ./icons --out-dir ./dist
```

It prints:

- the generated `.ttf` path
- the generated `glyph-sample.txt` path
- the copyable Unicode private-use glyph string itself

To verify the TTF outside the terminal UI, build the font and point a renderer at it with a temporary fontconfig setup. For example:

```bash
cargo run -- build-font --manifest ./petiglyph.toml --input-dir ./icons --out-dir ./dist
tmpfc="$(mktemp -d)"
mkdir -p "$tmpfc/fonts" "$tmpfc/cache"
cp ./dist/petiglyph.ttf "$tmpfc/fonts/"
cat > "$tmpfc/fonts.conf" <<EOF
<?xml version="1.0"?>
<!DOCTYPE fontconfig SYSTEM "fonts.dtd">
<fontconfig>
  <dir>/usr/share/fonts</dir>
  <dir>$tmpfc/fonts</dir>
  <cachedir>$tmpfc/cache</cachedir>
</fontconfig>
EOF
FONTCONFIG_FILE="$tmpfc/fonts.conf" fc-cache -f "$tmpfc/fonts"
FONTCONFIG_FILE="$tmpfc/fonts.conf" pango-view --no-display --font="Petiglyph 48" --text="$(cat ./dist/glyph-sample.txt)" --output ./dist/specimen.png
```
