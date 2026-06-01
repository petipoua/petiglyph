# CI Checks

This repository’s commit-time CI is defined in `.github/workflows/ci.yml`.

## When CI runs

- On every pull request (`pull_request` event).
- On pushes to `main` (`push` with `branches: [main]`).
- Manually via `workflow_dispatch`.

Notes:
- Jobs in the same workflow run in parallel.
- Concurrency is enabled (`ci-${workflow}-${ref}`), so older in-progress runs for the same ref are canceled when a newer commit arrives.

## Checks executed per CI run

### 1. `rust-quality` (Ubuntu, macOS, Windows)
Runs the Rust quality gate on all three OSes.

- **Show toolchain**: prints `rustc` and `cargo` versions to make build context explicit.
- **`cargo fmt --check`**: fails if formatting differs from `rustfmt` output.
- **`cargo check --locked`**: verifies the code compiles without building full artifacts and enforces lockfile consistency.
- **`cargo clippy --locked --all-targets --all-features -- -D warnings`**: runs lints across binaries/tests/examples/features; any warning is treated as an error.
- **`cargo test --locked`**: runs the test suite with lockfile enforcement.

### 2. `package-hygiene-linux` (Ubuntu)
Verifies what would be shipped and whether release packaging is clean.

- **`cargo package --list --allow-dirty`**: shows files that would go into the crate package.
- **`./scripts/release_assert_clean_tree.sh`**: enforces repository/package hygiene rules used for releases.

### 3. `tui-e2e-hty-linux` (Ubuntu)
Runs end-to-end TUI automation using `hty`.

- Installs `hty` from upstream install script.
- Validates CLI availability (`hty --help`).
- Runs core journeys: `./scripts/tui_e2e_hty.sh --journey 1,2,3,4,5,6,7`.
- Runs creation-workflow popup journeys: `./scripts/tui_e2e_hty.sh --journey 8,9,10`.

This catches interactive regressions in real TUI flows, including creation workflow behavior.

### 4. `runtime-smoke` (Ubuntu, macOS, Windows)
Checks cross-platform runtime behavior in isolated home/config directories.

- **Linux/macOS**: runs `./scripts/clipboard_smoke.sh --skip-clipboard-checks` with temporary `HOME/XDG_*` paths.
- **Windows**: runs `./scripts/clipboard_smoke.ps1 -SkipClipboardChecks` with temporary `HOME/APPDATA/TEMP` paths.

Purpose: quick real-runtime validation of core CLI/TUI execution paths without relying on host machine state.

### 5. `supply-chain-linux` (Ubuntu)
Performs dependency and vulnerability policy checks.

- Installs `cargo-deny` and `cargo-audit`.
- **`cargo deny check`**: enforces dependency policy (licenses/advisories/sources/bans as configured).
- **`cargo audit`**: checks for known RustSec vulnerabilities.
- **`cargo tree --locked -e normal`**: emits the normal dependency tree.
- Uploads `cargo-tree-normal.txt` as a CI artifact for inspection.

## What “passing CI” means

A commit/PR is green only when all jobs above pass for that run.

## Good to know

The commits after `bdf42556f0f0` (up to `9c75ea4e1a70`) fixed CI-only failures that were not reproducible locally. Keep these runner-specific behaviors in mind:

- Native codec dependencies can break hosted runners even when local builds succeed.
  - Symptom: transitive native dependency pressure from `image` AVIF native stack (`dav1d`/`dav1d-sys`) increased CI fragility.
  - Fix used: removed `avif-native` from `image` features (`8c4e7bc`), keeping AVIF support without requiring that native dependency chain on CI.

- Cross-platform lint/test imports must be gated precisely, not broadly.
  - Symptom: `#[cfg(unix)]` import logic caused issues in non-Linux CI contexts.
  - Fix used: narrowed permission import gate to `#[cfg(target_os = "linux")]` (`085a732`).

- Timing assertions around transient background-task UI state are flaky on shared runners.
  - Symptom: assertions expecting a short-lived “background task visible” state failed remotely.
  - Fix used: removed those timing-sensitive assertions and waited for task completion instead (`80a5ee9`).

- Clipboard availability is inconsistent on CI; tests must tolerate unavailable providers.
  - Symptom: copy-related TUI tests and status checks were brittle when no clipboard backend was usable.
  - Fixes used:
    - Made tests accept either successful copy notification or explicit clipboard failure status (`f2d4fe9`).
    - Added CI env `PETIGLYPH_MOCK_CLIPBOARD=1` and short-circuited clipboard writes under that env (`483c3fc`).

- Install-flow state transitions can race in CI.
  - Symptom: reinstall shortcut path and stale-output cleanup assertions intermittently failed due to focus/step variability.
  - Fixes used: made test flow more defensive (extra enter/focus handling, retry loop, direct rebuild fallback) while preserving functional assertions (`f2d4fe9`).

- `hty` E2E transitions need explicit retry/wait logic on runners.
  - Symptom: transition from tweak step to “Create Animation” occasionally missed on CI PTY scheduling.
  - Fixes used:
    - Added a dedicated retry helper to continue from tweak until animation config appears.
    - Added CI-only safer defaults for timeout and step delay when running under `GITHUB_ACTIONS` (without overriding explicit user overrides) (`cc3c49a`).

- Windows path parsing must preserve backslashes unless they escape a known token delimiter.
  - Symptom: shell/drop payload tokenization and unescape logic consumed backslashes in Windows paths, causing CI-only Windows failures.
  - Fixes used:
    - Updated token split/unescape logic in `src/tui.rs` and `src/animation_media.rs` to only treat backslash as escape before space/tab/quote/backslash.
    - Added regression tests for paths like `C:\Users\...\file.png` (`0f17634`, `7e747cb`).

- Path equality assertions should not rely on raw string identity across OSes.
  - Symptom: manifest path string comparisons differed due to path normalization/canonical forms.
  - Fix used: compare via helper that allows direct or canonicalized equality (`same_path`) (`0f17634`, cleaned in `ff553dd`).

- Runtime smoke scripts must avoid PowerShell reserved names and ambiguous splatting semantics.
  - Symptom: using `Args` as a parameter name in PowerShell wrapper was error-prone.
  - Fix used: renamed to `CliArgs` and updated call sites (`7e747cb`).

- System fallback fonts can pollute ownership scans on some runner images.
  - Symptom: `LastResort`-style fallback fonts advertised broad coverage and could be misinterpreted as owned PUA occupancy.
  - Fix used: explicitly ignore non-ownership fallback fonts (e.g., `LastResort`) during external scan (`7e747cb`).

- Commands that print to stdout can corrupt JSON-mode CLI output in CI checks.
  - Symptom: refresh commands writing to stdout leaked into JSON responses and broke parsers.
  - Fix used: switched refresh execution from `.status()` to `.output()`, suppressing successful output and only surfacing stdout/stderr on error (`9c75ea4`).

- CI hardening policy adopted in this period:
  - CI-only/headless-only adaptations are acceptable when needed for GitHub-hosted runner reliability.
  - Do not weaken local test journey coverage/semantics just to make CI pass (`97fb8eb`).
