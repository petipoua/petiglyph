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

## Bug Reports

Please use the bug report issue template and include reproduction steps plus environment details.
