# petiglyph (npm)

This meta package exposes the `petiglyph` CLI by selecting the native optional package for the current platform at runtime.

Supported npm native packages:

- `petiglyph-linux-x64-gnu`
- `petiglyph-linux-arm64-gnu`
- `petiglyph-linux-x64-musl`
- `petiglyph-linux-arm64-musl`
- `petiglyph-darwin-x64`
- `petiglyph-darwin-arm64`
- `petiglyph-win32-x64-msvc`
- `petiglyph-win32-arm64-msvc`

If optional dependencies are disabled or your platform is unsupported, install from GitHub Releases instead:

- https://github.com/petipoua/petiglyph/releases

`ffmpeg` must be available on `PATH` before `petiglyph` starts. It is used for AVIF import conversion and video/animated media workflows.

---

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

### CLI Workflows

Create an automation-oriented project:

```bash
petiglyph new-project my-font
```

Each project is self-contained:

```text
my-font/
  petiglyph.toml
  images/
  build/
```

`images/` holds source images and imported animation frames, `build/` holds generated artifacts, and `petiglyph.toml` holds project config.

Bare `petiglyph` and `petiglyph tui` open the workspace TUI. Scripted project operations use an exact project directory basename:

```bash
petiglyph use-project my-font build
petiglyph use-project my-font install-font
petiglyph use-project my-font show-sample
petiglyph use-project my-font doctor --repair
petiglyph use-project my-font tui
```

Project discovery searches the current directory and descendants through depth 2 without following directory symlinks. Inaccessible descendant directories are skipped. Duplicate basenames are rejected with candidate paths.

### Create Glyphs

The four creation types match the TUI:

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

# Each GIF/video frame split as a grid
petiglyph use-project my-font create animated-grid-glyph \
  --input dashboard.mp4 --name dashboard --fps 10 \
  --rows 2 --cols 4
```

One `--input` accepts multiple paths. Creation imports and configures sources without installing by default.

- `--build` also generates BDF, TTF, mapping, sample, and preview artifacts.
- `--install` implies build, performs managed installation and cache refresh, and prints the sample.
- `--threshold` defaults to `64`.
- `--invert` accepts `on` or `off`.
- Brightness, contrast, and gamma options control grayscale preprocessing.
- Static grayscale preprocessing defaults off; animated preprocessing defaults on.
- Grid horizontal bleed defaults to `weak`; vertical bleed defaults to `off`.

Supported static inputs are `png`, `jpg`, `jpeg`, `webp`, `bmp`, `gif`, `svg`, and `avif`. AVIF is converted to project-local PNG. Animation media supports `gif`, `mp4`, `mov`, `mkv`, `webm`, `avi`, and `m4v`.

### Configure Sources

```bash
petiglyph use-project my-font configure glyph logo.png \
  --threshold 88 --invert on

petiglyph use-project my-font configure glyph logo.png \
  --clear-threshold

petiglyph use-project my-font configure grid-glyph icons.png \
  --rows 2 --cols 8 \
  --horizontal-bleed strong --vertical-bleed off

petiglyph use-project my-font configure animation spinner \
  --fps 12 --threshold 72 --invert off

petiglyph use-project my-font delete animation spinner
```

Grid dimensions and bleed are rejected for non-grid animations. Clap rejects mutually exclusive threshold operations.

### Tuning Parameters And What They Do

All image-facing tuning in petiglyph follows the same broad pipeline:

1. Optional grayscale preprocessing during import.
2. Aspect-fit scaling into the glyph or grid canvas.
3. Thresholding into on/off pixels.
4. Optional invert after thresholding.
5. Optional grid bleed when adjacent grid tiles become final font outlines.

The same ideas show up in both the TUI and the CLI. The Home popup gives you live previews; CLI `create` and `configure` commands write the saved settings into `petiglyph.toml`.

#### Threshold And Invert

- `threshold`: `0..=255`, default `64`.
  Higher values keep only darker or more opaque pixels, so glyphs usually become thinner, cleaner, and more selective. Lower values include lighter grays and softer antialiasing, so glyphs usually become fuller or heavier.
- `invert`: `on` or `off`, default `off`.
  This flips the thresholded result after thresholding. Use it when your source has the opposite polarity from what you want, for example light foreground on dark background instead of dark foreground on light background.
- Manifest behavior:
  The top-level `threshold` value is the project-wide default. Per-source changes are stored in `threshold_overrides` and `invert_overrides`, keyed by paths relative to `images/`. `--clear-threshold` removes a per-source threshold override and falls back to the project default.

#### Grayscale Preprocessing

- `grayscale_enabled`:
  Controls whether petiglyph first converts imported raster media to grayscale and applies tone adjustments before later thresholding. Default is `off` for static image creation and `on` for animated GIF/video creation.
- `grayscale_brightness`: `-80..=80`, default `0`.
  Adds or subtracts brightness from the grayscale image. Positive values brighten the whole source and can wash out faint dark detail. Negative values darken the source and can make weak strokes survive thresholding.
- `grayscale_contrast`: `-80..=80`, default `0`.
  Pushes tones away from or toward mid-gray. Positive values increase separation between dark and light areas, which can make edges crisper at a given threshold. Negative values flatten the image and keep transitions softer.
- `grayscale_gamma_percent`: `50..=200`, default `100` which is gamma `1.00`.
  Remaps midtones before contrast and brightness. Values above `100` brighten midtones; values below `100` darken them. This is often the most useful control when the source is neither clearly too dark nor clearly too bright.
- Processing order:
  Petiglyph converts to grayscale, applies gamma, then contrast, then brightness, then threshold.
- Import behavior:
  In the Home creation workflow these controls update the preview live before you continue. For imported raster files they also affect the imported pixels that petiglyph keeps in the project for future builds. SVG sources are not rewritten, and AVIF imports are first converted to PNG.

#### Grid Slicing And Seam Controls

- `rows` and `cols`: both must be `> 0`.
  These define how a grid source is split. More rows or columns means more glyph cells and less detail per cell. Petiglyph fits the whole source to the full grid first, then slices it into cells, so changing the grid changes the composition of every tile.
- `horizontal_bleed`: `off`, `weak`, or `strong`, default `weak`.
  Allows neighboring grid glyphs to overlap slightly across left/right internal boundaries in the generated outlines. Use it to reduce visible seams when adjacent grid tiles are displayed together. This is fairly compatible across terminal emulators, so `weak`` is good usually.
- `vertical_bleed`: `off`, `weak`, or `strong`, default `off`.
  Same idea for top/bottom internal boundaries. This is NOT very compatible across terminal emulators, and it can make round shapes appear wobbly because shapes are just propagated in a straight line. That's why the default is `off`. Stripes over a consistent shape is better than weird wobbly shapes that are not consistent across terminals you render them on.
- Bleed strength:
  `off` keeps cell edges hard-clipped, `weak` adds mild overlap, and `strong` doubles that overlap. Bleed affects the final outline geometry, not the thresholding math itself.
- Scope:
  Bleed settings only apply to grid glyphs and animated grid glyphs. Standard single-glyph animations do not use rows, cols, or bleed.

#### Animation Controls

- `fps`: `1..=30`.
  Controls animation playback speed only. It changes how quickly frames advance in the installed font sample and other animation outputs; it does not modify the frame images themselves.
- `name`:
  The manifest identifier for an animation. It affects how you refer to the animation later with `configure animation ...` or `delete animation ...`, but it does not change the pixels.
- Home popup `Frames` control:
  Chooses which imported frame you are previewing while tuning. It is preview-only and does not change the saved animation.
- Home popup `Export Test` count: `1..=120`.
  Chooses how many frames the popup exports for a temporary test image/export flow. It does not change the saved animation or its FPS.

#### Project-Level Rendering Knobs

- `glyph_size`, default `64`.
  Controls the raster working size used when fitting source imagery into glyph cells. Higher values preserve more detail and subtle edges before thresholding, but they can also preserve more noise. Lower values simplify shapes sooner.
- `font_name`:
  Controls the generated font family name. It affects installation and display naming, not image processing.
- `codepoint_start`, default `U+100000`.
  Sets the first Unicode codepoint assigned to generated glyphs. It affects mapping and interoperability, not rasterization.
- `input_dir` and `out_dir`, defaults `images` and `build`.
  Control where petiglyph reads project-local sources from and where it writes artifacts.

### Build And Install

```bash
petiglyph use-project my-font build
petiglyph use-project my-font install-font
petiglyph use-project my-font show-sample
```

Installation is always explicit. `show-sample` only reads `build/glyph-sample.txt`; it does not build or install and reports the required build command when the artifact is absent.

### List And Delete

```bash
petiglyph list projects
petiglyph list installed-fonts
petiglyph delete-project my-font another-project
petiglyph uninstall-font "my-font Petiglyph"
petiglyph uninstall-all-fonts
petiglyph doctor --repair
```

`list installed-fonts` prints the exact managed family identifier accepted by `uninstall-font`. Batch deletion and uninstall validate all targets before changing files.

Project listing includes directory name, relative path, font name, manifest path, project ID, and malformed-manifest warnings. Installed-font listing includes family, ownership, TTF path, manifest path, and stale metadata/artifact warnings.

### JSON Automation

Add `--json` to automation commands:

```bash
petiglyph list projects --json
petiglyph use-project my-font create glyph --input logo.svg --build --json
petiglyph use-project my-font doctor --json
```

Output uses a stable envelope:

```json
{
  "ok": true,
  "command": "use-project.create.glyph",
  "version": "0.1.5",
  "data": {},
  "error": null
}
```

Failures return a non-zero exit code and include `error.message` plus nested causes.

### Runtime Notes

`ffmpeg` is checked before every command:

- Pass `--ffmpeg-auto-install` to run the detected platform install command.
- Set `PETIGLYPH_NO_FFMPEG_PROMPT=1` to suppress the interactive hint.

The TUI requires a terminal. Its Home panel supports project selection, all four creation workflows, previewing, tuning, building, installing, and copying samples. On Windows, creation workflows use a native file picker.

Managed install roots:

- Linux: `~/.local/share/fonts/petiglyph/`
- macOS: TTFs under `~/Library/Fonts/`, metadata under `~/Library/Fonts/petiglyph/`
- Windows: `%LOCALAPPDATA%/Microsoft/Windows/Fonts/petiglyph/`

After installation, fully restart applications that need to reload font state. On Windows
Terminal 1.21 or newer, add the installed petiglyph family after the primary terminal font
as a fallback, for example `"Cascadia Mono, my-font Petiglyph"`.

### Manifest Defaults

```toml
input_dir = "images"
out_dir = "build"
font_name = "Petiglyph"
glyph_size = 64
threshold = 64
codepoint_start = "U+100000"
```

`project_id` is managed automatically. Threshold and invert overrides use source keys relative to `images/`.

### Development

Useful docs:

- [CI.md](CI.md)
- [RELEASE.md](RELEASE.md)
- [CONTRIBUTING.md](CONTRIBUTING.md)

Useful environment variables:

- `PETIGLYPH_NO_FFMPEG_PROMPT=1`
- `PETIGLYPH_TUI_DEBUG=1`
- `PETIGLYPH_TUI_DEBUG_LOG=/path/to/log`
