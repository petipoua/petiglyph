# CROSS-COMPATIBILITY-GUIDE

Last verified: 2026-05-19

## 1. Purpose and Scope

This guide defines how to make `petiglyph` reliably usable across:

- Arch Linux, including the local/AUR packaging target
- Other glibc Linux distributions
- musl Linux distributions such as Alpine
- macOS on Intel and Apple Silicon
- Windows on x64, with ARM64 as a staged target

It covers both runtime surfaces:

- CLI commands
- Interactive TUI (`ratatui` + `crossterm`)

It also covers planned distribution through three channels:

- AUR
- npm
- PyPI

This is a decision-grade document: it separates verified repository state, external platform facts, recommended choices, risks, and implementation tasks.

---

## 2. Verified Current State

### 2.1 Technology and runtime model

- Language/runtime: Rust binary crate named `petiglyph`.
- Current package version: `0.0.1` in `Cargo.toml`.
- TUI stack: `ratatui` + `crossterm`.
- CLI parser: `clap`.
- Font install lifecycle includes OS-specific logic in `src/install.rs`.
- Animated/video import shells out to `ffmpeg` in `src/animation_media.rs`.
- The repo currently has Arch packaging helpers (`PKGBUILD`, `scripts/aur.sh`) but no npm or PyPI packaging files yet.

### 2.2 Existing OS support in code

`src/install.rs` supports these platforms through `env::consts::OS`:

- `linux`
- `macos`
- `windows`

User font roots currently used:

- Linux: `~/.local/share/fonts`
- macOS: `~/Library/Fonts`
- Windows: `%LOCALAPPDATA%/Microsoft/Windows/Fonts`, falling back to `~/AppData/Local/Microsoft/Windows/Fonts` if `LOCALAPPDATA` is unavailable

Managed petiglyph state is stored under a `petiglyph/` child directory inside the per-user font root. This includes immutable TTF artifacts, install metadata, machine state, and `.unicode-registry.json`.

### 2.3 Font cache and discovery behavior

Current refresh commands:

- Linux: `fc-cache -f <font_root>`
- macOS: `atsutil databases -removeUser`
- Windows: PowerShell command that broadcasts `WM_FONTCHANGE` through `SendMessageTimeout`

Linux-only fontconfig behavior:

- `src/install.rs` maintains `~/.config/fontconfig/conf.d/99-petiglyph.conf`.
- The alias exposes `Petiglyph` / `petiglyph` as stable terminal-facing families while preserving project-scoped installed font names.
- `fc-match` / fontconfig diagnostics are Linux-specific and must not be presented as universal checks.

### 2.4 Important portability observations in the codebase

1. `src/tui.rs` has a hardcoded debug log path: `/tmp/petiglyph-tui-debug.log`.
2. The TUI clipboard helper is Linux-focused: `wl-copy`, then `xclip -selection clipboard`.
3. The install/cache code has platform-specific branches, but most direct test coverage for install behavior is Linux-gated.
4. Windows font refresh depends on PowerShell availability.
5. Non-interactive TUI launch is already guarded with `IsTerminal` checks and fails explicitly when no terminal is attached.
6. Test-only paths still contain `/tmp/...` examples; those are not runtime compatibility issues unless moved into production code.

### 2.5 Packaging inconsistency already present

There is a real discrepancy:

- `PKGBUILD` currently lists `depends=('ffmpeg')`.
- `scripts/aur.sh` currently generates `depends=()`.

Impact: generated AUR package metadata can miss the video import runtime dependency unless this policy is made explicit and the files are synchronized.

### 2.6 Dependency risk affecting cross-platform builds

`Cargo.toml` currently uses:

```toml
image = { version = "0.25", default-features = false, features = ["avif", "avif-native", "bmp", "gif", "jpeg", "png", "webp"] }
```

`avif-native` can introduce native build requirements through AVIF codec dependencies. This is a portability and CI-risk item for cross builds and binary packaging.

---

## 3. Compatibility Dimensions to Treat Explicitly

### 3.1 Build-time compatibility

- Rust target triple support and tier expectations.
- Linker/toolchain availability per target.
- Native/system library requirements introduced by dependencies.
- Minimum OS/libc assumptions for produced binaries.
- Whether cross-compiled artifacts are only built, or actually smoke-tested on the target OS.

### 3.2 Install/distribution compatibility

- Registry-specific package format behavior.
- Per-platform artifact selection.
- Executable exposure in PATH.
- User-font install locations and cache refresh behavior.
- Whether optional runtime tools are bundled, declared as dependencies, or checked at runtime.

### 3.3 Runtime compatibility

- TTY availability for TUI launch.
- Terminal raw mode, alternate screen, bracketed paste, and keyboard enhancement support.
- Unicode private-use rendering and terminal width behavior.
- External command availability (`ffmpeg`, `fc-cache`, `atsutil`, PowerShell, clipboard tools).
- Filesystem and environment conventions (`HOME`, `USERPROFILE`, `LOCALAPPDATA`, path separators, executable suffixes).

### 3.4 Trust and security compatibility

- Source-to-binary traceability.
- Release checksums.
- Registry trusted publishing and provenance.
- GitHub artifact attestations.
- macOS signing/notarization.
- Windows Authenticode signing and SmartScreen expectations.

---

## 4. Platform-Level Compatibility Baseline

### 4.1 Recommended first-class targets

Initial release matrix:

- `x86_64-unknown-linux-gnu`
- `aarch64-unknown-linux-gnu`
- `x86_64-unknown-linux-musl`
- `aarch64-unknown-linux-musl`
- `x86_64-apple-darwin`
- `aarch64-apple-darwin`
- `x86_64-pc-windows-msvc`

Phase 2 target:

- `aarch64-pc-windows-msvc`

Important correction: not every target above has the same Rust support tier. Current Rust docs list the common GNU Linux, Apple Darwin, and MSVC Windows targets above as Tier 1 with host tools, while musl Linux targets are Tier 2 build targets. Treat musl artifacts as supported by this project only after CI builds and smoke tests prove they work for `petiglyph`.

### 4.2 GNU vs musl strategy for Linux

Decision options:

1. GNU only.
2. musl only.
3. both GNU and musl.

Recommendation:

- Build both GNU and musl binaries.
- Use GNU as the default for mainstream Linux distributions.
- Offer musl for Alpine/container-heavy environments.
- Do not use musl as the only Linux artifact unless every dependency and runtime command path is validated there.

Reason:

- Most desktop Linux users run glibc distributions.
- musl broadens deployment portability but can expose different dynamic-linking, libc, and external-tool assumptions.

### 4.3 Windows target strategy

Recommendation:

- Prefer `*-pc-windows-msvc` artifacts for public Windows downloads and registry packages.
- Treat MinGW/GNU Windows targets as non-primary unless a concrete user need appears.
- Use `.zip` release archives and ensure the executable is named `petiglyph.exe`.
- Keep PowerShell-dependent font refresh behavior documented.

Reason:

- MSVC is the conventional Rust target for Windows binary distribution.
- The Rust project lists x64 and ARM64 MSVC Windows targets as Tier 1 with host tools.

### 4.4 macOS target strategy

Recommendation:

- Build `x86_64-apple-darwin` and `aarch64-apple-darwin` separately.
- Consider a universal2 archive only if users strongly prefer a single macOS download.
- Sign and notarize direct-download release artifacts before calling macOS support first-class for non-developer users.

Reason:

- Apple Silicon and Intel macOS users are still both relevant.
- Unsigned CLI binaries work for developers, but Gatekeeper warnings create avoidable support load for broader distribution.

### 4.5 TUI backend choice

Recommendation:

- Keep `crossterm` as the default backend.

Reason:

- It is the mainstream cross-platform backend in the `ratatui` ecosystem.
- It supports Unix and Windows terminal backends.
- The current code already uses crossterm feature probing and degrades when some terminal enhancements fail.

---

## 5. Registry Strategy: AUR, npm, PyPI

### 5.1 AUR and Arch Linux

#### Packaging model

- Source-based package via `PKGBUILD` and `.SRCINFO`.
- Build in a clean chroot for correctness before publishing.
- Use local `scripts/aur.sh` as the canonical helper only after it is aligned with `PKGBUILD`.

#### Required decisions

1. Package type:

- `petiglyph`: stable release tarball.
- Optional future `petiglyph-git`: VCS package.

2. Runtime dependencies policy:

- If video import is considered part of the default installed feature set, include `ffmpeg` in `depends`.
- If video import is optional, either keep `ffmpeg` in `optdepends` with excellent runtime errors, or document that AUR installs intentionally omit video support until the user installs `ffmpeg`.
- Keep `PKGBUILD` and `scripts/aur.sh` synchronized.

3. Architecture declarations:

- Current `arch=('x86_64')` is honest for the local package.
- Add `aarch64` only after an Arch ARM build has been tested.
- Do not declare `any`; this is a compiled Rust binary.

#### AUR checklist

- `pkgver` matches `Cargo.toml` version.
- `source` points to an immutable release tarball for stable package publishing.
- `sha256sums` is real for published releases; `SKIP` is acceptable only for local draft/testing workflows.
- `depends`, `makedepends`, and optional dependency policy are explicit.
- `PKGBUILD` and `.SRCINFO` are updated together.
- Validate with `namcap`.
- Build in a clean chroot before publish.
- Ensure the package does not rely on undeclared transitive dependencies.

### 5.2 npm

#### Core fact

npm is JavaScript-first, but it can distribute native CLI binaries through wrapper packages and platform-specific artifacts.

#### Viable options

1. Single npm package with a `postinstall` download script.
2. Meta package plus platform packages.
3. Compile from source during install.

#### Recommendation

Use a meta package plus platform packages.

Package shape:

- `petiglyph` meta package
- `@petiglyph/petiglyph-linux-x64-gnu`
- `@petiglyph/petiglyph-linux-arm64-gnu`
- `@petiglyph/petiglyph-linux-x64-musl`
- `@petiglyph/petiglyph-linux-arm64-musl`
- `@petiglyph/petiglyph-darwin-x64`
- `@petiglyph/petiglyph-darwin-arm64`
- `@petiglyph/petiglyph-win32-x64-msvc`
- Optional phase 2: `@petiglyph/petiglyph-win32-arm64-msvc`

Mechanics:

- Use `optionalDependencies` in the meta package.
- Use `os`, `cpu`, and for Linux packages `libc` fields in platform packages.
- Use npm CPU names, not Rust arch names: `x64`, `arm64`.
- Use npm OS names: `linux`, `darwin`, `win32`.
- Use `libc` only for Linux packages. npm documents `libc` as Linux-only.
- Expose the command through `bin` in the meta package.
- The meta package should dispatch to the installed platform package and fail with a clear unsupported-platform message if no matching optional dependency installed.

Why this is best:

- Avoids unreliable source compilation during install.
- Avoids network-fetching install scripts.
- Aligns with common native npm distribution patterns used by tools like `esbuild`.
- Lets npm skip incompatible optional platform packages rather than failing the whole install.

npm-specific caveats:

- Do not copy `node_modules` across operating systems or libc families; reinstall on the destination platform.
- Verify Windows shim behavior generated from `bin` (`.cmd` and PowerShell shims).
- Keep package names and versions synchronized across meta and platform packages.
- If using npm trusted publishing, current npm docs require npm CLI 11.5.1+ and Node 22.14.0+ in supported hosted CI environments.
- Trusted publishing from GitHub Actions/GitLab automatically creates npm provenance attestations; self-hosted runners are not currently supported by npm trusted publishing.

### 5.3 PyPI

#### Viable options

1. Publish a Python wrapper that downloads a binary during install or first run.
2. Publish Rust binary wheels using `maturin` `bin` bindings.
3. Publish a Python extension module through `pyo3` and embed a CLI.

#### Recommendation

Use `maturin` `bin` wheels plus an sdist.

Reason:

- `maturin` explicitly supports Rust binaries as Python packages.
- `pip install petiglyph` can place a command into the environment's script path without requiring a Python extension API.
- Wheels give a fast install path; the sdist provides a source fallback for unsupported platforms.
- This keeps `petiglyph` as a Rust CLI/TUI product instead of forcing a Python API shape.

Minimum PyPI files to add:

- `pyproject.toml` with `build-backend = "maturin"`.
- `[tool.maturin] bindings = "bin"`.
- PEP 621 metadata in `[project]` or metadata inherited from Cargo where appropriate.
- Classifiers that accurately describe supported OSes only after wheels exist.

Linux wheel policy choices:

1. manylinux only.
2. musllinux only.
3. both manylinux and musllinux.

Recommendation:

- Publish manylinux wheels first for glibc Linux.
- Add musllinux wheels once musl artifacts pass smoke tests.
- Publish an sdist, but clearly document that source installs require a Rust toolchain and any native dependency prerequisites.

PyPI-specific caveats:

- Build Linux wheels in compliant manylinux/musllinux environments or with tooling such as maturin's supported zig path.
- Use `--compatibility pypi` or equivalent CI checks so unsupported wheel tags are rejected before upload.
- Keep the wheel matrix realistic at first; expand after smoke coverage is stable.
- PyPI trusted publishing should use the PyPA publishing action rather than long-lived API tokens.

---

## 6. Runtime Compatibility Plan

### 6.1 CLI compatibility requirements

1. Automation commands must run without a TTY.
2. TUI launch must fail clearly when no terminal is attached.
3. JSON contract must remain stable across platforms.
4. Errors must not mention platform-specific repair tools unless those tools apply.
5. Paths emitted in JSON must be valid native paths for the current OS.

Current status:

- Non-TTY TUI failure behavior is already implemented.
- JSON envelope behavior is covered in CLI contract tests.

### 6.2 TUI compatibility requirements

1. Raw mode and alternate screen behavior on Linux/macOS/Windows terminals.
2. Key handling parity for arrows, Enter, Esc, Tab, plus/minus, and paste paths.
3. Graceful degradation when keyboard enhancement flags are unsupported.
4. Unicode private-use rendering guidance per terminal.
5. Terminal too-small behavior must work consistently.
6. Clipboard behavior must not silently claim success when no provider exists.

Current status:

- `crossterm` feature toggles are used, and advanced flags are already best-effort.
- The clipboard helper needs platform-aware providers and status reporting.

### 6.3 Unicode and terminal rendering notes

Current project default:

- `codepoint_start = "U+100000"`

This is in Supplementary Private Use Area-B. Unicode's private-use ranges are:

- `U+E000..U+F8FF`
- `U+F0000..U+FFFFD`
- `U+100000..U+10FFFD`

Compatibility implications:

- Private-use codepoints only render correctly when the selected terminal font resolves to the petiglyph-generated font.
- Some terminals treat East Asian Ambiguous width differently; petiglyph samples assume single-cell ambiguous width.
- Composite grid glyphs assume tightly stacked terminal cells; custom line-height/cell-height/font-offset settings can create apparent seams unrelated to the generated font.
- Users may need to restart terminal processes after installing fonts because some terminals cache font discovery.

### 6.4 External runtime dependencies

`ffmpeg`:

- Required only for video input expansion.
- Not required for still images or GIF import through the Rust `image` crate.
- Keep explicit user-facing checks/errors.
- Decide per registry whether this is a hard dependency or optional capability.

Font tooling:

- Linux: `fc-cache` is required for the current refresh path; `fc-match` is useful for diagnostics.
- macOS: `atsutil` is used for cache refresh.
- Windows: PowerShell is used for font change broadcast.

Clipboard tooling:

- Linux Wayland: `wl-copy`.
- Linux X11: `xclip -selection clipboard`.
- macOS: `pbcopy`.
- Windows: prefer PowerShell `Set-Clipboard`; `clip.exe` is an acceptable fallback but may have encoding/newline quirks.

### 6.5 Immediate runtime hardening items

1. Replace hardcoded `/tmp/petiglyph-tui-debug.log` with `std::env::temp_dir().join("petiglyph-tui-debug.log")` or an overrideable debug log path.
2. Add OS-specific clipboard provider chain and return success/failure to the TUI status message.
3. Improve Windows/macOS diagnostics where Linux fontconfig tools are unavailable.
4. Keep `ffmpeg` checks explicit and user-facing.
5. Add macOS and Windows CI smoke checks for `petiglyph --help`, non-TTY TUI failure, `create --no-launch`, and `build` on a fixture project.

---

## 7. Build and Release Architecture

### 7.1 CI matrix

Use a GitHub Actions matrix by target triple with artifact upload:

- Linux GNU builds on Ubuntu runners.
- Linux musl builds with `cross`, musl toolchains, or maturin/zig where appropriate for PyPI.
- macOS builds on macOS runners.
- Windows MSVC builds on Windows runners.

Minimum verification per host OS:

- `cargo test --locked` where feasible.
- `petiglyph --help`.
- `petiglyph tui` in non-TTY context must fail with the documented terminal-required error.
- Build a fixture project with still images.
- If `ffmpeg` is installed, run one video import smoke; otherwise verify the error is explicit.

### 7.2 Artifact naming convention

Use consistent names for direct downloads and downstream packaging:

- `petiglyph-v{version}-{target}.{ext}`

Examples:

- `petiglyph-v0.0.1-x86_64-unknown-linux-gnu.tar.gz`
- `petiglyph-v0.0.1-aarch64-apple-darwin.tar.gz`
- `petiglyph-v0.0.1-x86_64-pc-windows-msvc.zip`

Archive contents:

- `petiglyph` or `petiglyph.exe`.
- `README.md`.
- `LICENSE`.
- Optional `completions/` only after shell completions are generated.

### 7.3 Shared release source of truth

Recommendation:

- Build release binaries once per target in CI.
- Reuse those artifacts for direct GitHub releases and npm platform packages where practical.
- Build PyPI wheels through maturin because wheel metadata and tagging are package-format-specific.
- Publish `SHA256SUMS` and, ideally, signed checksums.
- Generate GitHub artifact attestations for release artifacts.

### 7.4 Supply chain security controls

1. Use GitHub protected environments for publishing jobs.
2. Use OIDC trusted publishing for npm where supported.
3. Use OIDC trusted publishing for PyPI.
4. Generate provenance/attestations for release artifacts.
5. Publish checksums with releases.
6. Keep release jobs pinned to immutable action versions where feasible.
7. Ensure npm package `repository` metadata exactly matches the public repository when publishing provenance.

### 7.5 macOS and Windows trust UX

Decision options:

1. Unsigned binaries only.
2. Sign later.
3. Sign from the first public release.

Recommendation:

- Developer previews can be unsigned if the release notes are explicit.
- Public first-class macOS support should include Developer ID signing and notarization.
- Public first-class Windows support should include Authenticode signing.

Important Windows correction:

- Current Microsoft guidance says EV certificates no longer bypass SmartScreen by themselves. Signing still identifies the publisher and helps reputation, but every new file hash may need reputation to accumulate. Do not promise a no-warning Windows experience solely because a binary is signed.

---

## 8. Feature and Dependency Policy Decisions

### 8.1 `image` crate feature policy

Current risk:

- `avif-native` increases native dependency and cross-build risk.

Decision options:

1. Keep `avif-native` always on.
2. Make AVIF support an optional Cargo feature.
3. Remove AVIF temporarily for broadest portability.

Recommendation:

- Move AVIF native support behind an explicit Cargo feature.
- Keep default release builds dependency-light: PNG, JPEG, GIF, BMP, WebP, and SVG through existing `resvg` path if validated.
- Add an `avif` feature only after CI covers the native dependency path for the release targets.

### 8.2 External runtime dependency policy

Decision: classify `ffmpeg` as either hard dependency or optional capability per channel.

Recommendation:

- Treat `ffmpeg` as an optional capability in application behavior.
- Keep explicit runtime checks and good errors.
- For AUR, prefer `optdepends=('ffmpeg: video import support')` if minimizing install footprint matters.
- If the product promise is that animated/video creation works immediately after package install, keep `ffmpeg` in `depends` instead.
- Whichever policy is selected, apply it consistently in `PKGBUILD`, `scripts/aur.sh`, README, npm docs, and PyPI docs.

### 8.3 Font install policy

Recommendation:

- Keep installs per-user by default.
- Do not write to system font directories.
- Keep project-scoped font identities and registry ownership guardrails.
- Keep `doctor --repair` as the recovery path for stale metadata, lock files, and registry conflicts.

Reason:

- Per-user installs avoid privilege prompts and reduce risk on Linux/macOS/Windows.
- Project-scoped identities avoid collisions between petiglyph projects.

---

## 9. Test Strategy for Cross Compatibility

### 9.1 Test layers

1. Unit tests with `cargo test --locked`.
2. CLI contract tests per OS.
3. Install/uninstall tests per OS, with isolated fake HOME/USERPROFILE/LOCALAPPDATA where possible.
4. TUI smoke tests per OS.
5. Packaging/install tests for AUR, npm, and PyPI.
6. Direct release artifact smoke tests.

### 9.2 TUI smoke scenarios per OS

Minimum smoke suite:

1. Launch and quit cleanly in a real terminal/PTY.
2. Non-TTY launch fails cleanly.
3. Navigate Home/Glyphs panels.
4. Modify a threshold and verify manifest update.
5. Build from sample manifest.
6. Install font command path where safe in an isolated user profile.
7. Render sample output and record terminal family/version where possible.

### 9.3 hty-specific guidance

Current repo harness:

- `scripts/tui_e2e_hty.sh`

Policy:

- Prefer extending this harness instead of introducing `expect` or another PTY framework.
- Validate local `hty --help` and subcommand behavior before assuming flags.
- Favor waits on rendered state and filesystem artifacts over fixed sleeps.
- Keep visible debug flows (`hty watch`, `hty attach`, script `--watch`) useful for manual cross-terminal diagnosis.

### 9.4 Packaging smoke scenarios

AUR:

1. Build in a clean chroot.
2. Install package.
3. Run `petiglyph --help`.
4. Run `petiglyph doctor`.
5. Run `petiglyph build` in a fixture project.
6. If dependency policy says video is included, verify video import support.

npm:

1. `npm pack` meta and platform packages.
2. Install from local tarballs on each platform.
3. Run `petiglyph --help`.
4. Build in a fixture project.
5. Verify unsupported platform failure message by simulating missing platform package.

PyPI:

1. Build wheels and sdist.
2. Install wheel in a clean venv.
3. Run `petiglyph --help`.
4. Build in a fixture project.
5. Install sdist in a toolchain-equipped environment to verify source fallback.

### 9.5 Regression gates before release

Require:

- Successful build artifacts for all first-class targets.
- CLI contract pass on Linux/macOS/Windows.
- At least one TUI smoke run per OS family.
- Artifact checksum generation.
- Registry package dry-run or staging publish success.
- AUR clean-chroot build success before AUR publish.
- npm package install test on Linux, macOS, and Windows.
- PyPI wheel install test on Linux, macOS, and Windows.

---

## 10. Recommended Decisions

1. Registry delivery model:

- AUR: source `PKGBUILD` package.
- npm: meta package plus platform-specific optional dependency packages.
- PyPI: `maturin` `bin` wheels plus sdist.

2. Linux binary coverage:

- Ship GNU and musl, but make GNU the default documented Linux artifact.

3. AVIF native dependency policy:

- Make native AVIF support optional, not default-critical.

4. Runtime dependencies:

- Treat `ffmpeg` as an optional application capability unless the package channel explicitly chooses full video support by default.

5. Security/provenance:

- Use trusted publishing for npm/PyPI where supported.
- Publish checksums and GitHub artifact attestations.
- Plan macOS notarization and Windows signing for public first-class releases.

6. TUI portability hardening:

- Remove hardcoded `/tmp` production path.
- Add macOS/Windows clipboard providers.
- Make clipboard success/failure visible in TUI state.

7. Packaging consistency:

- Fix `scripts/aur.sh` dependency mismatch immediately after deciding whether `ffmpeg` is `depends` or `optdepends`.

---

## 11. Phased Implementation Plan

### Phase 0: Immediate documentation and repo fixes (1-2 days)

1. Decide `ffmpeg` dependency policy.
2. Update `PKGBUILD` and `scripts/aur.sh` to match that policy.
3. Replace hardcoded TUI debug path with a platform-agnostic temp path.
4. Add platform-aware clipboard providers.
5. Add README compatibility section for per-OS dependencies, terminal notes, and font-cache behavior.
6. Add CI smoke tests for non-TTY TUI failure on Linux/macOS/Windows.

### Phase 1: Build matrix and artifacts (2-4 days)

1. Add GitHub Actions matrix for first-class targets.
2. Produce release archives with normalized names.
3. Generate `SHA256SUMS`.
4. Add direct artifact smoke tests.
5. Add musl build jobs and mark musl support experimental until smoke-tested.

### Phase 2: Registry packaging (3-6 days)

npm:

1. Create meta and platform package layout.
2. Wire `optionalDependencies`, `os`, `cpu`, `libc`, and `bin`.
3. Add local `npm pack` install tests.

PyPI:

1. Add `pyproject.toml` and maturin config.
2. Build manylinux/macOS/Windows wheels first.
3. Add musllinux wheels after musl validation.
4. Publish to TestPyPI before PyPI.

AUR:

1. Generate `.SRCINFO` in release workflow.
2. Use immutable release tarballs and real checksums.
3. Add clean-chroot validation step.

### Phase 3: Trust and release quality (2-4 days)

1. Configure npm trusted publishing, accounting for npm/Node version requirements.
2. Configure PyPI trusted publishing.
3. Enable GitHub artifact attestations.
4. Add macOS signing and notarization workflow.
5. Add Windows Authenticode signing workflow.
6. Document SmartScreen expectations accurately.

---

## 12. Open Decisions to Finalize

1. Official architecture list for the first stable public release.
2. Whether `ffmpeg` is a hard dependency or optional capability in each channel.
3. Whether AVIF native support is default-on or feature-gated.
4. Timeline for macOS notarization and Windows code signing.
5. Whether npm/PyPI should publish only latest versions or maintain an N-1 compatibility/support matrix.
6. Whether Windows ARM64 is included at launch or phase 2.
7. Whether macOS should ship separate arch archives only or also a universal2 archive.
8. Whether TUI E2E on macOS/Windows should use `hty`, a platform-native PTY strategy, or a reduced smoke suite initially.

---

## 13. Source Links

### Repository references

- `Cargo.toml`
- `PKGBUILD`
- `scripts/aur.sh`
- `src/install.rs`
- `src/tui.rs`
- `src/animation_media.rs`
- `tests/cli_contract.rs`
- `README.md`

### External primary references

Rust and cross-compilation:

- [Rust platform support](https://doc.rust-lang.org/rustc/platform-support.html)
- [Rust target tier policy](https://doc.rust-lang.org/rustc/target-tier-policy.html)
- [Windows MSVC targets in rustc](https://doc.rust-lang.org/beta/rustc/platform-support/windows-msvc.html)
- [rustup cross-compilation](https://rust-lang.github.io/rustup/cross-compilation.html)
- [cross-rs](https://github.com/cross-rs/cross)

TUI / terminal backend:

- [crossterm crate docs](https://docs.rs/crossterm/latest/crossterm/)
- [ratatui backend concepts](https://ratatui.rs/concepts/backends/)
- [Windows console virtual terminal sequences](https://learn.microsoft.com/windows/console/console-virtual-terminal-sequences)

Fonts and Unicode:

- [Apple Font Book: install and validate fonts](https://support.apple.com/guide/font-book/install-and-validate-fonts-fntbk1000/mac)
- [Unicode FAQ: private-use areas](https://www.unicode.org/faq/basic_q.html)
- [Microsoft AddFontResourceEx reference](https://learn.microsoft.com/windows/win32/api/wingdi/nf-wingdi-addfontresourceexa)
- [fontconfig user documentation](https://www.freedesktop.org/software/fontconfig/fontconfig-user.html)

AUR / Arch packaging:

- [PKGBUILD](https://wiki.archlinux.org/title/PKGBUILD)
- [AUR submission guidelines](https://wiki.archlinux.org/title/AUR_submission_guidelines)
- [Creating packages](https://wiki.archlinux.org/title/Creating_packages)
- [Building in a clean chroot](https://wiki.archlinux.org/title/DeveloperWiki:Building_in_a_clean_chroot)
- [Makepkg](https://wiki.archlinux.org/title/Makepkg)

npm packaging and trust:

- [npm package.json fields](https://docs.npmjs.com/cli/v11/configuring-npm/package-json)
- [npm folders and bin behavior](https://docs.npmjs.com/cli/v11/configuring-npm/folders)
- [npm trusted publishers](https://docs.npmjs.com/trusted-publishers)
- [npm provenance statements](https://docs.npmjs.com/generating-provenance-statements)
- [esbuild package distribution reference](https://esbuild.github.io/getting-started/)

PyPI / Python packaging:

- [maturin distribution guide](https://www.maturin.rs/distribution.html)
- [maturin project README](https://github.com/PyO3/maturin)
- [Python wheel binary distribution format](https://packaging.python.org/specifications/binary-distribution-format/)
- [Platform compatibility tags](https://packaging.python.org/specifications/platform-compatibility-tags/)
- [PEP 600: manylinux platform tags](https://peps.python.org/pep-0600/)
- [PEP 656: musllinux platform tags](https://peps.python.org/pep-0656/)
- [PyPI trusted publishers](https://docs.pypi.org/trusted-publishers/)

Signing / provenance:

- [Apple notarizing macOS software](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution)
- [Microsoft SmartScreen reputation for app developers](https://learn.microsoft.com/windows/apps/package-and-deploy/smartscreen-reputation)
- [Microsoft code signing options](https://learn.microsoft.com/windows/apps/package-and-deploy/code-signing-options)
- [GitHub artifact attestations](https://docs.github.com/actions/concepts/security/artifact-attestations)
