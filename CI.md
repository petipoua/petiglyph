# CI Checks

Commit-time CI is defined in `.github/workflows/ci.yml`.

## Triggers

- `pull_request`
- `push` to `main`
- manual `workflow_dispatch`

Concurrency is enabled (`ci-${workflow}-${ref}`), so older in-progress runs on the same ref are cancelled.

## Jobs And Intent

## 1. `rust-quality` (Ubuntu, macOS, Windows)

Intent: keep core Rust quality bar consistent across major OS runners.

Runs:

- install FFmpeg
- `cargo fmt --check`
- `cargo clippy --locked --all-targets --all-features -- -D warnings`
- `cargo test --locked`

Cache guardrails:

- caches Cargo registry/git sources and `target/` with `actions/cache`
- key includes OS, workflow identity, `Cargo.lock` hash, and toolchain file hash (`rust-toolchain.toml` / `rust-toolchain` when present)
- cache restore/save is non-fatal; misses fall back to cold build automatically

## 2. `package-hygiene-linux` (Ubuntu)

Intent: ensure the package/release tree is clean and shippable.

Runs:

- `cargo package --list --allow-dirty`
- `./scripts/release_assert_clean_tree.sh`

## 3. `tui-e2e-hty-linux` (Ubuntu)

Intent: catch interactive TUI regressions with process-level journeys.

Runs:

- install FFmpeg
- install `hty` in CI
- `hty --help`
- `./scripts/tui_e2e_hty.sh --journey 1,2,3,4,5,6,7`
- `./scripts/tui_e2e_hty.sh --journey 8,9,10`

## 4. `runtime-smoke` (Ubuntu, macOS, Windows)

Intent: validate cross-platform runtime behavior in isolated home/config dirs.

Runs:

- install FFmpeg
- Linux/macOS: `./scripts/clipboard_smoke.sh --skip-clipboard-checks`
- Windows: `./scripts/clipboard_smoke.ps1 -SkipClipboardChecks`

## 5. `supply-chain-linux` (Ubuntu)

Intent: enforce dependency/security policy and export review artifact.

Runs:

- `cargo deny check`
- `cargo audit`
- `cargo tree --locked -e normal` (uploaded as artifact)

## Local Equivalent Commands

Run these locally before pushing:

```bash
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked
./scripts/release_assert_clean_tree.sh
./scripts/tui_e2e_hty.sh
./scripts/clipboard_smoke.sh --skip-clipboard-checks
cargo deny check
cargo audit
```

## CI Debugging Notes

### Manual cache maintenance

When CI performance or behavior suggests stale cache state use:

```bash
# list rust-quality caches for this repo
./scripts/gh_cache_delete.sh --prefix rust-quality --list

# delete matching rust-quality caches
./scripts/gh_cache_delete.sh --prefix rust-quality

# delete all caches only for main ref
./scripts/gh_cache_delete.sh --all --ref refs/heads/main
```

Notes:

- script requires authenticated `gh` CLI with `repo` scope
- use `--dry-run` first if you want to preview deletes

Use this section as operational memory when a CI failure is not immediately explained by the failing command output. Several commits after `bdf42556f0f0` through `9c75ea4e1a70` fixed runner-only failures that were not always reproducible locally.

General policy:

- Prefer CI-only/headless-only adaptations over weakening local E2E coverage or assertions.
- Reproduce the exact failing gate locally when possible, then check whether clean checkout, runner OS, headless execution, or package contents differ from local state.

TUI and headless runner behavior:

- For TUI E2E issues, verify `hty` behavior first (`hty --help`, `hty help run/send/wait`).
- Timing assertions around transient background-task UI state are flaky on shared runners.
  - Symptom: assertions expecting a short-lived “background task visible” state failed remotely.
  - Fix used: removed those timing-sensitive assertions and waited for task completion instead (`80a5ee9`).
- `hty` E2E transitions need explicit retry/wait logic on runners.
  - Symptom: transition from tweak step to “Create Animation” occasionally missed on CI PTY scheduling.
  - Fix used: added a retry helper and CI-only safer defaults for timeout and step delay when running under `GITHUB_ACTIONS` (`cc3c49a`).
- Clipboard availability is inconsistent on CI; tests must tolerate unavailable providers.
  - Symptom: copy-related TUI tests and status checks were brittle when no clipboard backend was usable.
  - Fixes used: accepted either successful copy notification or explicit clipboard failure status (`f2d4fe9`), then added `PETIGLYPH_MOCK_CLIPBOARD=1` for CI clipboard writes (`483c3fc`).
- Install-flow state transitions can race in CI.
  - Symptom: reinstall shortcut path and stale-output cleanup assertions intermittently failed due to focus/step variability.
  - Fix used: made the test flow more defensive while preserving functional assertions (`f2d4fe9`).

Platform-specific behavior:

- Windows FFmpeg install can fail transiently on Chocolatey feed timeouts.
  - Symptom: `Failed to fetch results from V2 feed ... 504 (Gateway Timeout)` followed by `ffmpeg: command not found`.
  - Current handling: CI and release smoke jobs now use `scripts/ci_install_ffmpeg.sh`, which retries `choco install ffmpeg` and falls back to `winget install Gyan.FFmpeg` if Chocolatey stays unavailable.
  - If this regresses again, inspect the `Install FFmpeg` step first before digging into downstream Rust/test failures; those are usually secondary fallout from the missing binary.
- Cross-platform lint/test imports must be gated precisely, not broadly.
  - Symptom: `#[cfg(unix)]` import logic caused issues in non-Linux CI contexts.
  - Fix used: narrowed permission import gates to `#[cfg(target_os = "linux")]` (`085a732`).
- Windows path parsing must preserve backslashes unless they escape a known token delimiter.
  - Symptom: shell/drop payload tokenization and unescape logic consumed backslashes in Windows paths, causing CI-only Windows failures.
  - Fix used: updated token split/unescape logic in `src/tui.rs` and `src/animation_media.rs` to only treat backslash as an escape before space, tab, quote, or backslash (`0f17634`, `7e747cb`).
- Windows smoke scripts should avoid PowerShell reserved names and ambiguous splatting semantics.
  - Symptom: using `Args` as a parameter name in PowerShell wrapper was error-prone.
  - Fix used: renamed it to `CliArgs` and updated call sites (`7e747cb`).
- System fallback fonts can pollute ownership scans on runner images.
  - Symptom: `LastResort`-style fallback fonts advertised broad coverage and could be misinterpreted as owned PUA occupancy.
  - Fix used: explicitly ignore non-ownership fallback fonts during external scan (`7e747cb`).

Packaging, fixtures, and release hygiene:

- For packaging drift, run `./scripts/release_assert_clean_tree.sh` and re-check version sync with `./scripts/release_sync_versions.sh`.
- AUR metadata is generator-driven, not hand-maintained.
  - Symptom: an AUR description or other metadata change appeared in `PKGBUILD` locally, then reverted or failed to appear on the AUR page after running the release helper.
  - Cause: `PKGBUILD` and `.SRCINFO` are regenerated from `scripts/lib/pkg_meta.sh`; editing only the generated files is not durable.
  - Fix used: update the canonical metadata in `scripts/lib/pkg_meta.sh`, then regenerate/publish with `./scripts/release_prepare_aur.sh` or `./scripts/release_publish_aur.sh` (`3512adc`).
- AUR packaging-only updates must bump `pkgrel`, not `pkgver`.
  - Symptom: an AUR-only fix such as description or packaging metadata changes was pushed without any visible version progression, or attempted to reuse the same `pkgver-pkgrel`.
  - Fix used: keep `pkgver` at the upstream release version and increment `pkgrel` (`0453dfa`, `12baab9`).
- AUR publication is a separate SSH-backed Git push, not a GitHub Actions publish step.
  - Symptom: release artifacts, npm, and PyPI were all green, but AUR was unchanged.
  - Cause: the AUR package is published from a separate `ssh://aur@aur.archlinux.org/petiglyph.git` repo and depends on local SSH configuration and key registration.
  - Fix used: publish with `./scripts/release_publish_aur.sh`, ensure `~/.ssh/config` points `aur.archlinux.org` at the intended key, and verify auth with `ssh aur@aur.archlinux.org help` before debugging packaging content.
- Generated release/download trees are working state, not source of truth.
  - Symptom: untracked directories such as `dist-release/` or extracted `src/petiglyph-*` trees created confusion during release cleanup or accidental diff review.
  - Fix used: treat them as disposable local state and remove them after the release/AUR flow; only commit intentional changes to canonical packaging files.
- Test fixtures must not match throwaway-project ignore rules.
  - Symptom: clean CI clones failed tests that referenced `test-assets/` because `/test-*/` ignored the fixture directory locally.
  - Fix used: explicitly unignore `/test-assets/` and commit the redistributable fixtures used by unit and integration-style tests.

Release artifact workflow:

- If the tag workflow fails while `main` CI is green, inspect `.github/workflows/release.yml` first. The release pipeline is not the same gate as `ci.yml`; it builds publishable artifacts, runs archive smoke checks, and then publishes a draft GitHub release.
- Registry publish workflows are separate from the main release build.
  - Symptom: the GitHub Release existed and release artifacts were green, but npm or PyPI/TestPyPI publishing still failed afterward.
  - Cause: `.github/workflows/npm-publish.yml` and `.github/workflows/pypi-publish.yml` run after a published GitHub Release and have their own auth, integrity, and environment-gate failure modes.
  - Fix used: debug those workflows directly rather than re-reading `ci.yml` or rebuilding release archives.
- Toolchain mismatches can hide behind the cross-build action.
  - Symptom: `actions-rust-cross` ran with a toolchain that did not match `rust-toolchain.toml`, so non-native targets were missing at build time.
  - Fix used: pinned the release build action to the repository toolchain (`1.88.0`) in `.github/workflows/release.yml` (`b5c2385`).
- Cross-compiled binaries must not be executed on incompatible runners.
  - Symptom: release smoke logic tried to run ARM-built artifacts on x86 runners.
  - Fix used: added per-matrix `smoke` flags and only ran archive smoke checks on host-compatible targets (`b5c2385`).
- `doctor` does not accept a `--workspace` flag.
  - Symptom: the release workflow passed `doctor --json --workspace ...`, which failed against the real CLI contract.
  - Fix used: changed smoke checks to `cd` into the temporary workspace and run `doctor --json` there (`b5c2385`).
- Release smoke environments still need FFmpeg.
  - Symptom: `doctor` was blocked before dispatch because the clean release runner did not have FFmpeg installed, while `ci.yml` did.
  - Fix used: installed FFmpeg in the runnable release smoke jobs before invoking the archive checks (`a5a6edd`).
- Windows ZIP packaging must preserve the top-level directory expected by the smoke check.
  - Symptom: `Compress-Archive -Path "$base/*"` flattened the archive layout, while the smoke check expected the same root folder structure used by Unix archives.
  - Fix used: changed Windows packaging to archive the directory itself (`Compress-Archive -Path $base -DestinationPath "$base.zip" -Force`) (`1e9a5e6`).
- A non-TTY `tui` failure is expected and should be asserted as such, not treated as a step failure.
  - Symptom: PowerShell propagated the expected non-interactive `tui` exit code as a failed smoke step.
  - Fix used: explicitly accept the terminal-required failure after validating the error text and exit the script successfully (`e5fc1a3`).
- npm release verification must validate the built archives, not rely on release-level attestations being present.
  - Symptom: `gh release verify vX.Y.Z` failed with `no attestations for tag ...` even though the release archives themselves were valid.
  - Cause: release-level attestation availability did not match the workflow assumption for an already-published release.
  - Fix used: download the published archives, verify each archive with `gh attestation verify`, then verify `SHA256SUMS` before publish (`1d33b13`).
- npm bootstrap publishing and long-term trusted publishing are distinct phases.
  - Symptom: the first npm publish failed even after package naming and workflow logic were corrected.
  - Cause: brand-new npm packages cannot use trusted publishing until the packages already exist and the trust relationship is configured per package.
  - Fix used: support a temporary `NPM_PUBLISH_TOKEN` bootstrap path in `.github/workflows/npm-publish.yml`, publish once, then configure trusted publishing for all package names and remove the token (`39e1394`, `191e30a`, `ac71674`).
- Manual reruns of the npm publish workflow must use the workflow file path and a stable checkout ref.
  - Symptom: `gh workflow run publish-npm ...` failed to find the workflow, or a manual rerun used the wrong checkout context for package metadata changes.
  - Fix used: dispatch `.github/workflows/npm-publish.yml` directly and make workflow-dispatch runs check out `main` while still validating the requested tag against `Cargo.toml` (`50a2310`).
- PyPI/TestPyPI trusted publishing is environment-specific.
  - Symptom: TestPyPI succeeded without a token, but the real PyPI publish still failed or remained pending.
  - Cause: TestPyPI and PyPI each need their own trusted publisher or token path; success on one registry does not imply success on the other.
  - Fix used: keep separate `testpypi` and `pypi` environments, and support separate bootstrap tokens only when trusted publishing is not configured (`ff24874`, `2d14f33`).
- `twine check` can fail on clean runners because the bundled metadata tooling is too old for current package metadata.
  - Symptom: validation rejected `license-file` or similar metadata even though the package itself was otherwise correct.
  - Fix used: upgrade `packaging` alongside `pip` and `twine` before running `twine check` in the publish workflow (`14964d5`).
- TestPyPI reruns for the same version must tolerate duplicate files.
  - Symptom: a rerun failed with `400 File already exists` from `https://test.pypi.org/legacy/`.
  - Cause: TestPyPI already had the exact wheel/sdist filenames from an earlier successful publish.
  - Fix used: publish to TestPyPI with `skip-existing: true` so the workflow can continue to the real PyPI gate on reruns (`71fd607`).

Dependency and supply-chain checks:

- For dependency/security failures, inspect the `cargo-tree-normal.txt` artifact and [DEPENDENCY_SUPPLY_CHAIN.md](DEPENDENCY_SUPPLY_CHAIN.md).
- Native codec dependencies can break hosted runners even when local builds succeed.
  - Symptom: transitive native dependency pressure from the `image` AVIF native stack (`dav1d`/`dav1d-sys`) increased CI fragility.
  - Fix used: removed `avif-native` from `image` features (`8c4e7bc`), keeping AVIF support without requiring that native dependency chain on CI.

CLI output hygiene:

- Commands that print to stdout can corrupt JSON-mode CLI output in CI checks.
  - Symptom: refresh commands writing to stdout leaked into JSON responses and broke parsers.
  - Fix used: switched refresh execution from `.status()` to `.output()`, suppressing successful output and only surfacing stdout/stderr on error (`9c75ea4`).
