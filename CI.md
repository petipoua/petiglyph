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
