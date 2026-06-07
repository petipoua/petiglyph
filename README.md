# petiglyph

`petiglyph` is a TUI and CLI tool for turning images and videos into custom font glyphs.

Each project is self-contained:

```text
my-font/
  petiglyph.toml
  images/
  build/
```

`images/` holds source images and imported animation frames, `build/` holds generated artifacts, and `petiglyph.toml` holds project config.

## Quickstart

### 1. Install `petiglyph`

Choose one:

```bash
npm install -g petiglyph
```
``` bash
pip install petiglyph
```
```bash
yay -S petiglyph
```

You can also download a prebuilt binary from GitHub Releases.

Make sure `ffmpeg` is available on `PATH`.

### 2. Open the TUI in the folder where you want the project

```bash
petiglyph
```

This opens the TUI Home panel, where you can create a project and start importing glyphs, grids, and animations.

## Reference

### Installation and runtime prerequisites

Distribution surfaces in this repo:

- GitHub Releases: prebuilt archives
- AUR: `petiglyph`
- npm: `petiglyph`
- PyPI: `petiglyph`

Install examples:

```bash
npm install -g petiglyph
pip install petiglyph
yay -S petiglyph
```

Runtime requirement:

- `ffmpeg` must be available on `PATH` before `petiglyph` starts.
- Interactive runs show an OS-aware install hint when `ffmpeg` is missing.
- To let petiglyph run the suggested install command for that run, pass `--ffmpeg-auto-install`.
- To suppress the one-time hint globally, set `PETIGLYPH_NO_FFMPEG_PROMPT=1`.

### Create a project and open the TUI

```bash
petiglyph create my-font
cd my-font
petiglyph
```

`petiglyph` with no subcommand launches the TUI. `petiglyph tui` does the same explicitly.

### Fast path from images to a font

Import a few images as glyphs:

```bash
petiglyph glyph create \
  --input ../assets/logo.png \
  --input ../assets/mark.svg
```

Build the font artifacts:

```bash
petiglyph build
```

Install the built font into the current user font directory:

```bash
petiglyph install-font
```

Build, install, refresh font state, and print the sample text you can paste into a terminal:

```bash
petiglyph sample
```

### First animation flow

Create an animated glyph from a GIF or video:

```bash
petiglyph animation create-standard \
  --input ../assets/run.mp4 \
  --fps 8 \
  --name run
```

### What the TUI is for

The TUI is the primary interface for:

- importing glyphs, grids, and animations through the Home panel workflows
- previewing glyphs without aspect-ratio distortion
- tuning per-glyph threshold and invert overrides
- building, installing, uninstalling, and sampling fonts
- selecting projects when no single manifest is auto-detected

On Windows, the Home creation workflows use a native file picker.

### Command reference

Core commands:

```bash
petiglyph create <name>
petiglyph create <name> --no-launch
petiglyph
petiglyph tui
petiglyph list
petiglyph delete
petiglyph build
petiglyph sample
petiglyph install-font
petiglyph uninstall-font
petiglyph uninstall-all-fonts
petiglyph doctor
```

Glyph commands:

```bash
petiglyph set-threshold <image> <threshold>
petiglyph clear-threshold <image>

petiglyph glyph create --input <path> [--input <path>...]
petiglyph glyph set-threshold <image> <threshold>
petiglyph glyph clear-threshold <image>
petiglyph glyph set-invert <image> --invert on|off
```

Composition and animation commands:

```bash
petiglyph grid create --input <path> --rows <n> --cols <n>
petiglyph composition set <image> --rows <n> --cols <n>
petiglyph composition clear <image>

petiglyph animation create-standard --input <path> --fps <n>
petiglyph animation create-grid --input <path> --rows <n> --cols <n> --fps <n>
petiglyph animation set-fps <name> --fps <n>
petiglyph animation delete <name>
```

Useful build-time options:

```bash
petiglyph build --force-remap
petiglyph sample --force-remap
petiglyph install-font --force-remap
```

`petiglyph uninstall` is intentionally a hidden stub that exits with guidance to use `uninstall-font` or `uninstall-all-fonts`.

### Project resolution and TUI launch behavior

Project-scoped commands accept `--manifest` to target a specific project.

When `--manifest` is omitted, petiglyph checks:

1. `./petiglyph.toml`
2. one directory level below the current directory

Behavior after discovery:

- exactly one project: that project is used
- zero projects: CLI commands fail with guidance; TUI opens on the Home panel
- multiple projects: CLI commands fail with guidance; TUI opens on the Home panel

`list` and `uninstall-all-fonts` are not manifest-scoped.

`petiglyph` and `petiglyph tui` require an interactive terminal. In non-TTY contexts they fail with an explicit terminal-required error.

### JSON automation contract

`--json` is supported on:

- `list`
- `delete`
- `set-threshold`
- `clear-threshold`
- `glyph create`
- `glyph set-threshold`
- `glyph clear-threshold`
- `glyph set-invert`
- `grid create`
- `composition set`
- `composition clear`
- `animation create-standard`
- `animation create-grid`
- `animation set-fps`
- `animation delete`
- `build`
- `sample`
- `install-font`
- `uninstall-font`
- `uninstall-all-fonts`
- `doctor`

Envelope shape:

```json
{
  "ok": true,
  "command": "build",
  "version": "0.1.0",
  "data": {}
}
```

Failure rules:

- non-zero exit code
- `ok: false`
- stable top-level fields: `ok`, `command`, `version`, `data`
- `error.message` always present on failures
- optional `error.causes[]` for nested context
- no extra human-oriented logs on stdout in JSON mode

### Manifest format

Default manifest values:

```toml
input_dir = "images"
out_dir = "build"
font_name = "Petiglyph"
glyph_size = 64
threshold = 64
codepoint_start = "U+100000"
```

Common managed sections:

```toml
[threshold_overrides]
"logo.png" = 72

[invert_overrides]
"logo.png" = true

[compositions."sheet.png"]
rows = 4
cols = 4
horizontal_bleed = "weak"
vertical_bleed = "off"
```

Notes:

- `project_id` is managed automatically and may be written back to the manifest during normal reads/builds.
- animation entries may also be normalized and written back during normal reads/builds.
- `threshold_overrides` and `invert_overrides` are keyed relative to `input_dir`.
- composition bleed defaults are `horizontal_bleed = "weak"` and `vertical_bleed = "off"`.

### Supported inputs

Direct build inputs in `images/`:

- `png`
- `jpg`
- `jpeg`
- `webp`
- `bmp`
- `gif`
- `svg`

Import-only conversion path:

- `avif` can be imported through CLI/TUI workflows, but it is converted to project-local `.png` files before build input scanning.

Animated creation workflows also accept:

- `gif`
- `mp4`
- `mov`
- `mkv`
- `webm`
- `avi`
- `m4v`

Media-processing limits:

- maximum `1200` extracted frames per media input
- maximum `3000` extracted frames per import operation

### Build outputs

`petiglyph build` recreates `out_dir` and writes:

- `<font-slug>.ttf`
- `<font-slug>.bdf`
- `glyph-map.json`
- `glyph-sample.txt`
- `previews/*.png`

It also maintains project-local lock/mapping state such as:

- `petiglyph.lock`
- `.petiglyph-build.lock`

`glyph-map.json` maps source files to assigned codepoints. `glyph-sample.txt` contains the generated sample string.

### Install, sample, and doctor

`install-font`:

- builds the project
- installs the `.ttf` into the current user font location
- uses a project-prefixed installed family name by default
- preserves project ownership through `project_id` and the Unicode registry

`sample`:

- builds the project
- performs a managed install
- refreshes platform font state
- prints the private-use sample string

`doctor`:

- runs global health checks without a manifest
- adds project-specific checks when a project is resolvable
- can repair stale locks, orphan metadata, and missing registry assignment with `--repair`

`uninstall-all-fonts` removes managed petiglyph fonts and managed metadata for the current user.

### Platform notes

Install roots:

- Linux: `~/.local/share/fonts/petiglyph/`
- macOS: `~/Library/Fonts/` for installed TTFs, with managed metadata under `~/Library/Fonts/petiglyph/`
- Windows: `%LOCALAPPDATA%/Microsoft/Windows/Fonts/petiglyph/`

Clipboard backends used by the TUI:

- Linux: `wl-copy` or `xclip`
- macOS: `pbcopy`
- Windows: PowerShell `Set-Clipboard` or `clip.exe`

After installing a font, fully quit and reopen terminal applications so they reload font state.

### Debugging and tests

Useful docs:

- [TESTS.md](TESTS.md)
- [CI.md](CI.md)
- [CROSS-COMPATIBILITY-GUIDE.md](CROSS-COMPATIBILITY-GUIDE.md)
- [RELEASE-GUIDE.md](RELEASE-GUIDE.md)
- [RELEASE-CHECKLIST.md](RELEASE-CHECKLIST.md)
- [CONTRIBUTING.md](CONTRIBUTING.md)

Useful env vars:

- `PETIGLYPH_NO_FFMPEG_PROMPT=1`
- `PETIGLYPH_TUI_DEBUG=1`
- `PETIGLYPH_TUI_DEBUG_LOG=/path/to/log`
- `PETIGLYPH_TUI_HTY_FULL_REPAINT=1`
