# RELEASE-CHECKLIST

Purpose: executable checklist for shipping `petiglyph` through GitHub Releases, AUR, npm, and PyPI/TestPyPI.

This checklist reflects the repository state as of 2026-05-31. Use `Cargo.toml` package version as the source of truth.

## 0. Current Release Surface

Direct GitHub archives build 8 targets:

- `x86_64-unknown-linux-gnu`
- `aarch64-unknown-linux-gnu`
- `x86_64-unknown-linux-musl`
- `aarch64-unknown-linux-musl`
- `x86_64-apple-darwin`
- `aarch64-apple-darwin`
- `x86_64-pc-windows-msvc`
- `aarch64-pc-windows-msvc`

Release runner labels (explicitly pinned to avoid `*-latest` drift in release pipelines):

- Linux targets: `ubuntu-latest`
- `x86_64-apple-darwin`: `macos-15-intel`
- `aarch64-apple-darwin`: `macos-15`
- Windows targets: `windows-2025`

npm publishes the `petiglyph` meta package plus 8 optional native `@petiglyph/*` packages matching the direct archive targets.

PyPI/TestPyPI publishes Linux GNU manylinux, macOS, and Windows wheels plus an sdist. It does not currently publish musllinux wheels.

AUR is manual: prepare `PKGBUILD`/`.SRCINFO`, validate locally, then push to the AUR package repository.

Runtime dependency policy:

- `ffmpeg` is required for video/animated media import workflows.
- Arch packaging declares `depends=('ffmpeg')`.
- Other package channels do not bundle `ffmpeg`; docs and runtime prompt must make that explicit.

Trust/signing policy:

- GitHub release assets include checksums and artifact attestations.
- npm and PyPI use trusted publishing through protected GitHub environments.
- macOS and Windows artifacts are unsigned unless a release explicitly says otherwise.
- macOS and Windows ARM64 artifacts should be marked limited-runtime-validation/unstable until directly runtime-tested.

## 1. Version Sync

1. Choose the release version, for example `0.1.0`.
2. Sync repo versions:

```bash
./scripts/release_sync_versions.sh 0.1.0
```

3. Verify synchronized metadata:

```bash
cargo metadata --no-deps --format-version 1 | jq -r '.packages[] | select(.name == "petiglyph") | .version'
rg -n '^(pkgver=|\s*"version":|version = )' Cargo.toml PKGBUILD npm/*/package.json
rg -n '"@petiglyph/petiglyph-[^"]+": "0\.1\.0"' npm/petiglyph/package.json
```

4. Confirm README JSON envelope sample version matches the release version.

## 2. Local Quality Gates

Run from repo root:

```bash
cargo fmt --check
cargo check --locked
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked
cargo run -- --help
cargo run -- tui </dev/null
```

Toolchain policy: use the repo-pinned stable toolchain from `rust-toolchain.toml` (with `clippy` and `rustfmt` components).

Expected:

- Formatting/check/clippy/test pass.
- `cargo run -- tui </dev/null` fails cleanly with the terminal-required error.

Smoke a scratch project before release:

```bash
rm -rf /tmp/petiglyph-release-smoke
mkdir -p /tmp/petiglyph-release-smoke
cp -R icons /tmp/petiglyph-release-smoke/icons
cat > /tmp/petiglyph-release-smoke/petiglyph.toml <<'MANIFEST'
input_dir = "icons"
out_dir = "build"
font_name = "Petiglyph Release Smoke"
glyph_size = 64
threshold = 64
codepoint_start = "U+100000"
MANIFEST

cargo run -- build --manifest /tmp/petiglyph-release-smoke/petiglyph.toml --json
cargo run -- sample --manifest /tmp/petiglyph-release-smoke/petiglyph.toml --json
cargo run -- install-font --manifest /tmp/petiglyph-release-smoke/petiglyph.toml --json
cargo run -- uninstall-font --manifest /tmp/petiglyph-release-smoke/petiglyph.toml --json
cargo run -- doctor --manifest /tmp/petiglyph-release-smoke/petiglyph.toml --json
```

## 3. Runtime/Clipboard Smoke

Linux/macOS:

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

If `ffmpeg` is installed, also smoke one animated media import manually through the TUI. If not installed, verify the missing-`ffmpeg` prompt/error is explicit.

## 4. TUI E2E

Install and validate `hty` first:

```bash
curl -fsSL https://raw.githubusercontent.com/LatentEvals/hty/main/scripts/install.sh | sh
export PATH="$HOME/.local/bin:$PATH"
hty --help
```

Run the headless harness:

```bash
./scripts/tui_e2e_hty.sh
```

Run the new creation-workflow popup journeys directly when debugging:

```bash
./scripts/tui_e2e_hty.sh --journey 8,9,10
```

Optional visible debug run:

```bash
PETIGLYPH_TUI_DEBUG=1 ./scripts/tui_e2e_hty.sh --watch --step-delay-ms 250
```

Before changing the harness, check local and upstream `hty` behavior:

```bash
hty --help
hty help run
hty help send
hty help wait
```

References:

- https://github.com/LatentEvals/hty
- https://hty.sh

## 5. GitHub Release Artifacts

Before tagging:

- [ ] Working tree is clean and release-prep changes are committed.
- [ ] Release package hygiene passes after release-prep changes are committed:

```bash
./scripts/release_assert_clean_tree.sh
```

- [ ] `README.md`, `AGENTS.md`, `CROSS-COMPATIBILITY-GUIDE.md`, and release docs match current command/package behavior.
- [ ] Release notes include unsigned macOS/Windows warnings where applicable.
- [ ] Release notes identify limited-runtime-validation targets.

Tag and push:

```bash
./scripts/release_assert_clean_tree.sh
git tag -s v0.1.0 -m "petiglyph v0.1.0"
git push origin v0.1.0
```

The tag push triggers `.github/workflows/release.yml`.

After workflow completion:

```bash
gh release view v0.1.0 --json assets
gh release download v0.1.0 -D ./dist-release
(cd dist-release && sha256sum -c SHA256SUMS)
gh release verify v0.1.0
```

Validate at least one extracted archive locally:

```bash
mkdir -p /tmp/petiglyph-asset-smoke
cd /tmp/petiglyph-asset-smoke
tar -xzf /path/to/dist-release/petiglyph-v0.1.0-x86_64-unknown-linux-gnu.tar.gz
./petiglyph-v0.1.0-x86_64-unknown-linux-gnu/petiglyph --help
./petiglyph-v0.1.0-x86_64-unknown-linux-gnu/petiglyph doctor --json
./petiglyph-v0.1.0-x86_64-unknown-linux-gnu/petiglyph tui </dev/null
```

## 6. AUR

Prepare release package metadata from the signed tag tarball:

```bash
./scripts/release_prepare_aur.sh 0.1.0
```

Validate locally on Arch:

```bash
makepkg -sf
namcap PKGBUILD
namcap petiglyph-*.pkg.tar.zst
```

Publish manually from the AUR package repository:

```bash
cp /path/to/petiglyph/PKGBUILD .
cp /path/to/petiglyph/.SRCINFO .
git add PKGBUILD .SRCINFO
git commit -m "petiglyph 0.1.0"
git push
```

For local development packaging, use:

```bash
./scripts/aur.sh
./scripts/aur.sh build
./scripts/aur.sh install
./scripts/aur.sh uninstall
```

## 7. npm

The GitHub Release `published` event triggers `.github/workflows/npm-publish.yml`.

Workflow order:

1. Check out the release tag.
2. Verify release integrity with `gh release verify` and `SHA256SUMS`.
3. Verify tag/version consistency.
4. Stage binaries into `npm/*/bin` with `scripts/release_stage_npm_artifacts.sh`.
5. Run `npm pack --dry-run` for each package.
6. Publish platform packages.
7. Publish the `petiglyph` meta package.

Local preflight after downloading release assets:

```bash
./scripts/release_npm_pack_test.sh dist-release
```

Post-publish checks:

```bash
npm view petiglyph version
npm view petiglyph bin
npm install -g petiglyph
petiglyph --help
```

## 8. PyPI/TestPyPI

The GitHub Release `published` event triggers `.github/workflows/pypi-publish.yml`.

Workflow order:

1. Build wheels for Linux GNU manylinux, macOS, and Windows targets.
2. Build sdist.
3. Publish to TestPyPI through the `testpypi` environment.
4. Publish to PyPI through the `pypi` environment after TestPyPI succeeds.

Local preflight:

```bash
python -m pip install -U maturin twine
maturin build --release --compatibility pypi --sdist
twine check target/wheels/*
```

TestPyPI install check:

```bash
python -m venv .venv-testpypi
. .venv-testpypi/bin/activate
pip install --index-url https://test.pypi.org/simple/ --extra-index-url https://pypi.org/simple petiglyph
petiglyph --help
```

## 9. Manual Approval Gates

Keep required reviewers enabled for these GitHub environments:

- `npm`
- `testpypi`
- `pypi`

Do not publish the GitHub Release until direct release assets, checksums, notes, and known limitations have been reviewed. Publishing the GitHub Release starts npm and PyPI workflows.

## 10. Rollback Actions

- GitHub assets: ship a fixed patch release; avoid mutating published assets.
- AUR: update `PKGBUILD` and increment `pkgrel` if reusing the same upstream version.
- npm: publish a fixed patch and deprecate the broken version.
- PyPI: publish a fixed patch and yank the broken release if necessary.
