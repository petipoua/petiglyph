# TODO-SCAF

Purpose: focused scaffolding cleanup plan for `petiglyph`.

Profile target:
- Keep repository small and maintainable.
- Keep multi-channel forward deployment (GitHub Releases + npm + PyPI + AUR).
- Reduce drift risk and hidden operational complexity.

Hard constraints from maintainer decision:
- Do NOT add PR templates.
- Do NOT add code of conduct.
- Do NOT add dependency update bot automation.
- Do NOT add optional code scanning/security automation.
- Do NOT change unit tests or E2E tests.

---

## 0. Definition Of Done

- All tasks in P0 and P1 are completed.
- Multi-channel release behavior remains unchanged from a user perspective.
- No test logic changes (unit/E2E files untouched).
- Docs are shorter, less duplicated, and have one clear source of truth per topic.

---

## P0 - Consistency Gaps (Fix First)

## P0.1 Harden `hty` install path in CI without changing test behavior

Problem:
- CI currently installs `hty` via a mutable upstream script reference (`main` branch), which is the biggest supply-chain inconsistency in current hardening.

Task:
- Replace mutable `hty` install in `.github/workflows/ci.yml` with one pinned/reproducible strategy.

Recommended options (choose one):
1. Pin raw installer URL to a specific commit SHA (fastest minimal change).
2. Download a versioned release artifact and verify SHA256 before install (stronger).
3. If no stable artifact strategy is available, keep current installer but add explicit risk acceptance note in docs and a periodic manual review task.

Maintainer note about remote CI:
- On hosted runners you still can pin by commit URL or by checksum-verified downloaded artifact; you do not need local preinstall.

Acceptance criteria:
- `ci.yml` no longer pulls `hty` from moving `main` without pin/checksum.
- CI still runs same journeys with no test-file changes.

---

## P0.2 Unify AUR runtime dependencies across local and release paths

Problem:
- `scripts/aur.sh` and release AUR path currently diverge in declared runtime deps.

Task:
- Define one canonical dependency list and reuse it for:
  - `PKGBUILD`
  - `scripts/aur.sh` generated PKGBUILD
  - `scripts/release_prepare_aur.sh` generated PKGBUILD

Implementation suggestion:
- Add a tiny shared shell include (example: `scripts/lib/pkg_meta.sh`) with canonical arrays/vars.
- Source it from both scripts to emit the same `depends=(...)`.

Acceptance criteria:
- Local AUR generation and release AUR generation output identical runtime dep set.
- `PKGBUILD` and `.SRCINFO` reflect same dependency contract.

---

## P0.3 Fix dependency docs drift (`avif-native` stale references)

Problem:
- Docs still reference `avif-native` in places where current Cargo feature set no longer matches.

Task:
- Update stale dependency statements in:
  - `docs/dependency-supply-chain.md`
  - `CROSS-COMPATIBILITY-GUIDE.md`
  - any duplicated references in `README.md` / release docs if present.

Acceptance criteria:
- Dependency docs accurately reflect `Cargo.toml` features and real dependency graph.
- No conflicting statements across docs.

---

## P0.4 Make `AGENTS.md` portable and path-safe

Problem:
- `AGENTS.md` contains host-specific absolute paths and path typos.

Task:
- Replace absolute path references with repo-relative paths.
- Keep content semantics unchanged.

Acceptance criteria:
- No machine-specific paths required to understand repo structure.
- Path references are valid for any clone location.

---

## P0.5 Eliminate platform matrix duplication drift risk (wrappers/release)

Problem:
- Platform target lists are repeated across workflows, npm metadata, and scripts.
- Drift risk is high over time.

Task:
- Introduce one canonical machine-readable matrix file (example: `docs/distribution-matrix.json` or `scripts/distribution-targets.json`) containing:
  - release Rust targets
  - archive extension
  - npm package mapping
  - binary filename per platform
- Update scripts to consume that source where practical (`release_stage_npm_artifacts.sh`, optional `release_npm_pack_test.sh`).
- For workflow YAML (where full dynamic generation may be overkill), add a lightweight sync-check script that validates YAML entries against canonical matrix.

Acceptance criteria:
- One canonical matrix exists.
- At least scripts use canonical matrix directly.
- CI has a sync check that fails on matrix drift.

---

## P1 - Rust Configs (Required)

## P1.1 Add explicit MSRV contract

Task:
- Add `rust-version = "..."` to `Cargo.toml` under `[package]`.

Guideline:
- Pick a version compatible with current dependencies and CI toolchain.

Acceptance criteria:
- `Cargo.toml` includes explicit `rust-version`.
- Build/test still pass with configured toolchain.

---

## P1.2 Decide and document toolchain pin policy

Task:
- Choose one policy and apply consistently:
1. Pin exact toolchain in `rust-toolchain.toml` (reproducibility-first), or
2. Keep `stable` and explicitly document why (maintenance-first).

Recommended for this repo profile:
- Prefer exact pin for release reproducibility if update burden is acceptable.

Acceptance criteria:
- Policy is explicit in docs.
- `rust-toolchain.toml` and docs are aligned.

---

## P2 - Docs Redundancy Reduction / Drift Control

Goal:
- Keep docs minimal but sufficient for multi-channel release operations.

## P2.1 Collapse overlapping release docs into one canonical runbook + one short checklist

Current overlap:
- `RELEASE-GUIDE.md`
- `RELEASE-CHECKLIST.md`
- parts of `README.md`

Task:
- Keep:
  - one canonical release runbook (detailed but concise)
  - one short executable checklist
- Remove duplicated procedural blocks from other files and replace with links to canonical docs.

Acceptance criteria:
- Only one place owns full release flow instructions.
- Checklist is concise and non-duplicative.

---

## P2.2 Trim CI narrative docs to operational essentials

Problem:
- CI history and postmortem-like detail is useful once but creates long-term maintenance drag.

Task:
- Reduce `CI.md` to:
  - triggers
  - jobs and intent
  - local equivalent commands
  - short troubleshooting pointers
- Move deep historical incident details to optional archive note (or remove if no longer needed).

Acceptance criteria:
- `CI.md` is significantly shorter and operationally focused.
- No loss of actionable day-to-day CI understanding.

---

## P2.3 Define doc ownership map to prevent future duplication

Task:
- Add a short section in `README.md` or `AGENTS.md` describing canonical ownership:
  - user usage contract -> `README.md`
  - release operations -> release runbook
  - CI behavior -> `CI.md`
  - dependency/supply-chain specifics -> dependency doc

Acceptance criteria:
- Every repeated topic has one declared source of truth.
- Cross-links replace copy-pasted repeated content.

---

## P3 - Licensing Improvements (Minimal, Practical)

Status: completed on 2026-06-01.

## P3.1 Tighten `deny.toml` license allowlist to what is actually used

Problem:
- Current policy warns about at least one allowed-but-not-encountered license entry.

Task:
- Remove unused allowlist entries or annotate why they are intentionally pre-allowed.
- Keep policy strict but practical.

Acceptance criteria:
- `cargo deny check licenses` passes without avoidable policy-noise warnings.
- Done:
  - removed unused `MPL-2.0` allowance from `deny.toml`.

---

## P3.2 Add lightweight third-party license inventory artifact process

Task:
- Add one repeatable command/process to export dependency license inventory at release time.

Implementation choices:
1. Document a `cargo deny`/`cargo tree` based manual inventory snapshot process, or
2. Add a small script generating `docs/THIRD_PARTY_LICENSES.md` from tooling output.

Constraint:
- Keep it lightweight; no extra heavy automation required.

Acceptance criteria:
- Release process includes explicit dependency-license visibility step.
- Maintainer can quickly answer "what third-party licenses are bundled transitively?"
- Done:
  - added `scripts/generate_third_party_licenses.sh`.
  - added generated artifact `docs/THIRD_PARTY_LICENSES.md`.
  - wired regeneration step into `RELEASE-GUIDE.md` and `RELEASE-CHECKLIST.md`.

---

## P3.3 Verify and document asset license provenance for `icons/`

Task:
- Ensure each redistributed sample asset has clear provenance/license.
- Record provenance in one small doc section (or dedicated `docs/assets-provenance.md`).

Acceptance criteria:
- No unknown provenance sample assets remain in release-bound repository state.
- Done:
  - removed repository `icons/` fixture pack from git tracking.
  - added `test-assets/` with maintainer-created fixtures.
  - added `docs/assets-provenance.md`.
  - ignored repository-root `icons/` in `.gitignore`.

---

## P4 - Optional Nice-To-Have (Only If Effort Is Low)

## P4.1 Add a small scaffolding preflight command

Task:
- Add a script or `make` target that runs only scaffolding drift checks:
  - release matrix sync check
  - docs source-of-truth check (basic)
  - `cargo deny` quick pass

Constraint:
- Do not change test suite.

Acceptance criteria:
- Single command validates non-app scaffolding consistency quickly.

---

## Out Of Scope (Explicitly Deferred By Maintainer)

- PR templates.
- Code of conduct.
- Dependabot / automated dependency update bots.
- CodeQL or other optional code scanning/security automation additions.
- Unit/E2E test behavior/content changes.

---

## Execution Order Recommendation

1. P0.2 AUR dependency unification
2. P0.3 dependency docs drift fix
3. P0.4 AGENTS path portability
4. P1 Rust config fixes
5. P0.5 platform matrix single source + sync check
6. P0.1 `hty` install hardening (strategy chosen once validated on remote)
7. P2 docs consolidation and redundancy removal
8. P3 licensing improvements

Reason for this order:
- Early tasks are high-confidence and reduce immediate drift.
- `hty` hardening is important but may need one remote-compatible decision iteration.
- Docs/licensing cleanup then locks in reduced maintenance overhead.
