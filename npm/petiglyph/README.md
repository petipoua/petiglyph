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

Project discovery searches the current directory and descendants through depth 2 without following directory symlinks. Duplicate basenames are rejected with candidate paths.

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
  "version": "0.1.1",
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

After installation, fully restart applications that need to reload font state.

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
- [DEPENDENCY_SUPPLY_CHAIN.md](DEPENDENCY_SUPPLY_CHAIN.md)

Useful environment variables:

- `PETIGLYPH_NO_FFMPEG_PROMPT=1`
- `PETIGLYPH_TUI_DEBUG=1`
- `PETIGLYPH_TUI_DEBUG_LOG=/path/to/log`
