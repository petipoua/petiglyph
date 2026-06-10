# petiglyph

`petiglyph` is a TUI and CLI tool for turning images and videos into custom font glyphs.

[![Demo video](https://raw.githubusercontent.com/petipoua/petiglyph/main/readme-assets/petiglyph-demo.gif)](https://raw.githubusercontent.com/petipoua/petiglyph/main/readme-assets/petiglyph-demo.mp4)

## Quickstart

### 1. Installation

Choose one:

```bash
npm install -g petiglyph
```

```bash
pip install petiglyph
```

```bash
yay -S petiglyph
```

You can also download a prebuilt binary from GitHub Releases.

Make sure `ffmpeg` is available on `PATH`.

### 2. Usage

In a terminal, go in the folder where you want the project to be, and type:

```bash
petiglyph
```

This opens the TUI Home panel, where you can create a project and start importing media to create glyphs, grids, and animations.

Supported media:

- images: `png`, `jpg`, `jpeg`, `webp`, `bmp`, `gif`, `svg`, `avif`
- animation media: `gif`, `mp4`, `mov`, `mkv`, `webm`, `avi`, `m4v`

Generated glyphs can be of 4 types:

- standard monospace, static
- standard monospace, animated
- grid of glyphs, static
- grid of glyphs, animated

> [!WARNING]
> After installing petiglyph fonts, you need to reboot all instances of the terminal you're using, or, if it doesn't work, reboot your computer, to make sure the new fonts are correctly loaded by your apps or system.

For all 4 types, petiglyph will generate the static and animated glyphs in the `Installed petiglyph fonts` area of the Home panel after installing the fonts.

To copy those glyphs for reusing them elsewhere, you have to navigate with your arrows to the `Installed petiglyph fonts` area, select the glyphs that you want, press ENTER to copy them to your clipboard, and paste them in your own tools or apps (those apps need to also have the new fonts loaded, see warning above).

Pressing ENTER on the animated glyphs copies all frames inside the clipboard. To see them animated, you need to make your own script to render all the glyphs into an animation.
<br>

---

## Documentation

### Project Model And Discovery

Each project is self-contained:

```text
my-font/
  petiglyph.toml
  images/
  build/
  petiglyph.lock
```

`petiglyph.toml` stores configuration, `images/` stores project-local sources and
imported frames, `build/` stores generated artifacts, and `petiglyph.lock`
preserves glyph-to-codepoint assignments after the first build.

Create a project with:

```bash
petiglyph new-project my-font
```

Without a subcommand, `petiglyph` opens the workspace TUI. Project discovery
searches the current directory and descendants through depth 2, without
following directory symlinks. One discovered project is selected automatically.
Multiple projects open the workspace selector in an interactive terminal;
automation commands instead report the ambiguity. `use-project` selects by exact
project directory basename and rejects duplicate matches with candidate paths.

### CLI Reference

| Command | Purpose and important options |
| --- | --- |
| `petiglyph` or `petiglyph tui` | Open the workspace TUI. Requires an interactive terminal. |
| `petiglyph new-project <name>` | Create `<name>/petiglyph.toml`, `images/`, and `build/`. |
| `petiglyph use-project <project> tui` | Open one selected project in the TUI. |
| `petiglyph use-project <project> create glyph ...` | Import one or more static glyph sources. |
| `petiglyph use-project <project> create grid-glyph ...` | Import a source split into `--rows` by `--cols`. |
| `petiglyph use-project <project> create animated-glyph ...` | Import GIF/video frames as glyphs; requires `--fps`. |
| `petiglyph use-project <project> create animated-grid-glyph ...` | Import frames split into a grid; requires `--fps`, `--rows`, and `--cols`. |
| `petiglyph use-project <project> configure glyph <source> ...` | Set or clear a source threshold and set invert. |
| `petiglyph use-project <project> configure grid-glyph <source> ...` | Configure grid dimensions, bleed, threshold, and invert. |
| `petiglyph use-project <project> configure animation <name> ...` | Configure FPS, threshold, invert, and grid-only settings. |
| `petiglyph use-project <project> delete animation <name>` | Remove an animation definition. |
| `petiglyph use-project <project> build` | Generate project artifacts. `--force-remap` discards prior codepoint assignments. |
| `petiglyph use-project <project> install-font` | Build and install the selected project's managed font. |
| `petiglyph use-project <project> show-sample` | Print the existing `build/glyph-sample.txt`; it does not build. |
| `petiglyph use-project <project> doctor` | Run global and selected-project checks; `--repair` applies supported repairs. |
| `petiglyph list projects` | List discovered projects and malformed-manifest warnings. |
| `petiglyph list installed-fonts` | List managed installed families and ownership details. |
| `petiglyph delete-project <project>...` | Validate the full batch, then delete project directories. |
| `petiglyph uninstall-font <installed-family>...` | Uninstall exact managed family names from `list installed-fonts`. |
| `petiglyph uninstall-all-fonts` | Remove all managed Petiglyph fonts and metadata. |
| `petiglyph doctor` | Run global checks and any resolvable project checks. |

Global `--debug` enables project-local pipeline diagnostics.
`--ffmpeg-auto-install` allows the detected package-manager command to run when
FFmpeg is missing. Automation-capable commands accept `--json`; creation also
accepts `--build` and `--install`, where `--install` implies a build. Use
`petiglyph <command> --help` for exhaustive flag syntax and accepted values.

### Creating And Configuring Glyphs

The CLI exposes the same four creation types as the TUI:

```bash
# One image per glyph
petiglyph use-project my-font create glyph \
  --input logo.png mark.svg

# One sprite sheet split into glyph cells
petiglyph use-project my-font create grid-glyph \
  --input icons.png --rows 4 --cols 4

# GIF/video frames as glyphs
petiglyph use-project my-font create animated-glyph \
  --input spinner.gif --name spinner --fps 8

# Each frame split into grid cells
petiglyph use-project my-font create animated-grid-glyph \
  --input dashboard.mp4 --name dashboard --fps 10 \
  --rows 2 --cols 4
```

One `--input` accepts multiple paths. Creation imports and configures sources
without building or installing unless requested. Static build inputs are `png`,
`jpg`, `jpeg`, `webp`, `bmp`, `gif`, and `svg`. `avif` is import-only and is
converted to project-local PNG. Animation media supports `gif`, `mp4`, `mov`,
`mkv`, `webm`, `avi`, and `m4v`.

```bash
petiglyph use-project my-font configure glyph logo.png \
  --threshold 88 --invert on
petiglyph use-project my-font configure glyph logo.png \
  --clear-threshold
petiglyph use-project my-font configure grid-glyph icons.png \
  --rows 2 --cols 8 --horizontal-bleed strong --vertical-bleed off
petiglyph use-project my-font configure animation spinner \
  --fps 12 --threshold 72 --invert off
petiglyph use-project my-font delete animation spinner
```

Grid dimensions and bleed are valid only for grid sources and grid animations.

### Tuning Parameters

The image pipeline performs optional grayscale import processing, aspect-fit
scaling, thresholding, optional inversion, and then optional grid-edge bleed.
All previews preserve the source or glyph aspect ratio with one uniform scale;
empty vertical rows may be cropped before fitting, but previews are never
stretched to fill spare width or height.

| Setting | Range/default | Effect |
| --- | --- | --- |
| `threshold` | `0..=255`, default `64` | Higher values retain fewer pixels; lower values produce fuller shapes. The project default can be overridden per source. |
| `invert` | `on` or `off`, default `off` | Flips the thresholded result. |
| `grayscale_enabled` | Static default off; animated default on | Rewrites imported raster pixels through grayscale tone processing. SVG files are not rewritten. |
| `grayscale_brightness` | `-80..=80`, default `0` | Brightens or darkens grayscale values. |
| `grayscale_contrast` | `-80..=80`, default `0` | Increases or reduces tonal separation. |
| `grayscale_gamma_percent` | `50..=200`, default `100` | Adjusts midtones; processing order is gamma, contrast, brightness, then threshold. |
| `rows`, `cols` | Both greater than `0` | Fit the full source to a grid, then split it into glyph cells. |
| `horizontal_bleed` | `off`, `weak`, `strong`; default `weak` | Extends outlines slightly across internal left/right grid boundaries to reduce seams. |
| `vertical_bleed` | `off`, `weak`, `strong`; default `off` | Extends outlines across internal top/bottom boundaries; support varies by renderer. |
| `fps` | `1..=30` | Stores Petiglyph preview/playback timing metadata. TTF files contain static glyphs and do not animate themselves. |
| `glyph_size` | Default `64` | Sets the raster working size used before font outline generation. |

`off` keeps grid edges clipped, `weak` adds a small overlap, and `strong` adds a
larger overlap. Bleed changes final outline geometry, not thresholding.

### Manifest Reference

Normal reads and builds may rewrite `petiglyph.toml` to create or normalize
managed fields, so keep it writable.

| Field | Type and default | Meaning |
| --- | --- | --- |
| `input_dir` | String, `"images"` | Project-relative source directory. |
| `out_dir` | String, `"build"` | Project-relative build directory. |
| `font_name` | String, `"Petiglyph"` | Generated font family name. |
| `glyph_size` | Integer, `64` | Raster working size. |
| `threshold` | Integer, `64` | Project-wide threshold. |
| `codepoint_start` | Codepoint string, `"U+100000"` | Preferred start of the project's supplementary private-use range. |
| `project_id` | Managed string | Stable project identity. Missing or blank values are generated automatically. |
| `threshold_overrides` | Table, empty | Per-source thresholds keyed by paths relative to `input_dir`. |
| `invert_overrides` | Table, empty | Per-source booleans keyed by paths relative to `input_dir`. |
| `compositions` | Table, empty | Static grid definitions keyed by source path. |
| `animations` | Array of tables, empty | Standard or grid animation definitions. |

Grid compositions contain `rows`, `cols`, `horizontal_bleed`, and
`vertical_bleed`. Omitted bleed values normalize to `weak` horizontally and
`off` vertically.

```toml
[compositions."icons.png"]
rows = 4
cols = 4
horizontal_bleed = "weak"
vertical_bleed = "off"

[[animations]]
name = "spinner"
type = "standard"
fps = 8
frames = ["spinner-0001.png", "spinner-0002.png"]

[[animations]]
name = "dashboard"
type = "grid"
fps = 10
frames = ["dashboard-0001.png", "dashboard-0002.png"]
rows = 2
cols = 4
horizontal_bleed = "weak"
vertical_bleed = "off"
```

Animation frame paths are relative source keys. Standard animations must not
define grid fields. Grid animations require positive `rows` and `cols`.
Animation entries may also contain `grayscale_processing`, which records
`grayscale_enabled` plus a nested `grayscale` table with `gamma_percent`,
`contrast`, and `brightness`. Tone values are normalized to their supported
ranges on read.

### Building And Codepoint Stability

```bash
petiglyph use-project my-font build
petiglyph use-project my-font build --force-remap
```

| Artifact | Location and purpose |
| --- | --- |
| TTF | `build/<slugified-font-name>.ttf`: standard font for applications and managed installation. |
| BDF | `build/<slugified-font-name>.bdf`: bitmap font representation. |
| Mapping | `build/glyph-map.json`: source/glyph names and assigned Unicode codepoints. |
| Sample | `build/glyph-sample.txt`: copyable text using the assigned codepoints. |
| Previews | `build/previews/*.png`: thresholded preview image for each generated glyph. |
| Glyph lock | `petiglyph.lock`: project-root assignment history used by future builds. |

Existing source mappings remain stable through `petiglyph.lock`. Removed
mappings may remain inactive so their codepoints are not accidentally reused.
Petiglyph reserves non-overlapping supplementary private-use ranges for managed
projects. A normal build can relocate and remap a project automatically if its
owned range must grow and existing assignments cannot be preserved.

`--force-remap` intentionally discards the existing assignments and allocates a
fresh mapping. Text stored with old codepoints can then render the wrong glyphs
or no glyph at all.

The generated TTF can be embedded in another application like any standard
font. Embedding the Petiglyph executable itself also requires FFmpeg to be
available on each user's system.

### Installing And Uninstalling

```bash
petiglyph use-project my-font install-font
petiglyph list installed-fonts
petiglyph uninstall-font "my-font Petiglyph"
petiglyph uninstall-all-fonts
```

Installation is explicit, managed, and owned by project metadata. Reinstalling
a project replaces that project's previous managed artifact. Petiglyph refreshes
or broadcasts font-cache state where the platform supports it.

Managed locations:

| Platform | Location |
| --- | --- |
| Linux | `~/.local/share/fonts/petiglyph/` |
| macOS | TTFs in `~/Library/Fonts/`; metadata in `~/Library/Fonts/petiglyph/` |
| Windows | `%LOCALAPPDATA%/Microsoft/Windows/Fonts/petiglyph/` |

Use `list installed-fonts` for the exact family names accepted by
`uninstall-font`. Uninstall commands select managed artifacts through Petiglyph
metadata and do not remove unrelated fonts. Restart applications after changing
fonts so they reload font state; a full system reboot is only a fallback.

On Windows Terminal 1.21 or newer, add the installed family after the primary
terminal font as a fallback, for example
`"Cascadia Mono, my-font Petiglyph"`.

### Doctor And Recovery

`petiglyph doctor` always checks global managed-install and Unicode registry
state. It also runs project checks when one or more manifests can be resolved.
`petiglyph use-project my-font doctor` selects one project explicitly.

| Finding | Meaning |
| --- | --- |
| `info` | A successful check or informational state. |
| `warning` | A recoverable inconsistency or missing expected state. |
| `error` | Invalid state that can block correct operation. |
| `repaired` status | The reported issue was changed successfully during this run. |

Checks cover stale global and project locks, malformed install metadata,
orphaned metadata and managed TTFs, Unicode registry structure and range
conflicts, manifest validity, source visibility, and glyph-lock identity,
entries, codepoints, and source references.

```bash
petiglyph doctor --repair
petiglyph use-project my-font doctor --repair
```

`--repair` performs only supported repairs, such as removing stale locks and
orphans, removing invalid glyph-lock entries, or recreating a missing project
range assignment. It does not rebuild or reinstall a project.

### JSON Automation

Add `--json` to supported commands:

```bash
petiglyph list projects --json
petiglyph use-project my-font create glyph \
  --input logo.svg --build --json
petiglyph use-project my-font doctor --json
```

Successful and failed commands use one envelope:

```json
{
  "ok": false,
  "command": "use-project.create.glyph",
  "version": "0.1.5",
  "data": {
    "completed_stages": ["resolve", "import", "configure"]
  },
  "error": {
    "code": "creation_stage_failed",
    "stage": "build",
    "message": "build failed",
    "causes": [],
    "hints": [
      "Imported files and manifest changes were retained.",
      "Fix the reported cause, then run the project build or install command."
    ]
  }
}
```

| Field | Meaning |
| --- | --- |
| `ok` | `true` on success, otherwise `false`. |
| `command` | Stable command identifier. |
| `version` | Petiglyph version that produced the response. |
| `data` | Command-specific payload; failed creation includes completed stages. |
| `error` | `null` on success, otherwise the structured failure. |
| `error.code` | Machine-readable failure category. |
| `error.stage` | Failed creation stage when applicable. |
| `error.message` | Primary user-facing error. |
| `error.causes` | Nested error-chain messages. |
| `error.hints` | Recovery guidance. |
| `error.candidates` | Candidate matches when supplied; omitted when empty. |

Failures return a non-zero exit code. A multi-stage creation failure can retain
completed imports and manifest changes; inspect `data.completed_stages`, fix the
reported cause, then run build or install again.

### Runtime, Debugging, And Troubleshooting

FFmpeg is checked before command dispatch, including TUI launch. Install it on
`PATH`, pass `--ffmpeg-auto-install`, or set
`PETIGLYPH_NO_FFMPEG_PROMPT=1` to suppress the interactive install hint.

`--debug` or a truthy `PETIGLYPH_DEBUG` enables image-pipeline diagnostics in
the selected project's `debug/pipeline.log` and `debug/artifacts/`.
`PETIGLYPH_DEBUG_CELL=<width>x<height>` overrides terminal cell geometry used by
debug preview rendering.

Set `PETIGLYPH_TUI_DEBUG=1` for TUI event logging. Logs use the platform
temporary directory by default; `PETIGLYPH_TUI_DEBUG_LOG=/path/to/log` overrides
the path.

| Problem | Action |
| --- | --- |
| FFmpeg is missing | Install FFmpeg on `PATH` or use `--ffmpeg-auto-install`, then rerun Petiglyph. |
| No project is found | Run from a directory containing a project within depth 2, or create one with `new-project`. |
| Project selection is ambiguous | Use `use-project <exact-directory-basename>` from a workspace where that basename is unique. |
| Manifest is malformed | Fix the reported TOML field or value; `list projects` reports malformed manifests without treating them as valid projects. |
| Media is unsupported | Convert it to a supported static or animation format; AVIF must enter through an import workflow. |
| TUI fails outside a terminal | Run it in an interactive terminal, or use CLI commands with `--json` in automation. |
| Codepoint or registry conflict | Run `doctor`; use `--repair` for supported fixes. Use build `--force-remap` only when changing old mappings is acceptable. |
| A stale lock blocks work | Run `doctor --repair`; stale locks are recognized after ten minutes. |
| Installed glyphs do not render | Confirm the exact family with `list installed-fonts`, configure it in the application, then fully restart that application. |
| Clipboard copy fails | Install an available provider: `wl-copy` or `xclip` on Linux/Unix, `pbcopy` on macOS, or PowerShell/`clip.exe` on Windows. |

### Development Links

- [CI.md](CI.md)
- [RELEASE.md](RELEASE.md)
- [CONTRIBUTING.md](CONTRIBUTING.md)
