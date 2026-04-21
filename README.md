# petiglyph

`petiglyph` turns a folder of images into a monochrome glyph font project, with a TUI as the primary interface.

## Current Model

- `petiglyph` itself is the installed tool.
- Each font project is self-contained in its own directory.
- Source images live in `icons/`.
- Generated assets live in `build/`.
- Project config lives in `petiglyph.toml`.
- The TUI is the main entrypoint for tuning, building, previewing, and installing fonts.

## Project Layout

```text
my-font/
  petiglyph.toml
  icons/
  build/
```

Deleting the project directory removes the project and all of its generated assets.

## Quick Start

```bash
petiglyph create my-font
cd my-font
petiglyph
```

After `create`, place your images in `icons/`. The TUI can then:

- rescan icons
- tune per-glyph thresholds
- show ASCII previews
- build the `.ttf`, `.bdf`, previews, glyph map, and sample text
- install the built `.ttf` into `~/.local/share/fonts/petiglyph/<project>/`

## Commands

```bash
# create a new project in the current directory
petiglyph create my-font

# launch the TUI for the current project
petiglyph
petiglyph tui

# build from the current project manifest
petiglyph build

# build and print the private-use sample string
petiglyph sample

# build and install the font into the user font directory
petiglyph install-font
```

All non-`create` commands also support `--manifest` to target another project.

## Manifest

```toml
input_dir = "icons"
out_dir = "build"
font_name = "Petiglyph"
glyph_size = 64
threshold = 64
codepoint_start = "U+100000"

[threshold_overrides]
"codex.png" = 72
"factory.svg" = 51
```

`threshold_overrides` stores per-file threshold tuning relative to `input_dir`.

## TUI Keys

- `1` / `2` / `3`: switch between Home, Glyphs, and Font views
- `R`: rescan `icons/`
- `j` / `k` or arrow keys: select glyph
- `+` / `-`: adjust threshold by 1 for the selected glyph
- `PgUp` / `PgDn`: adjust threshold by 10 for the selected glyph
- `r`: clear the selected glyph override
- `b`: build the project
- `i`: build and install the font
- `q` / `Esc`: quit

## Outputs

The build step writes these files into `build/`:

- `*.ttf`
- `*.bdf`
- `glyph-map.json`
- `glyph-sample.txt`
- `previews/*.png`

## Notes

- Supported inputs: `png`, `jpg`, `jpeg`, `webp`, `bmp`, `gif`, `svg`
- If source alpha exists, alpha drives glyph coverage.
- Otherwise, border color is treated as the background and contrast becomes coverage.
- The default codepoint range starts at `U+100000` to avoid common BMP private-use collisions.
