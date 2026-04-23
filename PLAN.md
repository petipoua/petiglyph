# Petiglyph Integration Plan

## Goal
Keep `petiglyph` excellent as a direct TUI for humans while making its CLI stable and machine-friendly for other apps to call in binary form across platforms/languages.

## Scope
- Keep current TUI-first user experience.
- Strengthen automation commands (`build`, `sample`, `install-font`) for programmatic callers.
- Add explicit lifecycle support for removing installed fonts via CLI (`uninstall-font`).
- Do **not** add a `clean` command to avoid overlap/confusion with uninstall semantics.

## Phase 1: Machine-Readable CLI Contract
1. Add `--json` support to `build`, `sample`, `install-font`, and `uninstall-font` (once added).
2. Define stable JSON response schemas with top-level fields:
   - `ok` (bool)
   - `command` (string)
   - `version` (string)
   - `data` (object)
   - `error` (object, only on failure)
3. Ensure JSON mode writes machine-readable payloads to stdout and keeps human logs off stdout.
4. Keep non-zero exit codes on failure and zero on success for all automation commands.

## Phase 2: Font Lifecycle Management
1. Keep/install behavior idempotent in `install-font`.
2. Add `uninstall-font` command that removes the font installed by `petiglyph` for a given manifest/project scope.
3. Require explicit scope (`--manifest` defaulting to local manifest) and prevent deleting unrelated fonts.
4. Return clear outcomes for uninstall:
   - removed
   - already absent
   - blocked/error with actionable message

## Phase 3: Safety and Cross-Platform Behavior
1. Preserve Linux behavior under `~/.local/share/fonts/petiglyph/<project>/`.
2. Add OS-specific install/uninstall handling for macOS and Windows in a scoped, testable way.
3. Make font cache refresh strategy OS-aware and fail with explicit diagnostics when refresh is unavailable.

## Phase 4: Developer Integration Quality
1. Add integration tests for automation commands:
   - exit codes
   - JSON schemas
   - idempotent install/uninstall flows
2. Add docs for app integrators:
   - command examples in shell/Node/Python
   - error handling expectations
   - uninstall flow expectations
3. Document command stability policy and versioning expectations.

## Phase 5: Release Readiness
1. Publish prebuilt binaries for Linux, macOS, and Windows.
2. Add release notes template that calls out contract/schema changes.
3. Verify README command section stays aligned with implementation.
4. When every task in this `PLAN.md` is fully implemented, delete `PLAN.md`.
