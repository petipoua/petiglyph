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
cd my-font
petiglyph
```

After `create`, place your images in `icons/` and use the TUI to tune thresholds, build, and install.

## CLI Commands

```bash
# create a new project in the current directory
petiglyph create my-font

# launch the TUI for the current project
petiglyph
petiglyph tui

# automation-friendly build
petiglyph build
petiglyph build --json

# build and print sample string
petiglyph sample
petiglyph sample --json

# build and install in user font location
petiglyph install-font
petiglyph install-font --json

# uninstall managed installed variants for the current project/font
petiglyph uninstall-font
petiglyph uninstall-font --json
```

All non-`create` commands accept `--manifest` to target another project.

When `--manifest` is omitted, petiglyph checks `./petiglyph.toml` first, then scans one directory below:

- if exactly one project is found, it auto-selects that project while keeping the Home panel available
- if none or multiple projects are found, `petiglyph` / `petiglyph tui` start on the integrated Home panel where you can create a project folder

## Automation API Contract

`--json` is supported on:

- `build`
- `sample`
- `install-font`
- `uninstall-font`

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
- human logs are kept off stdout in JSON mode

## Integration Examples

### Shell

```bash
# Build and capture JSON
petiglyph build --manifest ./petiglyph.toml --json

# Install and parse with jq
petiglyph install-font --json | jq -r '.data.installed_ttf'

# Uninstall during app uninstall hook
petiglyph uninstall-font --manifest ./petiglyph.toml --json
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

Install naming modes:

- TUI (`petiglyph` / `petiglyph tui` then `i`): installs `<font>.ttf`
- CLI (`petiglyph install-font`): installs `<project>-<font>.ttf`

Both are installed into a flat `petiglyph` directory under the user font root.

`install-font` is idempotent for a given effective font name.

`uninstall-font` removes both managed candidates for the current manifest font:

- `<font>.ttf`
- `<project>-<font>.ttf`

If the project directory name changes, the `<project>-<font>.ttf` candidate changes too; run `install-font` again to publish the new name.

Outcomes:

- `removed`
- `already_absent`
- blocked/error with actionable message when an expected managed path is invalid

Current install root by OS:

- Linux: `~/.local/share/fonts/petiglyph/`
- macOS: `~/Library/Fonts/petiglyph/`
- Windows: `%LOCALAPPDATA%/Microsoft/Windows/Fonts/petiglyph/`

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
```

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
```

`threshold_overrides` stores per-file threshold tuning relative to `input_dir`.

## TUI Keys

- TUI viewport is centered and capped at `148x46`; terminals smaller than `96x28` show a size warning screen
- `Tab`: cycle Home -> Glyphs
- `1` / `2`: switch between Home and Glyphs panels
- Home shows detected project folders, project creation controls, build/install actions, advanced generator placeholder buttons inside the projects card, a current-project panel for outputs/sample, and a machine-wide installed petiglyph font inventory with sample glyphs
- Home navigation: project list uses `↑` / `↓`; the create/build/install/generator controls use stacked rows with `←` / `→` moving within a row and `↑` / `↓` moving between rows; `Enter` opens the selected project or runs the focused Home action
- `R`: rescan the workspace project list and the active project's `icons/`
- `j` / `k` or `↑` / `↓`: select glyph (Glyphs view)
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

## Notes

- supported inputs: `png`, `jpg`, `jpeg`, `webp`, `avif`, `bmp`, `gif`, `svg`
- if source alpha exists, alpha drives glyph coverage
- otherwise, border color is treated as background and contrast becomes coverage
- default `codepoint_start` is `U+100000` to avoid common BMP private-use collisions
