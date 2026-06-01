# Petiglyph OSS Scaffolding Report

Date: 2026-06-01  
Repository: `petiglyph`  
Scope: scaffolding and repository operations only (CI/CD, release, packaging, security, docs, governance, wrappers, dependency policy, licensing). Core app behavior/code was intentionally excluded.

## 1. Executive Summary

`petiglyph` is already a strong OSS repository from a scaffolding perspective. It has:

- a serious release pipeline across GitHub Releases, npm, PyPI, and AUR,
- strong supply-chain controls (pinned actions, least-privilege permissions, checksum verification, artifact attestations, trusted publishing),
- broad testing coverage (unit/integration plus process-level TUI E2E),
- and rich operational documentation.

For your specific objective (a clean, minimal, reusable OSS base template), the main issue is not missing structure. The main issue is **operational and documentation sprawl** in a few areas, plus a handful of consistency gaps.

Bottom line:

- As an **advanced distribution template** for cross-platform binary tools: this repo is very good.
- As a **default template for all future OSS projects**: it is somewhat heavy and should be split into a lighter baseline + optional advanced distribution layer.

## 2. Direct Answers to Your Questions

### 2.1 Is the current scaffolding reasonable for this type of project?

Yes. For a Rust CLI/TUI binary distributed via GitHub, npm, PyPI, and AUR, this scaffolding is reasonable and often above average.

- Some parts are overkill for a minimal baseline (mainly docs/process volume and E2E harness complexity).
- Some parts are not enough for public OSS governance maturity (PR templates, code of conduct, automated dependency update bot, optional code scanning/security automation).

### 2.2 Does this look like a good OSS project starting point?

Yes, with one adjustment:

Use this as your **"advanced shipping template"**, not your universal default.

For future projects, define two template profiles:

1. `core-oss-template` (minimal): basic CI + tests + security baseline + simple release.
2. `multi-distribution-template` (this model): multi-arch binaries, npm/PyPI wrappers, attestations, staging gates.

That keeps future repositories clean while preserving this strong architecture when needed.

## 3. Rating Grid (Good / Overkill / Not Enough)

| Area | Rating | Verdict |
|---|---|---|
| CI | Good | Strong cross-platform checks with useful separation of quality, runtime smoke, E2E, and supply chain. |
| Security practices | Good | Excellent baseline for an early OSS project; one weak spot on dynamic installer fetch. |
| Packaging (overall) | Good | Coherent multi-channel packaging strategy and release choreography. |
| Testing suite (unit + e2e) | Overkill | Very high quality, but heavy for a generic minimal template. |
| Documentation (users + maintainers + contributors + AI agents) | Overkill | Rich but duplicated and costly to maintain; drift risk is high. |
| AUR packaging config | Good | Practical and workable; one dependency consistency issue to fix. |
| npm packaging config | Good | Correct native package split and verification flow. |
| PyPI packaging config | Good | Sensible maturin + TestPyPI gate strategy. |
| GitHub release packaging config | Good | Robust multi-target build and publish process with attestations/checksums. |
| Rust configs | Not enough | Missing explicit MSRV in Cargo metadata and tighter toolchain reproducibility. |
| Wrappers config (JS/Python/AUR scripts) | Good | Effective and coherent, but platform matrix duplication can drift. |
| Licensing (incl. dependencies) | Good | Clear MIT licensing and deny/audit policy; dependency disclosure can be tightened. |

## 4. Detailed Analysis by Topic

## 4.1 CI

### What is good

- Multi-OS quality matrix (`ubuntu-latest`, `macos-latest`, `windows-latest`) for `fmt/check/clippy/test`.
- Dedicated runtime smoke jobs with isolated home/config directories.
- Dedicated TUI E2E workflow using `hty`.
- Dedicated supply-chain workflow running `cargo deny`, `cargo audit`, and artifacting dependency tree.
- Workflow actions pinned to full SHAs.
- `concurrency` cancellation is enabled to avoid stale CI runs.

### What is overkill

- Not the CI itself, but the associated historical troubleshooting narrative in docs is too large for a reusable baseline.

### What is not enough

- `hty` installation in CI uses a mutable install script URL (`curl ... main/scripts/install.sh | sh`). This is the principal weak point compared to otherwise strong pinning practices.

### Recommendation

- Keep the CI architecture.
- Pin `hty` install source to immutable artifact/checksum or immutable commit path.

## 4.2 Security Practices

### What is good

- Strong principle of least privilege in workflow `permissions`.
- Trusted publishing (OIDC) for npm/PyPI.
- Integrity verification before npm publish (`gh release verify` + `SHA256SUMS` check).
- Artifact attestations in release workflow.
- Clear private vulnerability reporting policy (`SECURITY.md`).
- Cargo supply-chain checks via `cargo deny` and `cargo audit`.

### What is overkill

- Nothing major. These controls are appropriate for a public binary-distribution project.

### What is not enough

- Missing automation commonly expected in public OSS:
  - Dependabot (or equivalent) for dependencies/actions updates,
  - optional code scanning (CodeQL) and secret scanning policy statement.

### Recommendation

- Add lightweight `dependabot.yml` and optionally CodeQL workflow.
- Keep current release hardening as-is.

## 4.3 Packaging (Overall)

### What is good

- Canonical binaries are built once in GitHub release workflow.
- npm consumes release artifacts rather than rebuilding differently.
- PyPI wheels/sdist are built in controlled workflow with staging gate through TestPyPI.
- AUR release preparation script computes immutable source checksum from tag tarball.

### Risks

- Platform matrix and dependency definitions appear in several locations, which can drift.

### Recommendation

- Keep current process but centralize matrix/dependency constants where feasible (single source of truth strategy).

## 4.4 Testing Suite (Unit Tests + E2E)

### What is good

- Very broad coverage:
  - in-crate tests are extensive,
  - CLI contract test file is extensive,
  - process-level E2E journeys cover core user stories.
- Good use of deterministic artifact assertions and explicit waits.

### What is overkill

- For a base template, this is far beyond minimal:
  - `scripts/tui_e2e_hty.sh` is very large,
  - large test surface increases maintenance burden and CI flake management overhead.

### What is not enough

- No critical gap for this repo.

### Recommendation

- Keep this test depth for `petiglyph`.
- For future default template: start with minimal contract tests + a smaller smoke E2E suite, then scale up only when product complexity demands it.

## 4.5 Documentation (Users, Maintainers/Contributors, AI Agents)

### What is good

- Documentation breadth is excellent:
  - user docs (`README.md`),
  - contributor/security docs,
  - CI and testing docs,
  - release runbook/checklist,
  - compatibility notes,
  - AI agent instructions.

### What is overkill

- There is significant overlap across:
  - `README.md`,
  - `RELEASE-GUIDE.md`,
  - `RELEASE-CHECKLIST.md`,
  - `CROSS-COMPATIBILITY-GUIDE.md`,
  - and parts of `CI.md`.

High overlap causes maintenance drift risk.

### What is not enough

- Contributor-facing community governance is light:
  - no PR template,
  - no code of conduct,
  - no discussion of support boundaries (issues/discussions/triage labels) beyond basics.

### Recommendation

- Consolidate release and compatibility docs.
- Add minimal governance files.
- Keep AGENTS but remove machine-specific absolute paths.

## 4.6 AUR Packaging Config

### What is good

- Clear split between:
  - local Arch packaging/testing (`scripts/aur.sh`),
  - release-grade AUR prep (`scripts/release_prepare_aur.sh`).
- Release prep script uses immutable tag source URL + checksum generation.

### Issue

- Dependency mismatch risk:
  - `PKGBUILD` / release prep include `fontconfig`,
  - local generator path in `aur.sh` writes only `ffmpeg`.

### Recommendation

- Unify dependency declaration so local and release paths emit the same runtime deps.

## 4.7 npm Packaging Config

### What is good

- Correct architecture for native binaries in npm ecosystem:
  - thin meta package + platform-specific optional dependencies.
- Runtime selector in JS shim is straightforward and explicit.
- Publish order is correct (platform packages first, then meta package).
- Pre-publish integrity checks are strong.

### Minor risks

- Platform package list is duplicated in multiple scripts/workflows.

### Recommendation

- Centralize the package target map in one machine-readable file and derive workflow/script checks from it.

## 4.8 PyPI Packaging Config

### What is good

- `maturin` binary distribution setup is correct for a Rust CLI package in Python ecosystem.
- TestPyPI staging before PyPI is a strong safety gate.
- Uses trusted publishing and `twine check`.

### Limits

- No musllinux wheels (already documented).

### Recommendation

- Keep as-is unless demand appears.
- Optional improvement: add post-build smoke install/run check before final publish gate.

## 4.9 GitHub Release Packaging Config

### What is good

- Strong multi-target matrix and packaging logic.
- Archive smoke checks include behavior checks, not just `--help`.
- Checksums + attestations + draft release gate are good design.

### Recommendation

- Keep this architecture unchanged.

## 4.10 Rust Configs

### What is good

- Uses modern edition and pinned components in toolchain file.
- Lockfile discipline (`--locked`) is enforced in CI/release flows.

### Not enough

- `Cargo.toml` does not declare `rust-version` (MSRV contract).
- `rust-toolchain.toml` uses floating `stable`; reproducibility is lower than explicit stable version pin.

### Recommendation

- Set `rust-version` in `Cargo.toml`.
- Decide policy:
  - either pin toolchain version for exact reproducibility,
  - or keep floating stable intentionally and document that policy clearly.

## 4.11 Wrapper Configs (JS, Python, AUR PKGBUILD and scripts)

### What is good

- Wrappers are coherent and follow a sensible “single native binary per platform package” model.
- Release scripts automate version sync and binary staging well.

### Risk

- Repetition of platform lists/dependency metadata across multiple files raises drift probability.

### Recommendation

- Introduce one source file for target/package/dependency mapping, then generate checks and script loops from it.

## 4.12 Licensing (Including Dependencies)

### What is good

- Project license is clear (MIT).
- Dependency license policy is codified (`deny.toml`).
- Advisory policy is codified with explicit temporary ignore for known transitive warning.

### What to improve

- Keep dependency policy docs synchronized with actual features/dependency graph.
- Consider generating a machine-readable third-party notices artifact at release time for transparency.

## 5. Important Consistency Gaps Found

1. **Dynamic external script in CI (`hty` install) weakens supply-chain hardening.**
2. **AUR local script dependency list differs from release PKGBUILD path.**
3. **Dependency docs mention `avif-native` while Cargo features currently use `avif` only.**
4. **AGENTS file contains machine-specific absolute paths with path typo, reducing portability and reliability.**

## 6. Is This Overkill or Underkill for an OSS Baseline?

For `petiglyph` itself, the scaffolding is mostly well-calibrated.

For a **generic baseline template** used across many future projects, current state is:

- **Overbuilt** in process/docs/test harness complexity.
- **Underbuilt** in lightweight public-project governance defaults.

So the right move is not to simplify this repo aggressively. The right move is to **extract two template tiers**.

## 7. Suggested Template Strategy for Future Projects

## 7.1 Tier A: Minimal OSS Base (default)

Include only:

- `README.md` (concise),
- `LICENSE`,
- `CONTRIBUTING.md`,
- `SECURITY.md`,
- one CI workflow (`fmt/check/lint/test` + optional one smoke),
- one release workflow (single channel),
- dependency bot config,
- bug + PR template,
- optional code of conduct.

## 7.2 Tier B: Advanced Multi-Distribution Base (opt-in)

Add when needed:

- multi-target release matrix,
- npm platform packages,
- PyPI maturin flow with staging gates,
- AUR scripts/metadata,
- attestations/checksum verification,
- extensive operator release docs,
- process-level E2E harness.

`petiglyph` maps naturally to Tier B.

## 8. Priority Action Plan (Recommended)

## 8.1 Immediate (high value, low/medium effort)

1. Pin or checksum-verify `hty` installer input in CI.
2. Unify AUR dependency definitions (`ffmpeg` + `fontconfig`) across local/release paths.
3. Update dependency docs to match actual Cargo feature graph.
4. Replace absolute/host-specific paths in `AGENTS.md` with repo-relative references.

## 8.2 Near-term (medium value)

1. Add `rust-version` in `Cargo.toml` (explicit MSRV contract).
2. Add PR template and code of conduct.
3. Add dependency/action update automation (`dependabot.yml`).
4. Optionally add CodeQL workflow.

## 8.3 Structural simplification (template hygiene)

1. Consolidate release docs:
   - keep one canonical runbook,
   - keep one concise checklist.
2. Move deep CI incident chronology out of primary docs into a maintenance log.
3. Reduce repeated platform matrices by centralizing metadata.

## 9. Final Assessment

This repository is already a credible public OSS project and a strong example of responsible release/security engineering for a cross-platform native CLI/TUI.

To make it your long-term reusable OSS foundation, the best path is:

- preserve the current quality/security/release rigor,
- remove avoidable duplication and drift vectors,
- and split your reusable template philosophy into minimal-core vs advanced-distribution tiers.

That gives you both:

- a clean/simple default for most future repos,
- and a production-grade distribution scaffold when project scope justifies it.

## 10. Evidence Pointers (Key Files Audited)

- CI/release/publish workflows:
  - `.github/workflows/ci.yml`
  - `.github/workflows/release.yml`
  - `.github/workflows/npm-publish.yml`
  - `.github/workflows/pypi-publish.yml`
- Packaging and release scripts:
  - `scripts/release_assert_clean_tree.sh`
  - `scripts/release_sync_versions.sh`
  - `scripts/release_prepare_aur.sh`
  - `scripts/release_stage_npm_artifacts.sh`
  - `scripts/release_npm_pack_test.sh`
  - `scripts/aur.sh`
- Package metadata:
  - `Cargo.toml`
  - `pyproject.toml`
  - `PKGBUILD`
  - `.SRCINFO`
  - `npm/petiglyph/package.json`
  - `npm/petiglyph/bin/petiglyph.js`
  - `npm/petiglyph-*/package.json`
- Security/license/dependency policy docs:
  - `deny.toml`
  - `SECURITY.md`
  - `docs/dependency-supply-chain.md`
- User/maintainer/docs surface:
  - `README.md`
  - `CONTRIBUTING.md`
  - `CI.md`
  - `TESTS.md`
  - `RELEASE-GUIDE.md`
  - `RELEASE-CHECKLIST.md`
  - `CROSS-COMPATIBILITY-GUIDE.md`
  - `AGENTS.md`

## 11. Tooling Checks Executed During This Audit

- `cargo tree --locked -e normal`
- `cargo tree --locked --duplicates`
- `cargo deny check`
- `cargo audit`

Observed outcome summary at audit time:

- `cargo deny check` passed with duplicate warnings and one unmatched allowed-license policy entry warning.
- `cargo audit` passed with one explicitly tolerated advisory warning (`RUSTSEC-2024-0436`) in the AVIF transitive chain.

