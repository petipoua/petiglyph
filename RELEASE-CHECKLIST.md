# RELEASE-CHECKLIST

Purpose: executable release checklist for shipping `petiglyph` across direct GitHub artifacts, AUR, npm, and PyPI.

This checklist assumes the compatibility direction in `CROSS-COMPATIBILITY-GUIDE.md`:

- Rust CLI/TUI binary remains the source product.
- AUR ships a source-built Arch package.
- npm ships a meta package plus platform-specific optional dependency packages.
- PyPI ships `maturin` `bin` wheels plus an sdist.
- Linux direct artifacts ship GNU and musl binaries, with GNU as the default documented Linux download.
- macOS and Windows artifacts ship unsigned for this release, with explicit release-note warnings.

---

## 0. Release Decisions (Locked)

### 0.1 Release scope

- First cross-platform release version is `0.1.0`.
- No formal stable/beta/preview labeling is used for releases.
- GitHub releases are the canonical source of binary artifacts for downstream packaging.

### 0.2 Supported targets

Mandatory targets for the first release:

- `x86_64-unknown-linux-gnu`
- `aarch64-unknown-linux-gnu`
- `x86_64-unknown-linux-musl`
- `aarch64-unknown-linux-musl`
- `x86_64-apple-darwin`
- `aarch64-apple-darwin`
- `x86_64-pc-windows-msvc`
- `aarch64-pc-windows-msvc`

macOS distribution policy:

- Ship separate architecture archives only.
- Do not produce universal2 archives.

Runtime support tier policy for this release:

- Runtime-validated targets: Linux (`x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`, `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`) and Windows x64 (`x86_64-pc-windows-msvc`).
- Compile-only/limited-runtime-validation targets: macOS (`x86_64-apple-darwin`, `aarch64-apple-darwin`) and Windows ARM64 (`aarch64-pc-windows-msvc`).
- Release notes and docs must mark compile-only/limited-runtime-validation targets as `unstable` until direct runtime testing coverage exists.

### 0.3 Runtime dependency policy

- `ffmpeg` is a hard dependency.
- Package docs must state that media processing depends on `ffmpeg` availability.

### 0.4 Image format policy

- AVIF support is required for release behavior.
- AVIF support may be satisfied through `ffmpeg`-based media processing paths.
- Release documentation must list supported formats and the role of `ffmpeg` in AVIF/media workflows.

### 0.5 Signing and trust policy

- macOS artifacts are published unsigned and without notarization.
- Windows artifacts are published unsigned.
- Release notes must explicitly state that unsigned binaries can trigger OS trust/security prompts.
- Release notes must explicitly mark macOS and Windows ARM64 artifacts as `unstable` until runtime validation exists.

### 0.6 Registry ownership

- npm naming policy uses the default model:
`petiglyph` meta package plus `@petiglyph/*` platform packages (if the scope is available).
- PyPI project name target is `petiglyph` (if available).
- AUR publish order is `petiglyph` first; `petiglyph-bin` and `petiglyph-git` can be added later if needed.

### 0.7 Support window

- npm/PyPI support policy is latest-only.
- No `N-1` maintenance target is planned.

---

## 1. Pre-Implementation Cleanup

### 1.1 Align dependency policy

- [ ] Apply hard `ffmpeg` dependency policy from section 0.3.
- [ ] Update `PKGBUILD` to use `depends=('ffmpeg')`.
- [ ] Update `scripts/aur.sh` to generate the same dependency policy.
- [ ] Update README compatibility notes to match.
- [ ] Update `CROSS-COMPATIBILITY-GUIDE.md` if the owner decision differs from its recommendation.

Acceptance:

```sh
rg -n "depends=|optdepends|ffmpeg" PKGBUILD scripts/aur.sh README.md CROSS-COMPATIBILITY-GUIDE.md
```

The dependency story should be consistent across all files, with `ffmpeg` declared as required.

### 1.2 Reduce native build risk

- [ ] Apply AVIF policy from section 0.4.
- [ ] Ensure release docs explain whether AVIF is handled by native image decode, `ffmpeg`, or both.
- [ ] Add or update README supported input format list.
- [ ] Confirm `cargo build --release --locked` works on Linux after any feature changes.

Acceptance:

```sh
cargo build --release --locked
cargo test --locked
```

### 1.3 Runtime portability fixes

- [ ] Replace production hardcoded `/tmp/petiglyph-tui-debug.log` with `std::env::temp_dir()` or an overrideable env var.
- [ ] Add clipboard providers:
  - Linux Wayland: `wl-copy`
  - Linux X11: `xclip -selection clipboard`
  - macOS: `pbcopy`
  - Windows: PowerShell `Set-Clipboard`, optionally `clip.exe` fallback
- [ ] Make clipboard copy status accurately report success/failure.
- [ ] Review Windows/macOS error messages so they do not refer to Linux-only fontconfig tools.

Acceptance:

```sh
cargo test --locked
cargo run -- --help
cargo run -- tui </dev/null
```

Expected: non-TTY TUI launch fails with the documented terminal-required error.

---

## 2. Versioning and Release Rules

### 2.1 Version source of truth

Reasonable default: `Cargo.toml` is the canonical version source.

- [ ] Set `package.version` in `Cargo.toml`.
- [ ] Keep `PKGBUILD pkgver` synchronized with `Cargo.toml`.
- [ ] Keep npm meta/platform package versions synchronized with `Cargo.toml`.
- [ ] Keep PyPI version synchronized through `maturin` metadata.

Acceptance:

```sh
cargo metadata --no-deps --format-version 1 | jq -r '.packages[] | select(.name == "petiglyph") | .version'
rg -n "^(pkgver|version)" PKGBUILD package*.json pyproject.toml 2>/dev/null || true
```

### 2.2 Git tagging

Reasonable default:

- Release tags use `v{version}`, for example `v0.1.0`.
- Do not publish registries from branch pushes.
- Publish only from protected tags and approved GitHub environments.

Checklist:

- [ ] Create release branch or merge all release changes to `main`.
- [ ] Confirm working tree is clean.
- [ ] Create annotated tag `v{version}`.
- [ ] Push tag.

Commands:

```sh
git status --short
git tag -a "vVERSION" -m "petiglyph vVERSION"
git push origin "vVERSION"
```

---

## 3. Direct GitHub Artifacts

### 3.1 Artifact matrix

Reasonable default first matrix:

| Target | Runner/build strategy | Archive |
| --- | --- | --- |
| `x86_64-unknown-linux-gnu` | Ubuntu native | `.tar.gz` |
| `aarch64-unknown-linux-gnu` | `cross` or Linux cross toolchain | `.tar.gz` |
| `x86_64-unknown-linux-musl` | `cross` or musl toolchain | `.tar.gz` |
| `aarch64-unknown-linux-musl` | `cross` or musl toolchain | `.tar.gz` |
| `x86_64-apple-darwin` | macOS runner | `.tar.gz` |
| `aarch64-apple-darwin` | macOS runner | `.tar.gz` |
| `x86_64-pc-windows-msvc` | Windows runner | `.zip` |
| `aarch64-pc-windows-msvc` | Windows runner or cross build with ARM64 validation | `.zip` |

Checklist:

- [ ] Add GitHub Actions build workflow.
- [ ] Install Rust stable with target triple.
- [ ] Build with `cargo build --release --locked --target {target}`.
- [ ] Strip binaries where safe and available.
- [ ] Archive binary with README and LICENSE.
- [ ] Name artifacts as `petiglyph-v{version}-{target}.{tar.gz|zip}`.
- [ ] Generate `SHA256SUMS`.
- [ ] Upload artifacts to GitHub release.
- [ ] Generate GitHub artifact attestations if enabled.

### 3.2 Direct artifact smoke tests

For each produced artifact:

- [ ] Extract archive.
- [ ] Run `petiglyph --help`.
- [ ] Run `petiglyph doctor --json`.
- [ ] Run `petiglyph tui` without TTY and assert clean failure.
- [ ] Build a fixture project.
- [ ] If safe on that runner, run install/uninstall in an isolated user profile.
- [ ] For compile-only targets (macOS, Windows ARM64), if runtime tests are not executed, mark release notes/docs with `unstable` for those target artifacts.

Example Linux/macOS commands:

```sh
./petiglyph --help
./petiglyph doctor --json
./petiglyph tui </dev/null
```

Example Windows commands:

```powershell
.\petiglyph.exe --help
.\petiglyph.exe doctor --json
```

---

## 4. AUR Release

### 4.1 Package policy

Reasonable default:

- Stable package name: `petiglyph`.
- Source-based build from immutable GitHub release tarball.
- `makedepends=('cargo')`.
- Runtime dependency uses hard requirement `depends=('ffmpeg')`.
- `arch=('x86_64')` initially.

Checklist:

- [ ] Update `PKGBUILD pkgver`.
- [ ] Set `source` to immutable GitHub release tarball URL.
- [ ] Replace `sha256sums=('SKIP')` with real checksum before publish.
- [ ] Ensure `depends=('ffmpeg')` is present and no contradictory `optdepends` policy is documented.
- [ ] Confirm `package()` installs binary to `/usr/bin/petiglyph`.
- [ ] Confirm README/license docs install to appropriate `/usr/share/doc` and `/usr/share/licenses` paths.
- [ ] Generate `.SRCINFO`.
- [ ] Build with `makepkg` locally.
- [ ] Validate with `namcap`.
- [ ] Build in clean chroot.
- [ ] Publish to AUR.

Commands:

```sh
makepkg --printsrcinfo > .SRCINFO
makepkg -sf
namcap PKGBUILD
namcap petiglyph-*.pkg.tar.zst
```

Clean chroot command depends on local Arch devtools setup. Recommended default:

```sh
pkgctl build
```

Fallback for local-only testing:

```sh
extra-x86_64-build
```

### 4.2 AUR acceptance criteria

- [ ] Clean chroot build succeeds.
- [ ] Installed package exposes `petiglyph` on PATH.
- [ ] `petiglyph --help` succeeds.
- [ ] `petiglyph doctor` succeeds or reports actionable non-fatal issues.
- [ ] `petiglyph build` works in a fixture project.
- [ ] Video/media workflows function with `ffmpeg` installed from package dependencies.
- [ ] `.SRCINFO` matches `PKGBUILD`.

---

## 5. npm Release

### 5.1 Package layout

Reasonable default:

```text
npm/
  petiglyph/
    package.json
    bin/petiglyph.js
  petiglyph-linux-x64-gnu/
    package.json
    bin/petiglyph
  petiglyph-linux-arm64-gnu/
    package.json
    bin/petiglyph
  petiglyph-linux-x64-musl/
    package.json
    bin/petiglyph
  petiglyph-linux-arm64-musl/
    package.json
    bin/petiglyph
  petiglyph-darwin-x64/
    package.json
    bin/petiglyph
  petiglyph-darwin-arm64/
    package.json
    bin/petiglyph
  petiglyph-win32-x64-msvc/
    package.json
    bin/petiglyph.exe
  petiglyph-win32-arm64-msvc/
    package.json
    bin/petiglyph.exe
```

If the `@petiglyph` scope is available, platform packages should be scoped:

- `@petiglyph/petiglyph-linux-x64-gnu`
- `@petiglyph/petiglyph-linux-arm64-gnu`
- `@petiglyph/petiglyph-linux-x64-musl`
- `@petiglyph/petiglyph-linux-arm64-musl`
- `@petiglyph/petiglyph-darwin-x64`
- `@petiglyph/petiglyph-darwin-arm64`
- `@petiglyph/petiglyph-win32-x64-msvc`
- `@petiglyph/petiglyph-win32-arm64-msvc`

### 5.2 Meta package requirements

- [ ] `name` matches owner decision in section 0.6.
- [ ] `version` matches `Cargo.toml`.
- [ ] `bin` exposes `petiglyph`.
- [ ] `optionalDependencies` references all platform packages at the exact same version.
- [ ] Launcher resolves current platform package from `process.platform`, `process.arch`, and Linux libc detection.
- [ ] Launcher forwards args and stdio to native binary.
- [ ] Launcher exits with the native process status code.
- [ ] Launcher prints a clear unsupported platform message when no platform package is installed.
- [ ] `repository` metadata exactly matches the public repo for provenance.
- [ ] `license`, `description`, and `homepage` are set.

### 5.3 Platform package requirements

For each platform package:

- [ ] `name` matches package naming policy.
- [ ] `version` matches meta package.
- [ ] `os` is set: `linux`, `darwin`, or `win32`.
- [ ] `cpu` is set: `x64` or `arm64`.
- [ ] Linux packages set `libc` to `glibc` or `musl`.
- [ ] Non-Linux packages do not set `libc`.
- [ ] `files` includes only binary, README/license, and minimal metadata.
- [ ] Binary has executable mode on Unix packages.
- [ ] Windows package contains `petiglyph.exe`.

### 5.4 npm local tests

For each package:

```sh
npm pack --dry-run
npm pack
```

Install test from packed tarballs:

```sh
tmpdir="$(mktemp -d)"
cd "$tmpdir"
npm init -y
npm install /path/to/npm/petiglyph/*.tgz /path/to/npm/platform-package/*.tgz
npx petiglyph --help
```

Windows equivalent should be run in PowerShell on a Windows runner.

Acceptance:

- [ ] `npm pack --dry-run` includes only expected files.
- [ ] Local tarball install succeeds on Linux GNU.
- [ ] Local tarball install succeeds on Linux musl if supported in CI.
- [ ] Local tarball install succeeds on macOS x64/arm64 where runners are available.
- [ ] Local tarball install succeeds on Windows x64.
- [ ] Local tarball install succeeds on Windows ARM64 where runners are available.
- [ ] `npx petiglyph --help` succeeds.
- [ ] `npx petiglyph build` succeeds in a fixture project.

### 5.5 npm publish

Reasonable default:

- Use npm trusted publishing from GitHub-hosted runners.
- Use Node 22.14.0+ and npm 11.5.1+.
- Publish platform packages first, then meta package.
- Use protected GitHub environment approval.

Checklist:

- [ ] Reserve/verify package names.
- [ ] Configure npm trusted publisher for each package.
- [ ] Publish to npm from tag workflow.
- [ ] Verify provenance is present for published packages when supported.
- [ ] Install from public npm registry on Linux/macOS/Windows.

Commands after publish:

```sh
npm view petiglyph version
npm view petiglyph bin
npm i -g petiglyph
petiglyph --help
```

Use scoped name if owner chooses a scoped meta package.

---

## 6. PyPI Release

### 6.1 `pyproject.toml` baseline

Reasonable default:

```toml
[build-system]
requires = ["maturin>=1.0,<2.0"]
build-backend = "maturin"

[project]
name = "petiglyph"
description = "TUI-first Rust CLI for turning image folders into monochrome glyph fonts."
readme = "README.md"
requires-python = ">=3.8"
license = { text = "MIT" }
keywords = ["font", "glyph", "icons", "tui", "cli"]
classifiers = [
  "Programming Language :: Rust",
  "Environment :: Console",
  "License :: OSI Approved :: MIT License",
  "Operating System :: POSIX :: Linux",
  "Operating System :: MacOS",
  "Operating System :: Microsoft :: Windows",
]

[project.urls]
Homepage = "https://github.com/petipoua/petiglyph"
Repository = "https://github.com/petipoua/petiglyph"
Issues = "https://github.com/petipoua/petiglyph/issues"

[tool.maturin]
bindings = "bin"
strip = true
compatibility = "pypi"
```

Checklist:

- [ ] Add `pyproject.toml`.
- [ ] Confirm project name availability.
- [ ] Confirm version is inherited correctly from Cargo or explicitly set once.
- [ ] Confirm `maturin build --release` produces a wheel containing the `petiglyph` command.
- [ ] Confirm sdist contains all files required for source build.

### 6.2 Wheel matrix

Reasonable default first matrix:

- Linux x86_64 manylinux
- Linux aarch64 manylinux
- macOS x86_64
- macOS aarch64
- Windows x86_64
- Windows ARM64 (if wheel pipeline supports it)

Add musllinux after musl smoke tests pass:

- Linux x86_64 musllinux
- Linux aarch64 musllinux

Checklist:

- [ ] Build manylinux wheels in compliant containers or maturin action.
- [ ] Build macOS wheels on macOS runners.
- [ ] Build Windows wheels on Windows runners.
- [ ] Build sdist.
- [ ] Run `twine check` or equivalent metadata validation.
- [ ] Install each wheel in a clean venv and run `petiglyph --help`.
- [ ] Install sdist in a Rust-toolchain environment and run `petiglyph --help`.
- [ ] If Windows ARM64 wheel is not produced, validate `pip install` via sdist fallback on a Windows ARM64 environment.

Example local commands:

```sh
python -m pip install maturin twine
maturin build --release --compatibility pypi --sdist
twine check target/wheels/*
```

### 6.3 TestPyPI and PyPI publish

Reasonable default:

- Publish to TestPyPI before PyPI until at least one successful full release has shipped.
- Use PyPI trusted publishing from GitHub Actions.
- Use protected GitHub environment approval.

Checklist:

- [ ] Configure Trusted Publisher on TestPyPI.
- [ ] Publish wheels/sdist to TestPyPI.
- [ ] Install from TestPyPI in a clean venv.
- [ ] Configure Trusted Publisher on PyPI.
- [ ] Publish wheels/sdist to PyPI.
- [ ] Install from PyPI in a clean venv.

Commands:

```sh
python -m venv .venv-petiglyph-release-test
. .venv-petiglyph-release-test/bin/activate
pip install --index-url https://test.pypi.org/simple/ --extra-index-url https://pypi.org/simple petiglyph
petiglyph --help
```

After real publish:

```sh
python -m venv .venv-petiglyph-pypi-test
. .venv-petiglyph-pypi-test/bin/activate
pip install petiglyph
petiglyph --help
```

Use Windows PowerShell venv activation on Windows runners.

---

## 7. Cross-Channel Acceptance Criteria

A release is valid only when all selected channels pass.

### 7.1 Core behavior

- [ ] `petiglyph --help` succeeds from direct artifact.
- [ ] `petiglyph --help` succeeds from AUR package.
- [ ] `petiglyph --help` succeeds from npm install.
- [ ] `petiglyph --help` succeeds from PyPI install.
- [ ] `petiglyph tui` fails clearly in non-TTY contexts.
- [ ] `petiglyph build` succeeds in a fixture project.
- [ ] `petiglyph doctor --json` emits valid JSON.
- [ ] Runtime validation is required for Linux and Windows x64.
- [ ] macOS and Windows ARM64 may ship compile-tested only, but must be tagged `unstable` in docs/release notes.

### 7.2 Install/font behavior

- [ ] Linux install writes only under isolated test HOME font paths.
- [ ] macOS install writes only under isolated/test user font paths where possible.
- [ ] Windows install writes only under isolated/test LOCALAPPDATA where possible.
- [ ] `install-font` and `uninstall-font` are idempotent.
- [ ] `doctor --repair` can repair expected stale metadata cases.

### 7.3 Registry behavior

- [ ] npm package selects correct native binary on each supported OS/arch/libc.
- [ ] npm package fails clearly on unsupported OS/arch/libc.
- [ ] PyPI wheel installs without requiring Rust on supported wheel platforms.
- [ ] PyPI sdist builds with Rust toolchain installed.
- [ ] AUR package builds in clean chroot.

### 7.4 Security/provenance

- [ ] GitHub release artifacts have checksums.
- [ ] GitHub artifact attestations are generated if enabled.
- [ ] npm provenance appears for packages where trusted publishing supports it.
- [ ] PyPI publish uses trusted publishing, not long-lived API tokens.
- [ ] Release notes clearly document unsigned macOS and Windows artifacts.
- [ ] Release notes clearly document `unstable` status for macOS and Windows ARM64 artifacts until runtime-tested.

---

## 8. Rollback and Failure Handling

### 8.1 Direct GitHub artifacts

If direct artifact validation fails before public announcement:

- [ ] Delete failed draft release or mark as pre-release with failure note.
- [ ] Delete/move bad artifacts if they were not publicly consumed.
- [ ] Create a new patch version if artifacts were already public.

### 8.2 AUR

If AUR package is bad:

- [ ] Push corrected `PKGBUILD` with incremented `pkgrel` if upstream version is unchanged.
- [ ] Regenerate `.SRCINFO`.
- [ ] Add comment explaining fix if users may have installed the bad package.

### 8.3 npm

If npm package is bad:

- [ ] Prefer publishing a fixed patch version.
- [ ] Do not rely on unpublish except for immediate, policy-compliant accidents.
- [ ] Deprecate bad version with a clear message if needed.

Command:

```sh
npm deprecate petiglyph@BAD_VERSION "Broken release; upgrade to FIXED_VERSION"
```

### 8.4 PyPI

If PyPI package is bad:

- [ ] Publish a fixed patch version.
- [ ] Yank the bad version if appropriate.
- [ ] Do not delete files as the normal rollback strategy.

Command:

```sh
# Use PyPI UI or trusted workflow/tooling to yank with a reason.
```

---

## 9. First Implementation Work Items

Recommended order:

1. Use section 0 as locked policy for this release.
2. Fix `ffmpeg` policy mismatch between `PKGBUILD` and `scripts/aur.sh`.
3. Align AVIF implementation and docs with section 0.4 policy.
4. Fix hardcoded `/tmp` runtime path.
5. Add platform-aware clipboard providers.
6. Add GitHub direct artifact build workflow.
7. Add AUR release hardening.
8. Add PyPI `pyproject.toml` and wheel build workflow.
9. Add npm package layout and local package tests.
10. Add trusted publishing and provenance only after local/staging package installs pass.
