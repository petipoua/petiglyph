# petiglyph

`petiglyph` turns a folder of images into a monochrome glyph font project.

It is designed for two modes:

- direct human use through a TUI (`petiglyph` / `petiglyph tui`)
- automation use by other tools via stable CLI commands and JSON output

## Project Model

Each font project is self-contained:

```text
my-font/
  petiglyph.toml
  icons/
  build/
```

- source images live in `icons/`
- generated artifacts live in `build/`
- config lives in `petiglyph.toml`

Deleting the project directory removes project-local assets only.

## Installation

Current distribution channels:

- GitHub Releases: prebuilt archives for Linux GNU/musl, macOS, and Windows.
- AUR: source-built `petiglyph` package for Arch-style local/AUR testing.
- npm: `petiglyph` meta package with platform-specific optional native packages under `@petiglyph/*`.
- PyPI: `maturin` binary wheels plus sdist for Python-based installation flows.

Runtime tools:

- `ffmpeg` is required for video import and animated media expansion.
- Packaged Arch installs declare `ffmpeg` as a runtime dependency.
- Interactive runs offer a one-time OS-aware `ffmpeg` setup hint when `ffmpeg` is missing and the command is not running in JSON mode.
- By default, petiglyph only shows the suggested install command and does not execute package-manager commands.
- To opt in to automatic command execution for that run, pass `--ffmpeg-auto-install`.
- To disable the one-time setup hint globally, set `PETIGLYPH_NO_FFMPEG_PROMPT=1`.

## Quick Start

```bash
petiglyph create my-font
# optional non-interactive create mode
petiglyph create my-font --no-launch
cd my-font
petiglyph
```

After `create`, add source images to `icons/` or drop/paste files into the Home panel creation workflows. Use the TUI to tune thresholds, compose grids, define animations, build, install, and sample the font.

## Home Panel Creation Workflows

From the Home panel you can launch four guided creation workflows:

- `Create glyph`
- `Create grid`
- `Create animated glyph`
- `Create animated grid glyph`

When a workflow is started, petiglyph shows a centered **Creation Workflow In Progress** popup. The popup owns the flow until it is completed or canceled:

- `Create glyph`: import images, then continue to the Glyphs panel.
- `Create grid`: import exactly one image, configure rows/columns/bleed in the popup, then create the composition.
- `Create animated glyph`: import frame media, optionally adjust selected frames, configure name/FPS, then create the animation.
- `Create animated grid glyph`: import frame media, configure name/FPS/grid/bleed, then create the animation.

Creation workflow previews use aspect-fit scaling: the same scale factor is applied on both axes, so previews fit the available panel without vertical or horizontal stretching.

Keyboard controls during the import popup:

- `Enter` continues to the next workflow step
- `Esc` cancels the workflow

Animated workflow import input types:

- `Create animated glyph` / `Create animated grid glyph` accept still images, GIF files, and video files in the popup import area.
- GIF/video inputs are expanded into deterministic per-frame PNG files in `icons/` and selected as animation frames.
- Video import requires `ffmpeg` available on `PATH`.
- Per-media extraction is capped at 1200 frames, and one import is capped at 3000 extracted frames.
- On first interactive run, if `ffmpeg` is missing, petiglyph shows a one-time OS-aware setup hint and records the outcome in managed user state.
- petiglyph never runs package-manager commands in JSON mode or non-interactive (non-TTY) execution paths.

## CLI Commands

```bash
# create a new project in the current directory
petiglyph create my-font
petiglyph create my-font --no-launch

# list local projects and managed installed fonts
petiglyph list
petiglyph list --json

# delete a project
petiglyph delete --manifest ./my-font/petiglyph.toml
petiglyph delete --manifest ./my-font/petiglyph.toml --json

# per-glyph threshold overrides
petiglyph set-threshold alpha.png 128
petiglyph set-threshold alpha.png 128 --json
petiglyph clear-threshold alpha.png
petiglyph clear-threshold alpha.png --json

# glyph workflow + overrides (parity command family)
petiglyph glyph create --input ./assets/alpha.png --threshold 128 --invert off --json
petiglyph glyph set-threshold alpha.png 128 --json
petiglyph glyph clear-threshold alpha.png --json
petiglyph glyph set-invert alpha.png --invert on --json

# composition workflow
petiglyph composition set alpha.png --rows 2 --cols 2 --horizontal-bleed weak --vertical-bleed off --json
petiglyph composition clear alpha.png --json

# grid workflow
petiglyph grid create --input ./assets/sheet.png --rows 2 --cols 2 --threshold 96 --invert off --json

# animation workflow
petiglyph animation create-standard --input ./assets/run.gif --fps 8 --name run --json
petiglyph animation create-grid --input ./assets/run.gif --rows 2 --cols 2 --fps 8 --name run_grid --json
petiglyph animation set-fps run --fps 12 --json
petiglyph animation delete run --json

# launch the TUI for the current project/workspace
petiglyph
petiglyph tui

# automation-friendly build
petiglyph build
petiglyph build --json
petiglyph build --force-remap

# build, install, refresh cache, and print sample string
petiglyph sample
petiglyph sample --json
petiglyph sample --force-remap

# build and install in user font location
petiglyph install-font
petiglyph install-font --json
petiglyph install-font --force-remap

# uninstall managed installed variants for the current project/font
petiglyph uninstall-font
petiglyph uninstall-font --json

# uninstall petiglyph tool state for current user (all managed petiglyph fonts + registry/metadata)
petiglyph nuke-everything
petiglyph nuke-everything --json

# inspect lock/registry health
petiglyph doctor
petiglyph doctor --repair
petiglyph doctor --json
```

Project-scoped commands accept `--manifest` to target a specific project:

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
- `tui`
- `build`
- `sample`
- `install-font`
- `uninstall-font`
- `doctor`

`list` is workspace/global-state scoped and does not accept `--manifest`. `nuke-everything` is current-user tool-state cleanup and does not accept `--manifest`.

`petiglyph` (no subcommand) and `petiglyph tui` require an interactive terminal (TTY).

For project-scoped commands, when `--manifest` is omitted, petiglyph checks `./petiglyph.toml` first, then scans one directory below:

- if exactly one project is found, automation commands use it and the TUI opens it while keeping the Home panel available
- if none or multiple projects are found, automation commands fail with guidance to pass `--manifest`
- if none or multiple projects are found, `petiglyph` / `petiglyph tui` start on the integrated Home panel where you can create or select a project folder

`petiglyph uninstall` is intentionally ambiguous and exits with guidance to use either `uninstall-font` or `nuke-everything`.

## Automation API Contract

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
- `nuke-everything`
- `doctor`

When `--json` is enabled, stdout is a single machine-readable envelope:

```json
{
  "ok": true,
  "command": "build",
  "version": "0.1.0",
  "data": {}
}
```

Top-level fields are stable:

- `ok` (`bool`)
- `command` (`string`)
- `version` (`string`)
- `data` (`object`)
- `error` (`object`, omitted on success and present on failures)

Failure mode rules:

- non-zero exit code
- `ok: false`
- actionable `error.message`
- optional `error.causes[]` for nested error chain context
- human logs are kept off stdout in JSON mode

## Integration Examples

### Shell

```bash
# Build and capture JSON
petiglyph build --manifest ./petiglyph.toml --json

# Install and parse with jq
petiglyph install-font --json | jq -r '.data.installed_ttf'

# Uninstall one project font during your app uninstall hook
petiglyph uninstall-font --manifest ./petiglyph.toml --json

# Uninstall all petiglyph-managed user state before removing the petiglyph tool itself
petiglyph nuke-everything --json
```

### Node.js

```js
import { spawnSync } from "node:child_process";

const run = (args) => {
  const out = spawnSync("petiglyph", args, { encoding: "utf8" });
  const payload = JSON.parse(out.stdout.trim());
  if (!out.status || out.status !== 0 || !payload.ok) {
    throw new Error(payload.error?.message ?? "petiglyph command failed");
  }
  return payload.data;
};

const build = run(["build", "--manifest", "./petiglyph.toml", "--json"]);
const install = run(["install-font", "--manifest", "./petiglyph.toml", "--json"]);
```

### Python

```python
import json
import subprocess


def run_petiglyph(*args):
    proc = subprocess.run(["petiglyph", *args], check=False, text=True, capture_output=True)
    payload = json.loads(proc.stdout.strip())
    if proc.returncode != 0 or not payload.get("ok"):
        raise RuntimeError(payload.get("error", {}).get("message", "petiglyph failed"))
    return payload["data"]

build = run_petiglyph("build", "--manifest", "./petiglyph.toml", "--json")
install = run_petiglyph("install-font", "--manifest", "./petiglyph.toml", "--json")
```

## Font Lifecycle Behavior

Install naming mode:

- TUI (`petiglyph` / `petiglyph tui` then `i`): uses project-scoped effective font identity `<project>-<font>`
- CLI (`petiglyph install-font`): uses project-scoped effective font identity `<project>-<font>`

Both are installed into a flat `petiglyph` directory under the user font root.

`install-font` is idempotent for a given effective font name.

`sample` performs a managed install before printing glyphs so the sample codepoints are immediately available without terminal restart/cache-manual steps.

Install artifacts are immutable per build:

- installed files use progressive immutable names: `<font_slug>.ttf` first, then `<font_slug>_<hash>.ttf` with hash length auto-expanding only on conflicts
- active metadata is atomically switched to the new artifact
- previous active artifact for the same project/font identity is removed after switch

`uninstall-font` removes the active immutable artifact for the current manifest font identity.

`nuke-everything` is tool-level cleanup for the current user:
- removes all managed petiglyph TTFs in the managed install directory
- removes petiglyph install metadata and machine state files
- removes the managed `petiglyph` install directory when empty

For cleanup compatibility with older installs, it also removes legacy fixed-name candidates when present:
- `<font>.ttf`
- `<project>-<font>.ttf`

Install identity is anchored to `project_id` to prevent slug collisions between similarly named projects.

Linux font fallback alias:

- petiglyph maintains `~/.config/fontconfig/conf.d/99-petiglyph.conf`
- alias families `Petiglyph` and `petiglyph` are kept in sync with installed project-scoped families
- this gives a stable terminal-facing family while preserving project-scoped install isolation
- alias file is removed automatically when no managed petiglyph fonts remain

Outcomes:

- `removed`
- `already_absent`
- blocked/error with actionable message when an expected managed path is invalid

Current install root by OS:

- Linux: `~/.local/share/fonts/petiglyph/`
- macOS: `~/Library/Fonts/petiglyph/`
- Windows: `%LOCALAPPDATA%/Microsoft/Windows/Fonts/petiglyph/`

Unicode ownership guardrails:

- petiglyph maintains a machine-wide Unicode registry at:
  - `~/.local/share/fonts/petiglyph/.unicode-registry.json` (Linux)
  - `~/Library/Fonts/petiglyph/.unicode-registry.json` (macOS)
  - `%LOCALAPPDATA%/Microsoft/Windows/Fonts/petiglyph/.unicode-registry.json` (Windows)
- each `project_id` gets a disjoint owned range in supplementary private-use space
- build-time allocation is lock-backed and project-stable; existing mappings remain pinned via `petiglyph.lock`
- if a project lock attempts to use codepoints owned by another project, build fails with an actionable conflict error
- if that conflict is intentional, `--force-remap` explicitly discards current lock mappings and rebuilds codepoints in a fresh owned range
- registry and install metadata updates are file-lock protected to avoid concurrent corruption/races

`doctor` guardrail tooling:

- `petiglyph doctor` inspects global install/registry health and project lock consistency when a project can be resolved
- `petiglyph doctor --repair` applies safe repairs:
  - removes stale lock files
  - removes orphan `.petiglyph-install-*.json` metadata entries
  - creates missing project Unicode registry assignment (when conflict-free)
- if no project can be auto-detected and `--manifest` is not provided, project checks are skipped with a warning in the report

## Command Stability Policy

Within `0.x`, command names and JSON top-level envelope fields are treated as stable for integrators.

- additive fields may be introduced inside `data`
- existing keys and semantics are not changed silently
- contract changes are called out in release notes

See [docs/release-notes-template.md](docs/release-notes-template.md) for release notes structure and schema-change callouts.
See [RELEASE-GUIDE.md](RELEASE-GUIDE.md) for the canonical multi-channel release runbook.
See [RELEASE-CHECKLIST.md](RELEASE-CHECKLIST.md) for the short operator checklist.

## Documentation Ownership Map

Use one source of truth per topic to minimize drift:

- User usage contract (commands, JSON envelope, UX expectations): `README.md`
- Release operations (GitHub/AUR/npm/PyPI execution details): `RELEASE-GUIDE.md`
- Release execution checklist (short operator sequence): `RELEASE-CHECKLIST.md`
- CI behavior (triggers/jobs/local equivalents/troubleshooting): `CI.md`
- Manual GitHub Actions cache maintenance helper: `scripts/gh_cache_delete.sh` (usage documented in `CI.md`)
- Dependency and supply-chain policy: `docs/dependency-supply-chain.md`
- Third-party dependency license snapshot artifact: `docs/THIRD_PARTY_LICENSES.md` (generated by `scripts/generate_third_party_licenses.sh`)
- Repository fixture asset provenance: `docs/assets-provenance.md`
- Contributor workflow and review expectations: `CONTRIBUTING.md`
- Agent-specific repository guardrails: `AGENTS.md`

When updating a topic, edit its canonical document first and replace duplicates elsewhere with links.

## Local Cargo Testing

Run these from the repository root while developing:

```bash
cargo fmt --all -- --check
cargo check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

Quick manual smoke test with a scratch project:

```bash
rm -rf /tmp/petiglyph-smoke
mkdir -p /tmp/petiglyph-smoke
mkdir -p /tmp/petiglyph-smoke/icons
cp test-assets/images/*.svg /tmp/petiglyph-smoke/icons/
cat > /tmp/petiglyph-smoke/petiglyph.toml <<'EOF'
input_dir = "icons"
out_dir = "build"
font_name = "Petiglyph Smoke"
glyph_size = 64
threshold = 64
codepoint_start = "U+100000"
EOF

cargo run -- build --manifest /tmp/petiglyph-smoke/petiglyph.toml
cargo run -- sample --manifest /tmp/petiglyph-smoke/petiglyph.toml
cargo run -- tui --manifest /tmp/petiglyph-smoke/petiglyph.toml
cargo run -- install-font --manifest /tmp/petiglyph-smoke/petiglyph.toml --json
cargo run -- uninstall-font --manifest /tmp/petiglyph-smoke/petiglyph.toml --json
cargo run -- nuke-everything --json
cargo run -- doctor --manifest /tmp/petiglyph-smoke/petiglyph.toml --json
```

Cross-OS clipboard/runtime smoke checks after cloning:

```bash
# Linux/macOS (uses cargo run by default)
./scripts/clipboard_smoke.sh

# optional: test an already-built binary instead of cargo run
./scripts/clipboard_smoke.sh --bin ./target/release/petiglyph

# clipboard checks only (skip CLI checks)
./scripts/clipboard_smoke.sh --skip-cli-checks
```

```powershell
# Windows PowerShell / pwsh
pwsh -File .\scripts\clipboard_smoke.ps1

# optional: test an already-built binary
pwsh -File .\scripts\clipboard_smoke.ps1 -PetiglyphPath .\target\release\petiglyph.exe
```

## TUI E2E with hty

To run process-level TUI E2E journeys (headless PTY automation with optional live watch):

```bash
# install hty (https://hty.sh)
curl -fsSL https://raw.githubusercontent.com/LatentEvals/hty/main/scripts/install.sh | sh
export PATH="$HOME/.local/bin:$PATH"

# verify local CLI behavior
hty --help

# run all journeys (1-10)
./scripts/tui_e2e_hty.sh

# optional: run only creation-workflow journeys
./scripts/tui_e2e_hty.sh --journey 6,7,8

# optional: live watch each session (auto terminal detection)
./scripts/tui_e2e_hty.sh --watch

# optional: force watcher terminal
./scripts/tui_e2e_hty.sh --watch --watch-terminal alacritty

# optional: render diagnostics only
./scripts/tui_e2e_hty.sh --render-probe-only
```

The harness mirrors these critical flows:

- launch + quit from existing project
- create project from Home panel
- build + rescan includes newly added sources in output artifacts
- glyph threshold override persists and clears in manifest
- workspace multi-project selection builds the selected project only
- creation workflow popup: create glyph
- creation workflow popup: create grid (rows/cols/bleed persisted)
- creation workflow popup: create animated glyph from deterministic GIF fixture
- install lifecycle via the Home panel in an isolated session `HOME`
- full user story journey: project creation, all creation workflows + glyph tweaks, install, sample copy checks, project/font cleanup, quit

The harness seeds the one-time FFmpeg prompt state in an isolated session `HOME` so focus stays on TUI interactions even when `ffmpeg` is not installed.
Use `--render-probe` (or `--render-probe-only`) when diagnosing hty repaint/erase regressions.

## Local AUR-Style Test Scripts

Use this script from repo root to simulate the AUR flow locally on Arch:

```bash
./scripts/aur.sh
./scripts/aur.sh uninstall
./scripts/aur.sh reinstall
./scripts/aur.sh build
./scripts/aur.sh install
./scripts/aur.sh pkgbuild
./scripts/aur.sh tarball
```

`scripts/aur.sh` builds in an isolated `.makepkg/` workspace so it does not touch your repo `src/` tree.
By default (`./scripts/aur.sh`), it performs a full reinstall cycle (remove, rebuild, install). Use `./scripts/aur.sh uninstall` for uninstall-only.
The uninstall step now attempts `petiglyph nuke-everything --json` first to clean current-user Petiglyph state, then runs `pacman -Rns`.
Source tarballs are created from your current working tree snapshot (tracked + untracked, excluding ignored files), so uncommitted local changes are included in package test builds.

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

[compositions]
"ollama.png" = { rows = 2, cols = 3 }
# optional: disable seam-hiding bleed if it distorts strict line geometry
"logo.png" = { rows = 2, cols = 2, horizontal_bleed = "off", vertical_bleed = "weak" }

[invert_overrides]
"claude.png" = true

[[animations]]
name = "runner_anim"
type = "standard"
fps = 10
frames = ["runner_001.png", "runner_002.png", "runner_003.png"]

[[animations]]
name = "spinner_anim"
type = "grid"
fps = 8
frames = ["spinner_sheet.png", "spinner_sheet_2.png"]
rows = 2
cols = 2
horizontal_bleed = "weak"
vertical_bleed = "off"
```

`threshold_overrides` stores per-file threshold tuning relative to `input_dir`.
`invert_overrides` stores per-file monochrome inversion.
`compositions` stores per-file tile grids; each tile becomes its own glyph and is arranged as a grid sample. Left/right TTF bleed defaults to `"weak"` for internal grid edges, while top/bottom bleed defaults to `"off"`; tune with `"off" | "weak" | "strong"` as needed.
`animations` stores animation definitions for standard and grid animated glyph playback and export.
`project_id` is managed automatically (generated if missing) and anchors install/Unicode ownership.

Legacy manifests may still contain boolean bleed values in compositions (`true`/`false`); they are accepted and normalized when loaded.

## TUI Keys

- TUI viewport is centered and capped at `148x46`; terminals smaller than `96x28` show a size warning screen
- `Tab` / `Shift+Tab`: cycle Home <-> Glyphs
- `1` / `2`: switch panels outside Home workflow mode; on Home with an active project, `1`-`4` start the four creation workflows
- `v`: toggle verbose path display
- Home shows detected project folders, project creation controls, build/install/delete actions, creation workflow launchers, a current-project panel for outputs/sample, and a machine-wide installed petiglyph font inventory with sample glyphs and animation exports
- Home navigation: project list uses `↑` / `↓`; the create/build/install/delete/workflow controls use stacked rows with `←` / `→` moving within a row and `↑` / `↓` moving between rows; `Enter` opens the selected project, renames the active project from the project list, or runs the focused Home action
- Installed-font rows: `Enter` copies the selected path/sample/animation export to the clipboard, or runs uninstall when the uninstall button is focused
- `R`: rescan the workspace project list and the active project's `icons/`
- Grid creation: `←` / `→` moves focus across the one-line control strip; `↑` / `↓` adjust rows/cols or toggle a focused bleed knob; `Space` also toggles a focused bleed knob; `Enter` creates on the Create button
- `j` / `k` or `↑` / `↓`: select glyph (Glyphs view)
- `Enter` / `Space`: expand or collapse a composed parent row in Glyphs view
- `c`: create a default `2x2` composition for the selected image
- `C`: remove the selected image composition from `petiglyph.toml`
- `D`: delete the selected animation or the animation linked to a selected frame row
- `←` / `→` or `+` / `-`: adjust threshold by 1 for selected glyph (Glyphs view)
- `PgUp` / `PgDn`: adjust threshold by 10 for selected glyph (Glyphs view)
- `↑` / `↓` on animation rows can also adjust FPS
- `Space` / `Enter` on invert-capable rows toggles invert
- `r`: clear selected glyph override (Glyphs view)
- `b`: build font outputs directly
- `i`: install font directly
- `q` / `Esc`: quit

## Build Outputs

`build/` contains:

- `*.ttf`
- `*.bdf`
- `glyph-map.json`
- `glyph-sample.txt`
- `previews/*.png`

Project root also contains:

- `petiglyph.lock` (stable source-file to codepoint allocations; keeps removed entries tombstoned to avoid unsafe reuse)
- `.petiglyph-build.lock` (ephemeral build lock while assigning/writing glyph mappings)

## Debug Pipeline

Run with `--debug` to emit per-step image-to-glyph artifacts and logs:

- `<project>/debug/pipeline.log`
- `<project>/debug/artifacts/*.png`

For each grayscale coverage/bitmap artifact, Petiglyph now writes two PNGs:

- raw generated glyph bitmap (`...png`)
- terminal cell-aspect preview (`..._terminal_preview.png`)

The debug session logs the cell geometry source at startup (for example `terminal-window-size ...` or `fallback:1x2`).
You can force the debug terminal cell geometry with `PETIGLYPH_DEBUG_CELL=<width>x<height>` (example: `PETIGLYPH_DEBUG_CELL=7x14`).
For composition grids, `ttf.bleed` log lines show which internal tile edges receive outline expansion in the final TTF, including separate horizontal and vertical units.

The TUI also has a separate event/debug log for interaction troubleshooting. Set `PETIGLYPH_TUI_DEBUG=1` to enable it. By default it writes `petiglyph-tui-debug.log` under the platform temp directory; set `PETIGLYPH_TUI_DEBUG_LOG=/path/to/log` to override the path. Set `PETIGLYPH_TUI_HTY_FULL_REPAINT=1` when diagnosing `hty` repaint behavior.

## Notes

- supported build inputs: `png`, `jpg`, `jpeg`, `webp`, `avif`, `bmp`, `gif`, `svg`
- animated popup import also accepts video files: `mp4`, `mov`, `mkv`, `webm`, `avi`, `m4v` (frame extraction via `ffmpeg`)
- if source alpha exists, alpha drives glyph coverage
- otherwise, border color is treated as background and contrast becomes coverage
- `glyph_size` is the generated terminal-cell height; one-cell glyphs use half that width so they fit normal tall terminal cells without needing a trailing blank
- standard glyphs fit and center inside that one-cell rectangle
- composition grids fit once into the full emitted grid, then each logical square tile is split into two one-cell rectangular glyphs
- composition tile TTF outlines can use small internal-edge expansion to hide rasterizer seams without changing glyph advance; vertical expansion is stronger than horizontal when enabled, but can be disabled for straighter row-crossing geometry
- default `codepoint_start` is `U+100000` to avoid common BMP private-use collisions
- private-use codepoints are East Asian Ambiguous width in Unicode; for stable terminal alignment keep ambiguous width as single-cell (for example: WezTerm `treat_east_asian_ambiguous_width_as_wide = false`, iTerm2 disable “Ambiguous characters are double-width”)
- while validating composite grids, avoid custom terminal line/cell spacing tweaks (`line_height`, `cell_height`, `font.offset.y`) that can introduce artificial row gaps
- if metadata/lock artifacts are incompatible, CLI errors include an actionable `petiglyph doctor --repair` hint
