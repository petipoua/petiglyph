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

## Quick Start

```bash
petiglyph create my-font
# optional non-interactive create mode
petiglyph create my-font --no-launch
cd my-font
petiglyph
```

After `create`, place your images in `icons/` and use the TUI to tune thresholds, build, and install.

## CLI Commands

```bash
# create a new project in the current directory
petiglyph create my-font
petiglyph create my-font --no-launch

# launch the TUI for the current project
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

All non-`create` commands accept `--manifest` to target another project.

`petiglyph` (no subcommand) and `petiglyph tui` require an interactive terminal (TTY).

When `--manifest` is omitted, petiglyph checks `./petiglyph.toml` first, then scans one directory below:

- if exactly one project is found, it auto-selects that project while keeping the Home panel available
- if none or multiple projects are found, `petiglyph` / `petiglyph tui` start on the integrated Home panel where you can create a project folder

## Automation API Contract

`--json` is supported on:

- `build`
- `sample`
- `install-font`
- `uninstall-font`
- `uninstall`
- `doctor`

When `--json` is enabled, stdout is a single machine-readable envelope:

```json
{
  "ok": true,
  "command": "build",
  "version": "0.0.1",
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

`sample` now performs a managed install before printing glyphs so the sample codepoints are immediately available without terminal restart/cache-manual steps.

Install artifacts are immutable per build:

- installed files use progressive immutable names: `<font_slug>.ttf` first, then `<font_slug>_<hash>.ttf` with hash length auto-expanding only on conflicts
- active metadata is atomically switched to the new artifact
- previous active artifact for the same project/font identity is removed after switch

`uninstall-font` removes the active immutable artifact for the current manifest font identity.

`uninstall` is tool-level cleanup for the current user:
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

See [docs/release-notes-template.md](docs/release-notes-template.md) for the release checklist and schema-change callouts.

## Local Cargo Testing

Run these from the repository root while developing:

```bash
cargo fmt --all -- --check
cargo check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

Quick manual smoke test with the bundled sample manifest:

```bash
cargo run -- build --manifest ./petiglyph.toml
cargo run -- sample --manifest ./petiglyph.toml
cargo run -- tui --manifest ./petiglyph.toml
cargo run -- install-font --manifest ./petiglyph.toml --json
cargo run -- uninstall-font --manifest ./petiglyph.toml --json
cargo run -- uninstall --json
cargo run -- doctor --manifest ./petiglyph.toml --json
```

## TUI E2E with hty

To run process-level TUI E2E journeys (headless PTY automation with optional live watch):

```bash
# requires hty: https://hty.sh
./scripts/tui_e2e_hty.sh

# optional live observer + slower key steps
./scripts/tui_e2e_hty.sh --watch --step-delay-ms 250
```

The harness mirrors these critical flows:

- launch + quit from existing project
- create project from Home panel
- build shortcut writes artifacts
- glyph threshold override persists and clears
- workspace multi-project selection builds chosen project
- rescan picks up new image and rebuild includes it
- multi-project create/build/install/uninstall lifecycle in one session, using real `png`/`svg`/`jpg` fixtures from `icons/`

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
"logo.png" = { rows = 2, cols = 2, horizontal_bleed = true, vertical_bleed = false }
```

`threshold_overrides` stores per-file threshold tuning relative to `input_dir`.
`compositions` stores per-file tile grids; each tile becomes its own glyph and is arranged as a grid sample. Left/right TTF bleed defaults on for internal grid edges, while top/bottom bleed defaults off; use `horizontal_bleed = false` or `vertical_bleed = true` to override the defaults when seam hiding distorts important geometry.
`project_id` is managed automatically (generated if missing) and anchors install/Unicode ownership.

## TUI Keys

- TUI viewport is centered and capped at `148x46`; terminals smaller than `96x28` show a size warning screen
- `Tab`: cycle Home -> Glyphs
- `1` / `2`: switch between Home and Glyphs panels
- Home shows detected project folders, project creation controls, build/install actions, advanced generator actions (Compose Grid / Animate Glyph), a current-project panel for outputs/sample, and a machine-wide installed petiglyph font inventory with sample glyphs
- Home navigation: project list uses `↑` / `↓`; the create/build/install/generator controls use stacked rows with `←` / `→` moving within a row and `↑` / `↓` moving between rows; `Enter` opens the selected project or runs the focused Home action (including uninstall for the selected installed-font row)
- `R`: rescan the workspace project list and the active project's `icons/`
- Grid creation: `←` / `→` moves focus across the one-line control strip; `↑` / `↓` adjust rows/cols or toggle a focused bleed knob; `Space` also toggles a focused bleed knob; `Enter` creates on the Create button
- `j` / `k` or `↑` / `↓`: select glyph (Glyphs view)
- `Enter` / `Space`: expand or collapse a composed parent row in Glyphs view
- `c`: create a default `2x2` composition for the selected image
- `C`: remove the selected image composition from `petiglyph.toml`
- `←` / `→` or `+` / `-`: adjust threshold by 1 for selected glyph (Glyphs view)
- `PgUp` / `PgDn`: adjust threshold by 10 for selected glyph (Glyphs view)
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

## Notes

- supported inputs: `png`, `jpg`, `jpeg`, `webp`, `avif`, `bmp`, `gif`, `svg`
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
