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
- `cargo fmt --check`
- `cargo check --locked`
- `cargo clippy --locked --all-targets --all-features -- -D warnings`
- `cargo test --locked`

## 2. `package-hygiene-linux` (Ubuntu)

Intent: ensure the package/release tree is clean and shippable.

Runs:
- `cargo package --list --allow-dirty`
- `./scripts/release_assert_clean_tree.sh`

## 3. `tui-e2e-hty-linux` (Ubuntu)

Intent: catch interactive TUI regressions with process-level journeys.

Runs:
- install `hty` in CI
- `hty --help`
- `./scripts/tui_e2e_hty.sh --journey 1,2,3,4,5,6,7`
- `./scripts/tui_e2e_hty.sh --journey 8,9,10`

## 4. `runtime-smoke` (Ubuntu, macOS, Windows)

Intent: validate cross-platform runtime behavior in isolated home/config dirs.

Runs:
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
cargo check --locked
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked
./scripts/release_assert_clean_tree.sh
./scripts/tui_e2e_hty.sh
./scripts/clipboard_smoke.sh --skip-clipboard-checks
cargo deny check
cargo audit
cargo tree --locked -e normal
```

## Quick Troubleshooting

- If CI-only flakiness appears, prefer CI-only/headless-only adaptations over weakening local E2E semantics.
- For TUI E2E issues, verify `hty` behavior first (`hty --help`, `hty help run/send/wait`).
- For dependency/security failures, inspect `cargo-tree-normal.txt` artifact and `docs/dependency-supply-chain.md`.
- For packaging drift, run `./scripts/release_assert_clean_tree.sh` and re-check version sync with `./scripts/release_sync_versions.sh`.
