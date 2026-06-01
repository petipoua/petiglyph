# petiglyph Release Readiness TODO

This document is an implementation queue for getting `petiglyph` ready to publish through GitHub Releases, npm, PyPI, and AUR while preserving the dual TUI/CLI product contract.

Use it as a step-by-step handoff for future agents. Do not try to clear the whole file in one pass. Pick one section, implement it, run the listed validation, and update the status before moving on.

## Current Inspection Snapshot

Local checks run from `/home/petipoua/Code/petiglyph`:

- `cargo fmt --check`: passed.
- `cargo check --locked`: passed.
- `cargo test --locked`: passed, 165 in-crate tests plus 23 CLI contract tests.
- `cargo clippy --locked --all-targets --all-features -- -D warnings`: failed on current Rust 1.96.0 with lint errors across `src/build.rs`, `src/cli.rs`, `src/doctor.rs`, `src/glyph_debug.rs`, `src/install.rs`, `src/tui.rs`, and `src/tests.rs`.
- `hty --help`: failed because `hty` is not installed locally.
- `makepkg --printsrcinfo | diff -u .SRCINFO -`: passed.
- `uvx maturin sdist --out /tmp/petiglyph-sdist-check`: built an sdist but warned that `project.version` is missing from `pyproject.toml`.
- `uvx twine check /tmp/petiglyph-sdist-check/*`: passed.
- `uvx maturin build --release --compatibility pypi --out /tmp/petiglyph-wheel-check`: built a local Linux wheel but failed PyPI validation because it produced `linux_x86_64`, which PyPI rejects. The CI manylinux action may still be fine, but local release docs currently imply this command should work.
- `npm view petiglyph version`: 404, name appears unpublished.
- `npm view @petiglyph/petiglyph-linux-x64-gnu version`: 404, scoped package appears unpublished.
- `https://pypi.org/pypi/petiglyph/json`: 404, name appears unpublished.
- AUR RPC for `petiglyph`: `resultcount: 0`, package appears unpublished.
- `gh repo view petipoua/petiglyph`: repo exists but is currently private; no releases or tags were found.

Important local workspace state:

- Initial inspection found `test-1/` as untracked scratch output, not ignored by `.gitignore`, and auto-detected as a project from the repo root. Section 1 now ignores `test-*` scratch projects.
- Initial inspection found `test_parse`, `test_parse.rs`, `test_ws`, and `test_ws.rs` tracked even though the repo instructions classify those names as scratch artifacts rather than canonical source. Section 1 removes them from tracked source going forward.
- Initial inspection found `cargo package --list --allow-dirty` and the generated maturin sdist including `test-1/`, `test_parse`, and `test_ws`. Section 1 now adds a guard to prevent that regression.

## P0 - Release Blockers

### 1. Clean Source Tree And Package Contents

Status: implemented. The full guard intentionally fails until this cleanup is committed because it requires `git status --short` to be empty; the package-content checks no longer report scratch paths.

Original observation:

- `test-1/` is untracked scratch output but not ignored.
- `test_parse`, `test_ws`, and their `.rs` files are tracked scratch artifacts.
- Source distribution tooling currently packages scratch files in dirty-tree scenarios. AUR local tarball generation also includes tracked and untracked non-ignored files.

Tasks:

- [x] Remove tracked scratch artifacts from git history going forward: `test_parse`, `test_parse.rs`, `test_ws`, `test_ws.rs`.
- [x] Add explicit `.gitignore` entries for `test-*`, `test_parse*`, `test_ws*`, and other known local scratch project names that are not already ignored.
- [x] Add a packaging guard script, for example `scripts/release_assert_clean_tree.sh`, that fails if:
  - `git status --short` is non-empty,
  - `cargo package --list --allow-dirty` includes root scratch paths,
  - `npm/*/bin` contains staged native binaries before release staging, except `.gitkeep`,
  - local package artifacts are present outside ignored paths.
- [x] Call that guard from release-prep docs and from relevant release workflows before publishing.

Validation:

```bash
git status --short
cargo package --list --allow-dirty | rg '^(test-|test_parse|test_ws)'
uvx maturin sdist --out /tmp/petiglyph-sdist-check
tar -tzf /tmp/petiglyph-sdist-check/petiglyph-*.tar.gz | rg '(^|/)(test-|test_parse|test_ws)'
```

Expected result: no scratch paths appear.

### 2. Fix Clippy Or Make The Lint Policy Explicit

Status: implemented. Added `rust-toolchain.toml` (stable + `clippy` + `rustfmt`), fixed non-TUI clippy errors, and kept TUI behavior stable with behavior-preserving refactors plus a TUI-local lint allow list for remaining style-only lints.

Observation:

- The release checklist asks for `cargo clippy --all-targets --all-features -- -D warnings`.
- That command currently fails on Rust 1.96.0.
- Most errors are mechanical style lints, but several are in `src/tui.rs`. The TUI guardrail still applies: preserve behavior and avoid UX/text/layout changes unless intentionally requested.

Tasks:

- [x] Decide and document the Rust toolchain policy first: either add `rust-toolchain.toml` and an MSRV, or explicitly target current stable.
- [x] Fix non-TUI clippy failures first in focused patches.
- [x] Fix TUI clippy failures only as behavior-preserving refactors. Do not change keybindings, labels, layout, copy, or flow while doing this.
- [x] If a lint is intentionally undesirable for this codebase, add a narrow `allow` with a short reason rather than broad disabling.

Validation:

```bash
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked
```

### 3. Add Real CI Before Any Release Workflow Is Trusted

Status: implemented in-repo via `.github/workflows/ci.yml` with a small-project policy: require only core CI checks to pass before merge; keep additional branch rules minimal.

Observation:

- `.github/workflows/` currently has release and publish workflows only.
- There is no normal PR/push CI workflow that gates `main`.
- The release workflow builds artifacts but does not run `cargo fmt`, `cargo clippy`, or `cargo test` before publishing release assets.

Tasks:

- [x] Add `.github/workflows/ci.yml` for PRs and pushes to `main`.
- [x] Run at minimum:
  - `cargo fmt --check`
  - `cargo check --locked`
  - `cargo clippy --locked --all-targets --all-features -- -D warnings`
  - `cargo test --locked`
  - `cargo package --list --allow-dirty` plus the package hygiene guard from section 1
- [x] Run CI on Linux, macOS, and Windows for native targets.
- [x] Do not run `scripts/tui_e2e_hty.sh` in required CI for now; keep TUI E2E as local/manual or future non-blocking CI until timing stability is proven on hosted runners.
- [x] Require only core CI status checks to pass before merge (small-team baseline): `rust-quality-ubuntu-latest`, `rust-quality-macos-latest`, `rust-quality-windows-latest`, `package-hygiene-linux`.

Validation:

- Open a PR and verify all four required checks pass before merge.
- Confirm branch protection requires only the four core CI checks above.

### 4. Replace Stale Or Ambiguous GitHub Runner Labels

Status: implemented for release/publish workflows. Intel macOS now uses `macos-15-intel`, Apple Silicon macOS uses `macos-15`, and Windows release/publish jobs use `windows-2025`. ARM-native runtime smoke jobs remain a future enhancement.

Observation:

- `release.yml` and `pypi-publish.yml` use `macos-13` for `x86_64-apple-darwin`.
- Current GitHub-hosted runner docs list Intel macOS labels such as `macos-15-intel` and `macos-26-intel`, and arm64 labels such as `macos-latest`, `macos-14`, `macos-15`, and `macos-26`.
- `windows-latest` and `macos-latest` can drift over time.

Tasks:

- [x] Replace `macos-13` with an explicit supported Intel macOS runner label.
- [x] Consider replacing `windows-latest` with an explicit Windows label such as `windows-2025` after confirming toolchain support.
- [ ] Consider using native `windows-11-arm` and `ubuntu-24.04-arm` jobs for arm64 runtime smoke checks when the repo is public or account limits allow it.
- [x] Update `CROSS-COMPATIBILITY-GUIDE.md` and `RELEASE-CHECKLIST.md` to match actual runner labels.

Validation:

```bash
gh workflow run ci.yml
gh workflow run release.yml --ref <test-tag-or-branch-if-workflow_dispatch-is-added>
```

## P1 - Test Coverage And Runtime Confidence

### 5. Make The TUI E2E Harness A First-Class Release Gate

Status: validated locally after the hty E2E refactor. `hty --help` passes, `./scripts/tui_e2e_hty.sh` passes all 10 journeys, and `./scripts/tui_e2e_hty.sh --journey 8` passes.

Observation:

- `scripts/tui_e2e_hty.sh` defines 7 useful process-level journeys.
- `hty` is not installed locally, and no CI workflow installs or runs it.
- The harness does not yet cover the full Home creation workflow popup paths for glyph, grid, animated glyph, and animated grid imports.

Tasks:

- [x] Check upstream `hty` docs and local CLI behavior before editing the harness.
- [x] Add a documented installer step for `hty` in CI and local docs.
- [x] Run the current 7 journeys in headless CI mode on Linux.
- [x] Add at least one journey for the "Creation Workflow In Progress" popup that imports a glyph and reaches the Glyphs panel.
- [x] Add a grid popup journey that imports one image, configures rows/cols/bleed, and verifies manifest composition output.
- [x] Add an animated popup journey using a small GIF fixture. Keep video/FFmpeg testing as a separate slower smoke unless CI installs `ffmpeg`.
- [x] Ensure CI sets up `ffmpeg` or explicitly disables/satisfies the first-run FFmpeg prompt so it cannot steal TUI focus.

Validation:

```bash
hty --help
./scripts/tui_e2e_hty.sh
./scripts/tui_e2e_hty.sh --journey <new-journey-id>
```

### 6. Broaden CLI Contract Tests For Missing Edges

Status: implemented. Added edge-case coverage for hidden uninstall guidance, non-TTY TUI invocation paths, FFmpeg prompt suppression/state behavior in JSON and non-interactive flows, unsupported import paths across glyph/grid/animation create commands, `doctor --repair --json` stale-lock repair, and non-interactive `create` auto-launch skip behavior.

Observation:

- `tests/cli_contract.rs` covers many JSON command paths and important install lifecycle behavior.
- Gaps remain around hidden/ambiguous commands, first-run prompts, non-JSON behavior, and some error paths.

Tasks:

- [x] Add tests for hidden `petiglyph uninstall` returning guidance and non-zero exit.
- [x] Add tests for `petiglyph tui </dev/null` with and without `--manifest`.
- [x] Add tests that no JSON command ever triggers the FFmpeg first-run prompt.
- [x] Add tests for FFmpeg prompt state behavior using a temporary `HOME` and fake `PATH`.
- [x] Add tests for unsupported import files in `glyph create`, `grid create`, and `animation create-*`.
- [x] Add tests for `doctor --repair --json` against a project with stale lock entries.
- [x] Add tests for `create <name>` in non-interactive mode without `--no-launch`, verifying it skips TUI launch cleanly.

Validation:

```bash
cargo test --locked --test cli_contract
```

### 7. Add Cross-Platform Runtime Smoke Checks

Status: partially implemented. Added cross-platform runtime smoke CI (`runtime-smoke-${os}` in `.github/workflows/ci.yml`) and test env isolation for `HOME`/`USERPROFILE`/`LOCALAPPDATA` in `tests/cli_contract.rs`, plus macOS/Windows install lifecycle tests gated by target OS. Remaining manual OS-deep validation is still pending.

Observation:

- Unit tests simulate several cross-OS clipboard/provider cases.
- Actual runtime validation has only been performed on local Linux in this inspection.
- Font installation paths are OS-specific and use external commands:
  - Linux: `fc-cache`
  - macOS: `atsutil`
  - Windows: PowerShell `WM_FONTCHANGE` broadcast

Tasks:

- [x] Add a CI job that runs `petiglyph --help`, `petiglyph doctor --json`, and `petiglyph tui </dev/null` on Linux, macOS, and Windows.
- [x] Use an isolated temporary `HOME`/`USERPROFILE`/`LOCALAPPDATA` for install/uninstall tests so developer state does not affect results.
- [ ] Validate Windows per-user font installation on a real Windows runner or VM. If copying to `%LOCALAPPDATA%/Microsoft/Windows/Fonts/petiglyph/` plus `WM_FONTCHANGE` is insufficient, add HKCU font registry registration and uninstall cleanup.
- [ ] Validate macOS detects fonts from `~/Library/Fonts/petiglyph/` after `atsutil databases -removeUser`; document whether terminal restart is required.
- [ ] Validate Linux install on a minimal image where `fontconfig` may be absent.

Validation:

```bash
./scripts/clipboard_smoke.sh --bin ./target/release/petiglyph
pwsh -File .\scripts\clipboard_smoke.ps1 -PetiglyphPath .\target\release\petiglyph.exe
```

### 8. Add Dependency, License, And Supply-Chain Checks

Status: implemented. Added `deny.toml` policy, added CI `supply-chain-linux` checks in `.github/workflows/ci.yml` (`cargo deny`, `cargo audit`, `cargo tree`), and documented current dependency/cross-build-sensitive areas in `docs/dependency-supply-chain.md`.

Observation:

- Workflows pin GitHub Action refs to full SHAs, which is good.
- There is no dependency audit or license policy gate.
- AVIF support pulls in heavier codec dependencies such as `dav1d`/`rav1e`, which can be cross-build sensitive.

Tasks:

- [x] Add `cargo audit` or equivalent advisory scanning.
- [x] Add `cargo deny` with explicit license and duplicate-version policy.
- [x] Verify all bundled sample assets under `icons/` are redistributable.
- [x] Run `cargo tree -e normal` and document native/cross-build-sensitive dependencies.
- [x] If AVIF causes cross-build instability, decide whether AVIF should be optional behind a feature or remain in the default release build.

Validation:

```bash
cargo deny check
cargo audit
cargo tree --locked -e normal
```

## P1 - Packaging And Publishing Channels

### 9. GitHub Release Flow

Status: partially implemented while private. Release artifacts now support manual dispatch, publish as a draft release by default, run per-target archive smoke checks before upload, keep `SHA256SUMS` plus attestations, and prefill release notes from `docs/release-notes-template.md`. Public-release prerequisites remain pending.

Observation:

- The repo is currently private.
- `release.yml` now creates a draft GitHub Release on tag push (or manual dispatch), which keeps downstream publish workflows gated until a manual publish action.
- Publishing the release fires npm and PyPI workflows via `release.published`.
- Artifact unpack smoke tests now run per target archive before upload.

Tasks:

- [ ] Make the repository public before release if npm provenance, AUR source URLs, and public GitHub release distribution are required.
- [x] Add a manual preflight workflow or change release artifact creation to a draft release so assets can be inspected before `release.published` triggers npm/PyPI.
- [x] Add per-target archive smoke checks:
  - unpack archive,
  - run `petiglyph --help`,
  - run `petiglyph doctor --json` in an isolated temp home/workspace,
  - run `petiglyph tui </dev/null` and verify terminal-required failure.
- [x] Keep artifact attestations and `SHA256SUMS`.
- [x] Add release notes from `docs/release-notes-template.md` before public publish.

Validation:

```bash
gh release list --repo petipoua/petiglyph
gh release verify <tag>
(cd dist-release && sha256sum -c SHA256SUMS)
```

### 10. npm Package Flow

Observation:

- `npm/petiglyph` is a meta package with 8 optional native packages.
- Platform packages use `os`, `cpu`, and Linux `libc`.
- `npm/petiglyph/bin/petiglyph.js` dispatches by `process.platform`, `process.arch`, and Linux libc detection.
- npm names currently appear unpublished, but the `@petiglyph` scope still needs to be created/owned.
- npm trusted publishing now requires explicit allowed actions for new trusted publisher configurations.

Tasks:

- [ ] Create/claim the npm `petiglyph` package name and the `@petiglyph` org/scope.
- [ ] Configure trusted publishers for all 9 npm packages:
  - owner/repo: `petipoua/petiglyph`
  - workflow filename: `npm-publish.yml`
  - environment: `npm`
  - allowed action: `npm publish` or staged publish if adopted
- [ ] Add unit tests for `npm/petiglyph/bin/petiglyph.js` platform resolution, including Linux unknown-libc failure.
- [ ] Extend `scripts/release_npm_pack_test.sh` to inspect each packed tarball for exactly the expected files and executable mode.
- [ ] Add a local install smoke for the meta package plus each platform package where it can run natively.
- [ ] Consider npm staged publishing to reduce risk of partial multi-package publication.
- [ ] Ensure publish failure handling is documented because npm versions are immutable after publish.

Validation:

```bash
node -c npm/petiglyph/bin/petiglyph.js
./scripts/release_npm_pack_test.sh dist-release
npm pack --dry-run
npm view petiglyph version
```

### 11. PyPI/TestPyPI Flow

Status: partially implemented. `pyproject.toml` now declares `dynamic = ["version"]`, local `uvx maturin sdist` + `uvx twine check` pass without the previous missing-version warning, release docs now route host-local validation through sdist checks while treating CI manylinux wheels as release-valid, and `.github/workflows/pypi-publish.yml` now runs `twine check dist/*` before both TestPyPI and PyPI uploads.

Observation:

- `pyproject.toml` now uses `dynamic = ["version"]` so maturin can source version metadata from Cargo.
- Local `maturin build --compatibility pypi` on Linux produced a `linux_x86_64` wheel and failed PyPI validation. The GitHub workflow uses maturin's manylinux mode, so the CI path may still work.
- PyPI package name currently appears unpublished.
- The workflow publishes to TestPyPI and then PyPI, but there is no install smoke between them.

Tasks:

- [x] Fix `pyproject.toml` metadata by adding `dynamic = ["version"]` if maturin should source the version from Cargo, or by synchronizing an explicit `version`.
- [x] Update release docs so local Linux PyPI validation uses the correct manylinux path, container, or CI workflow instead of a command that fails on a plain host.
- [ ] Configure pending trusted publishers for both TestPyPI and PyPI:
  - project: `petiglyph`
  - owner/repo: `petipoua/petiglyph`
  - workflow: `pypi-publish.yml`
  - environments: `testpypi` and `pypi`
- [x] Add `twine check dist/*` in the PyPI workflow before upload.
- [ ] After TestPyPI publish, create a fresh venv, install from TestPyPI, and run `petiglyph --help`, `petiglyph doctor --json`, and the non-TTY TUI guard before allowing PyPI publish.
- [ ] Decide whether musllinux wheels are intentionally absent, and document that limitation in package metadata and release notes.

Validation:

```bash
uvx maturin sdist --out /tmp/petiglyph-sdist-check
uvx twine check /tmp/petiglyph-sdist-check/*
python -m pip install --index-url https://test.pypi.org/simple/ petiglyph
petiglyph --help
```

### 12. AUR Flow

Observation:

- `PKGBUILD` and `.SRCINFO` are currently in sync.
- `scripts/release_prepare_aur.sh` computes a real SHA256 from the GitHub tag tarball.
- Runtime dependency now includes `fontconfig` alongside `ffmpeg`, matching Linux install behavior that shells out to `fc-cache`.
- The repo is private, so the GitHub source URL in a release-grade `PKGBUILD` will not be usable by AUR users until the repo is public.
- AUR package name currently appears unpublished.

Tasks:

- [ ] Make the GitHub repo public before AUR publication.
- [x] Add `fontconfig` to `depends` if Linux install/sample workflows require `fc-cache` to succeed.
- [x] Decide whether `arch=('x86_64')` is enough or whether Arch Linux ARM/aarch64 should be documented separately. Current decision: keep AUR package metadata at `x86_64` for initial release and document ARM as a separate manual path until validated.
- [ ] Run `scripts/release_prepare_aur.sh <version>` only after the matching GitHub tag exists.
- [ ] Build in a clean Arch environment and install/uninstall the package.
- [ ] Create and push the AUR repo with at least `PKGBUILD` and `.SRCINFO`.

Validation:

```bash
./scripts/release_prepare_aur.sh 0.1.0
makepkg -sf
makepkg --printsrcinfo | diff -u .SRCINFO -
```

## P2 - Documentation And Product Polish Before Public Announcement

### 13. Align README With Actual Publication State

Observation:

- `README.md` currently says the distribution channels exist, but npm/PyPI/AUR are not yet published.

Tasks:

- [ ] Before first publication, either mark channels as "planned" or avoid public claims that packages are installable.
- [ ] After publication, add concrete install commands:
  - GitHub archive download example
  - `npm install -g petiglyph`
  - `pipx install petiglyph` or `python -m pip install petiglyph`
  - AUR helper/manual clone example
- [ ] Add a support matrix with build targets and validation level:
  - built
  - unit-tested
  - runtime-smoked
  - manual font-install verified
- [ ] Document `ffmpeg` as an external runtime dependency and clarify that non-Arch packages do not bundle it.
- [ ] Document terminal/font caveats for supplementary private-use codepoints.

Validation:

```bash
rg -n 'npm install|pipx|AUR|GitHub Releases|ffmpeg|support matrix' README.md RELEASE-GUIDE.md RELEASE-CHECKLIST.md
```

### 14. Add Public-Repo Hygiene Files

Observation:

- `LICENSE` exists.
- This is a small solo/low-maintainer project; hygiene should be lightweight and low-maintenance, not enterprise/LTS-heavy.
- Minimal public-repo basics are still useful so outside users know how to report bugs, where to send security issues, and what local checks to run before opening changes.

Tasks:

- [ ] Add a short `SECURITY.md` with:
  - private reporting path (email or GitHub private advisory),
  - latest-version-only support policy,
  - best-effort response timeline.
- [ ] Add a short `CONTRIBUTING.md` with:
  - local validation commands (`fmt`, `clippy -D warnings`, `test`),
  - reminder of the TUI change guardrail from `AGENTS.md`,
  - small PR expectations (tests/docs when behavior changes).
- [ ] Add one lightweight bug-report issue template (`.github/ISSUE_TEMPLATE/bug_report.yml`) with fields for expected/actual behavior, reproduction steps, environment, and logs.
- [ ] Keep changelog process minimal: use GitHub-generated release notes plus `docs/release-notes-template.md` (do not require maintaining a full manual `CHANGELOG.md` for now).

Validation:

```bash
test -f SECURITY.md
test -f CONTRIBUTING.md
test -f .github/ISSUE_TEMPLATE/bug_report.yml
rg -n 'release notes template|GitHub-generated release notes|latest.*version|best-effort|cargo fmt --check|cargo clippy --locked --all-targets --all-features -- -D warnings|cargo test --locked' SECURITY.md CONTRIBUTING.md TODO.md
```

### 15. Decide How Aggressive The FFmpeg Auto-Install Prompt Should Be

Observation:

- The CLI now defaults to "show command only" when `ffmpeg` is missing.
- Automatic package-manager command execution is opt-in via `--ffmpeg-auto-install`.
- Global prompt suppression is available via `PETIGLYPH_NO_FFMPEG_PROMPT=1`.
- It is suppressed for JSON and non-TTY paths.
- This behavior should be very clear before public release because package managers and privilege prompts are sensitive UX.

Tasks:

- [x] Add tests for prompt suppression and state persistence.
- [x] Add an explicit env var to disable the prompt globally: `PETIGLYPH_NO_FFMPEG_PROMPT=1`.
- [x] Change the prompt to "show command only" unless the user passes an opt-in flag (`--ffmpeg-auto-install`).
- [x] Ensure docs explain that `petiglyph` never runs package-manager commands in JSON or non-interactive contexts.

Validation:

```bash
cargo test --locked ffmpeg
```

## Final Release Rehearsal

Do this only after all P0 items and the relevant P1 packaging tasks are complete.

- [ ] Start from a clean clone.
- [ ] Confirm no scratch files:

```bash
git status --short
if cargo package --list --allow-dirty | rg '^(test-|test_parse|test_ws)'; then
  exit 1
fi
```

- [ ] Run local gates:

```bash
cargo fmt --check
cargo check --locked
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked
```

- [ ] Run TUI E2E:

```bash
hty --help
./scripts/tui_e2e_hty.sh
```

- [ ] Run packaging dry runs:

```bash
uvx maturin sdist --out /tmp/petiglyph-sdist-check
uvx twine check /tmp/petiglyph-sdist-check/*
./scripts/release_sync_versions.sh
makepkg --printsrcinfo | diff -u .SRCINFO -
```

- [ ] Push a release-candidate tag to a fork or temporary repo and verify GitHub artifacts before using the real package names.
- [ ] Confirm package names are still available immediately before publish:

```bash
npm view petiglyph version
npm view @petiglyph/petiglyph-linux-x64-gnu version
curl -fsS https://pypi.org/pypi/petiglyph/json
curl -fsS 'https://aur.archlinux.org/rpc/?v=5&type=info&arg[]=petiglyph'
```

Expected result before first publish: npm/PyPI return 404 and AUR returns `resultcount: 0`.

## References To Recheck During Implementation

- GitHub hosted runner labels: https://docs.github.com/en/actions/reference/runners/github-hosted-runners
- npm trusted publishing: https://docs.npmjs.com/trusted-publishers/
- npm `package.json` platform fields: https://docs.npmjs.com/cli/v11/configuring-npm/package-json/
- PyPI trusted publishers: https://docs.pypi.org/trusted-publishers/
- PyPI pending trusted publishers: https://docs.pypi.org/trusted-publishers/creating-a-project-through-oidc/
- AUR submission guidelines: https://wiki.archlinux.org/title/AUR_submission_guidelines
- hty docs: https://hty.sh and https://github.com/LatentEvals/hty
