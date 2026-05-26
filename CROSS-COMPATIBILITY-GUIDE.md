# CROSS-COMPATIBILITY-GUIDE

Last verified: 2026-05-23

## 1. Purpose

This guide records the current cross-platform state of `petiglyph` and the release/runtime constraints that matter when changing code, packaging, or docs.

`petiglyph` is a Rust binary crate that exposes both:

- CLI automation commands.
- An interactive TUI built with `ratatui` + `crossterm`.

Supported distribution surfaces in this repository are:

- GitHub Release binary archives.
- AUR/source-style Arch package metadata.
- npm meta package plus optional native platform packages.
- PyPI/TestPyPI `maturin` binary wheels plus sdist.

## 2. Verified Repository State

- Package version: `0.1.0` in `Cargo.toml`.
- Rust edition: `2024`.
- CLI parser: `clap`.
- TUI stack: `ratatui` + `crossterm`.
- Image decode/rendering: `image` with `avif`, `avif-native`, `bmp`, `gif`, `jpeg`, `png`, `webp`; SVG through `resvg`.
- Video/GIF frame expansion: `src/animation_media.rs`; video expansion shells out to `ffmpeg`.
- Install/uninstall lifecycle: `src/install.rs` with Linux, macOS, and Windows branches.
- Health/repair checks: `src/doctor.rs`.
- Direct release workflow: `.github/workflows/release.yml`.
- npm publish workflow: `.github/workflows/npm-publish.yml`.
- PyPI/TestPyPI publish workflow: `.github/workflows/pypi-publish.yml`.
- npm package layout exists under `npm/`.
- PyPI metadata exists in `pyproject.toml`.
- Arch local/AUR helpers exist in `PKGBUILD`, `scripts/aur.sh`, and `scripts/release_prepare_aur.sh`.

## 3. Runtime Behavior By Platform

User font roots:

- Linux: `~/.local/share/fonts/petiglyph/`
- macOS: `~/Library/Fonts/petiglyph/`
- Windows: `%LOCALAPPDATA%/Microsoft/Windows/Fonts/petiglyph/`, falling back under `~/AppData/Local/` if `LOCALAPPDATA` is unavailable

Font cache refresh:

- Linux: `fc-cache -f <font_root>`.
- macOS: `atsutil databases -removeUser`.
- Windows: PowerShell broadcast of `WM_FONTCHANGE` through `SendMessageTimeout`.

Linux-only fontconfig behavior:

- `~/.config/fontconfig/conf.d/99-petiglyph.conf` is maintained while managed fonts exist.
- Alias families `Petiglyph` and `petiglyph` point at installed project-scoped families.
- The alias file is removed when no managed petiglyph fonts remain.
- `fc-match` diagnostics are Linux-specific and must not be documented as universal.

Managed state:

- Immutable installed TTF artifacts, metadata, first-install state, the Unicode registry, and registry/install locks live under the managed `petiglyph/` font directory.
- `nuke-everything` removes managed petiglyph fonts, metadata, machine state, registry files, and the managed directory when empty.
- Install and Unicode registry writes are file-lock protected and stale locks are repairable with `doctor --repair`.

## 4. External Runtime Tools

`ffmpeg`:

- Required for video import and animated media expansion from video files.
- GIF expansion uses Rust image decoding; video expansion uses `ffmpeg`.
- First interactive non-JSON runs offer a one-time OS-aware install prompt when `ffmpeg` is missing.
- Prompt state is stored as `.ffmpeg-setup-prompt-v1.json` under the managed install directory.
- Arch packaging declares `depends=('ffmpeg')`.

Clipboard providers used by the TUI installed-font card:

- Linux Wayland: `wl-copy`.
- Linux non-Wayland/default fallback: `xclip -selection clipboard`, then `wl-copy`.
- macOS: `pbcopy`.
- Windows: PowerShell `Set-Clipboard`, then `clip.exe`.
- Failures are aggregated and reported in TUI status text.

TUI debug logging:

- Enable with `PETIGLYPH_TUI_DEBUG=1`.
- Default path is `std::env::temp_dir()/petiglyph-tui-debug.log`.
- Override with `PETIGLYPH_TUI_DEBUG_LOG=/path/to/log`.
- Use `PETIGLYPH_TUI_HTY_FULL_REPAINT=1` when diagnosing `hty` repaint behavior.

## 5. Supported Inputs

Build/source image extensions:

- `png`, `jpg`, `jpeg`, `webp`, `avif`, `bmp`, `gif`, `svg`

Animated creation workflow media extensions:

- Still image formats above.
- GIF files.
- Video files: `mp4`, `mov`, `mkv`, `webm`, `avi`, `m4v`.

Animated media limits:

- Maximum 1200 extracted frames per media input.
- Maximum 3000 extracted frames per import operation.
- Extracted frames use deterministic names in `icons/` based on source stem, source hash, and frame index.

## 6. Distribution Matrix

### GitHub Release Archives

`.github/workflows/release.yml` builds 8 targets:

| Target | Runner | Archive |
| --- | --- | --- |
| `x86_64-unknown-linux-gnu` | Ubuntu | `.tar.gz` |
| `aarch64-unknown-linux-gnu` | Ubuntu/cross action | `.tar.gz` |
| `x86_64-unknown-linux-musl` | Ubuntu/cross action | `.tar.gz` |
| `aarch64-unknown-linux-musl` | Ubuntu/cross action | `.tar.gz` |
| `x86_64-apple-darwin` | macOS 13 | `.tar.gz` |
| `aarch64-apple-darwin` | macOS latest | `.tar.gz` |
| `x86_64-pc-windows-msvc` | Windows latest | `.zip` |
| `aarch64-pc-windows-msvc` | Windows latest | `.zip` |

Release archives include the binary, `README.md`, and `LICENSE`. The publish job generates `SHA256SUMS`, produces artifact attestations, and attaches assets to the GitHub Release.

### npm

`npm/petiglyph` is the meta package. It dispatches to one optional native package by `process.platform`, `process.arch`, and Linux libc detection:

- `@petiglyph/petiglyph-linux-x64-gnu`
- `@petiglyph/petiglyph-linux-arm64-gnu`
- `@petiglyph/petiglyph-linux-x64-musl`
- `@petiglyph/petiglyph-linux-arm64-musl`
- `@petiglyph/petiglyph-darwin-x64`
- `@petiglyph/petiglyph-darwin-arm64`
- `@petiglyph/petiglyph-win32-x64-msvc`
- `@petiglyph/petiglyph-win32-arm64-msvc`

The publish workflow verifies `gh release verify`, validates `SHA256SUMS`, stages binaries from release archives, runs `npm pack --dry-run`, publishes platform packages first, and publishes the meta package last.

### PyPI/TestPyPI

`.github/workflows/pypi-publish.yml` builds:

- `x86_64-unknown-linux-gnu` manylinux 2014 wheel.
- `aarch64-unknown-linux-gnu` manylinux 2014 wheel.
- `x86_64-apple-darwin` wheel.
- `aarch64-apple-darwin` wheel.
- `x86_64-pc-windows-msvc` wheel.
- `aarch64-pc-windows-msvc` wheel.
- sdist.

The current PyPI workflow does not build musllinux wheels. Musl binaries are currently distributed through GitHub archives and npm platform packages.

### AUR / Local Arch Packaging

- `PKGBUILD` and `scripts/aur.sh` use `depends=('ffmpeg')` and `makedepends=('cargo')`.
- `scripts/aur.sh` creates a tarball from the current working tree snapshot for local package testing.
- `scripts/release_prepare_aur.sh` prepares release-grade `PKGBUILD` and `.SRCINFO` from an immutable GitHub tag tarball and computes a real SHA256.

## 7. CLI/TUI Portability Contracts

- `petiglyph` with no subcommand launches the interactive workspace TUI.
- `petiglyph tui` launches the same TUI surface and accepts project overrides only when a concrete project is resolvable or `--manifest` is passed.
- TUI launch requires both stdin and stdout to be terminals; non-TTY runs fail with an explicit terminal-required error.
- Manifest auto-detection checks `./petiglyph.toml` first, then one directory below the current directory.
- Automation commands that need a project fail on zero or multiple detected projects unless `--manifest` is passed.
- `doctor` can run global checks without a manifest and adds project checks when a project can be resolved.
- `list` is workspace/global-state scoped and has no `--manifest` option.
- `nuke-everything` is current-user tool-state cleanup and has no `--manifest` option.

## 8. Validation Commands

Repository-local checks:

```bash
cargo fmt --all -- --check
cargo check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo run -- --help
cargo run -- tui </dev/null
```

Expected non-TTY behavior:

- `cargo run -- tui </dev/null` should fail with an explicit terminal-required error.

Clipboard/runtime smoke:

```bash
./scripts/clipboard_smoke.sh
./scripts/clipboard_smoke.sh --bin ./target/release/petiglyph
./scripts/clipboard_smoke.sh --skip-cli-checks
```

Windows:

```powershell
pwsh -File .\scripts\clipboard_smoke.ps1
pwsh -File .\scripts\clipboard_smoke.ps1 -PetiglyphPath .\target\release\petiglyph.exe
```

TUI E2E:

```bash
./scripts/tui_e2e_hty.sh
./scripts/tui_e2e_hty.sh --watch --step-delay-ms 250
```

Before modifying `hty` flows, validate local `hty --help` and check upstream docs:

- https://github.com/LatentEvals/hty
- https://hty.sh

## 9. Known Risks And Policy Notes

- macOS direct artifacts are unsigned and not notarized unless a release explicitly states otherwise.
- Windows direct artifacts are unsigned unless a release explicitly states otherwise.
- macOS and Windows ARM64 artifacts should remain called out as limited-runtime-validation/unstable until direct runtime testing exists.
- `image` `avif-native` can introduce native build requirements; keep CI/package builds watched for AVIF codec regressions.
- PyPI does not currently ship musllinux wheels.
- Windows font refresh depends on PowerShell availability.
- Clipboard support depends on external platform tools; failures should remain user-visible and non-fatal.
- Composite grid rendering assumes tight terminal cell stacking; custom line-height/cell-height/font-offset settings can create apparent gaps unrelated to generated fonts.

## 10. Change Guidance

When changing platform behavior:

- Update this guide and `README.md` in the same change.
- Keep `AGENTS.md` aligned with command/package/test workflow changes that affect future agents.
- Keep `PKGBUILD`, `scripts/aur.sh`, npm package versions, and README JSON sample version synchronized through `scripts/release_sync_versions.sh`.
- Prefer extending `scripts/tui_e2e_hty.sh` for TUI E2E rather than reintroducing `expect`/PTY frameworks.
- Do not document a new platform as first-class until install, font-cache refresh, clipboard, TUI launch, and at least a minimal build/install/uninstall lifecycle have been validated there.
