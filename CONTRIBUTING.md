# Contributing

Thanks for helping improve `petiglyph`.

## Before Opening a PR

Run the local checks:

```bash
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked
```

## Project Guardrails

- Keep changes focused and minimal.
- If behavior changes, update tests and docs in the same PR.
- TUI guardrail: do not change TUI behavior, keybindings, copy/text, layout, or flow unless the change is explicitly requested.
- For local throwaway test projects created inside this repo, use `/.scratch/`.
- Keep the repo root clean of ad hoc local project folders; `test-assets/` is reserved for tracked fixtures.

## Debugging and Intervention Guidance

- For user-facing CLI diagnostics (`doctor`, `list`, automation commands), do not silently drop recoverable errors. Surface what failed and what was skipped.
- Keep machine-readable outputs trustworthy: if human output has a warning/finding, expose the same state in JSON output.
- In contract tests, assert semantics over formatting:
  - Parse TOML/JSON when checking manifest/output shape.
  - Avoid brittle spacing/alignment assertions for help text unless wording itself is the contract.
- In TUI E2E flows, wait on state-specific markers, not generic popup titles.
- In TUI E2E selectors, prefer semantic labels/controls over decorative border glyphs.

## Bug Reports

Please use the bug report issue template and include reproduction steps plus environment details.
