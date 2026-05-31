# RELEASE-GUIDE

Last updated: 2026-05-31

This guide is the operator runbook for publishing `petiglyph` to:

- GitHub Releases (canonical binaries)
- AUR (`petiglyph` source package)
- npm (`petiglyph` meta package + `@petiglyph/*` platform packages)
- PyPI (maturin `bin` wheels + sdist)

It complements `RELEASE-CHECKLIST.md` and reflects the release automation now in this repository.

## 1. What Is Implemented In Repo

- GitHub artifact release workflow: `.github/workflows/release.yml`
  - Trigger: tag push (`v*`) or manual `workflow_dispatch` with `tag`.
  - Builds 8 targets.
  - Packages archives per target.
  - Runs per-target archive smoke checks (`--help`, `doctor --json`, non-interactive `tui` failure).
  - Generates `SHA256SUMS`.
  - Publishes release assets to a draft GitHub Release using `docs/release-notes-template.md`.
  - Generates GitHub artifact attestations.
  - Uses immutable full-commit SHA pins for all referenced actions.
- npm publish workflow: `.github/workflows/npm-publish.yml`
  - Trigger: GitHub release `published`.
  - Downloads release archives.
  - Verifies release integrity (`gh release verify`) and release `SHA256SUMS` before publishing.
  - Stages native binaries into `npm/*` packages.
  - Publishes platform packages, then meta package.
- PyPI/TestPyPI publish workflow: `.github/workflows/pypi-publish.yml`
  - Trigger: GitHub release `published`.
  - Builds Linux GNU manylinux, macOS, and Windows wheels plus sdist with maturin.
  - Does not currently build musllinux wheels.
  - Verifies release integrity (`gh release verify`) before publishing.
  - Publishes to TestPyPI then PyPI using trusted publishing.
- Packaging/release helper scripts:
  - `scripts/release_assert_clean_tree.sh`
  - `scripts/release_sync_versions.sh`
  - `scripts/release_stage_npm_artifacts.sh`
  - `scripts/release_npm_pack_test.sh`
  - `scripts/release_prepare_aur.sh`
- Supply-chain policy and dependency notes:
  - `deny.toml`
  - `docs/dependency-supply-chain.md`
- PyPI metadata: `pyproject.toml`
- npm package layout: `npm/` (meta + 8 platform packages)
- License file: `LICENSE`
- Runtime media dependency policy:
  - Arch packaging declares `depends=('ffmpeg')`.
  - GitHub/npm/PyPI artifacts do not bundle `ffmpeg`; runtime docs and the interactive prompt cover missing `ffmpeg`.

## 2. One-Time Account And Access Setup

### 2.1 GitHub

1. Enable 2FA/passkey on maintainer accounts.
2. Ensure repo admin rights to configure:
   - Environments: `npm`, `testpypi`, `pypi`.
   - Required reviewers on these environments.
   - Prevent self-review for environment approvals.
3. Ensure branch/tag governance:
   - Protect `main`.
   - Restrict who can push release tags (`v*`).
4. Configure release notes process so macOS + Windows unsigned binary warning is always included.

### 2.2 AUR

1. Create an AUR account.
2. Generate a dedicated SSH keypair for AUR publishing.
3. Add the public key to your AUR account SSH keys.
4. Ensure you can authenticate:

```bash
ssh aur@aur.archlinux.org help
```

5. Create/own the `petiglyph` package base on AUR before first push.

### 2.3 npm

1. Create/secure npm account (2FA/passkey enabled).
2. Reserve package names:
   - `petiglyph`
   - `@petiglyph/petiglyph-linux-x64-gnu`
   - `@petiglyph/petiglyph-linux-arm64-gnu`
   - `@petiglyph/petiglyph-linux-x64-musl`
   - `@petiglyph/petiglyph-linux-arm64-musl`
   - `@petiglyph/petiglyph-darwin-x64`
   - `@petiglyph/petiglyph-darwin-arm64`
   - `@petiglyph/petiglyph-win32-x64-msvc`
   - `@petiglyph/petiglyph-win32-arm64-msvc`
3. For each package, configure npm Trusted Publisher for the GitHub repo/workflow:
   - workflow: `.github/workflows/npm-publish.yml`
4. Prefer trusted publishing (OIDC). Avoid long-lived publish tokens.

### 2.4 PyPI / TestPyPI

1. Create and secure PyPI + TestPyPI accounts.
2. Create projects (`petiglyph`) or configure publisher during first release.
3. Configure Trusted Publisher on TestPyPI and PyPI:
   - owner/repo: `petipoua/petiglyph`
   - workflow: `.github/workflows/pypi-publish.yml`
   - environment: `testpypi` (TestPyPI), `pypi` (PyPI)
4. Prefer trusted publishing (OIDC). Do not store long-lived `PYPI_TOKEN` secrets unless emergency fallback is needed.

## 3. Versioning And Tagging Procedure

1. Sync versions to release version:

```bash
./scripts/release_sync_versions.sh 0.1.0
```

2. Validate working tree/tests.
3. Commit.
4. Assert the committed release tree is clean and packageable:

```bash
./scripts/release_assert_clean_tree.sh
```

5. Create signed annotated tag:

```bash
git tag -s v0.1.0 -m "petiglyph v0.1.0"
git push origin v0.1.0
```

6. The tag push triggers `.github/workflows/release.yml`.

## 3.1 Technical Execution Timeline (From Commit To Registries)

This section describes exactly what happens when you decide a specific commit is releasable.

Important trigger behavior:

- No, it does not run on each new commit.
- `.github/workflows/release.yml` runs on `push` of tags matching `v*`, and can also be run manually with `workflow_dispatch` for an existing tag.
- `.github/workflows/npm-publish.yml` and `.github/workflows/pypi-publish.yml` run only on GitHub Release event `published`.

Automation boundary (critical to understand):

- This pipeline is semi-automated, not fully automatic.
- Build/publish jobs run automatically after their trigger events.
- Trigger events and approval gates are human-controlled.
- If you publish a GitHub Release, you are intentionally starting registry workflows.

### A. You choose the commit

1. You prepare and merge your release changes to `main`.
2. You ensure versions are synchronized (`Cargo.toml`, npm packages, PKGBUILD, docs) with:

```bash
./scripts/release_sync_versions.sh X.Y.Z
```

3. You commit that final release state.
4. You create and push signed tag `vX.Y.Z` pointing to that commit:

```bash
git tag -s vX.Y.Z -m "petiglyph vX.Y.Z"
git push origin vX.Y.Z
```

At this point, the release-artifacts workflow starts automatically because of the tag push.

### B. GitHub artifacts workflow runs (`release.yml`)

1. Event context is `push` with `refs/tags/vX.Y.Z`.
2. Build matrix launches parallel jobs for 8 targets.
3. Each job:
   - checks out the tagged commit (`actions/checkout` pinned SHA),
   - builds target with cross toolchain action (pinned SHA),
   - packages archive containing binary + `README.md` + `LICENSE`,
   - uploads archive as workflow artifact.
4. Publish job waits for all matrix jobs to complete.
5. Publish job:
   - validates release tag naming format,
   - downloads all build artifacts,
   - computes `SHA256SUMS`,
   - creates GitHub artifact attestations,
   - uploads archives + `SHA256SUMS` to a draft GitHub Release with template-based notes.

Result: GitHub Release now has canonical binaries + checksum file + attestations.

### C. You publish the GitHub Release

The npm/PyPI workflows do not start until the GitHub Release is in `published` state.

- If you keep the release as draft, registry workflows do not run.
- Once published, `release: published` event triggers both registry workflows.

This means you have a deliberate manual gate between:

1. “Artifacts built and attached to GitHub Release”
2. “Registry publishing is allowed to start”

Practical rule:

- Do not click Publish Release until you are ready for npm/TestPyPI/PyPI jobs to execute.

### D. npm workflow runs (`npm-publish.yml`)

1. Checks out exact tagged commit from release event.
2. Uses pinned `setup-node`.
3. Verifies release integrity:
   - `gh release verify "$TAG"` (release-level verification),
   - downloads release assets + `SHA256SUMS`,
   - verifies downloaded assets with `sha256sum -c SHA256SUMS`.
4. Verifies tag/version consistency (`Cargo.toml` version must match release tag).
5. Stages binaries from release archives into platform package directories via:
   - `scripts/release_stage_npm_artifacts.sh dist-release`
6. Verifies every expected platform binary exists.
7. Runs `npm pack --dry-run` for each package.
8. Publishes platform packages first, then meta package.
9. npm publish uses OIDC trusted publishing (`id-token: write`) through protected `npm` environment.

### E. PyPI workflow runs (`pypi-publish.yml`)

1. Wheel build jobs check out tag and build Linux GNU manylinux, macOS, and Windows wheels with maturin action (pinned SHA).
2. Sdist job builds source distribution.
3. Build outputs are uploaded as workflow artifacts.
4. TestPyPI publish job:
   - requires `testpypi` environment approval,
   - verifies release integrity (`gh release verify "$TAG"`),
   - downloads wheels + sdist artifacts,
   - publishes to TestPyPI with OIDC trusted publishing.
5. After TestPyPI publish succeeds, run manual staging validation before final PyPI approval:
   - create a clean virtual environment,
   - install from TestPyPI,
   - run a minimal runtime check (`petiglyph --help`).

```bash
python -m venv .venv-testpypi
. .venv-testpypi/bin/activate
pip install --index-url https://test.pypi.org/simple/ --extra-index-url https://pypi.org/simple petiglyph
petiglyph --help
```

6. If staging validation passes, approve the `pypi` environment gate.
7. PyPI publish job:
   - depends on successful TestPyPI publish,
   - requires `pypi` environment approval,
   - publishes same artifacts to PyPI with OIDC trusted publishing.

Behavior summary:

- With required reviewers enabled on `testpypi`/`pypi`, publication is staged and requires manual approval.
- Without required reviewers, publish continues automatically once previous jobs succeed.

### F. Security controls active during this flow

1. Actions are pinned to immutable commit SHAs.
2. Minimal token permissions are set per workflow/job.
3. Registry publish jobs require environment gates (`npm`, `testpypi`, `pypi`).
4. Integrity is checked before npm/PyPI publication.
5. Canonical binaries are produced once (tag workflow) and consumed downstream.

### G. What does not happen automatically

1. AUR publish is not automatic; you still run `scripts/release_prepare_aur.sh` and push AUR repo manually.
2. Runtime validation on real target machines is not automatic for every target; you still run smoke/runtime checks before announcement.

## 3.2 Explicit Manual vs Automatic Steps

Use this section as the control-plane reference.

### Manual actions (you must do these)

1. Pick release commit on `main`.
2. Sync/review versions and commit release prep changes.
3. Create and push signed tag `vX.Y.Z`.
4. Wait for artifact workflow completion and review results.
5. Validate release assets/checksums/notes.
6. Publish GitHub Release (this is the event that starts npm/PyPI workflows).
7. Approve protected environment gates (`npm`, `testpypi`, `pypi`) when prompted.
8. Run TestPyPI install validation before approving final PyPI gate.
9. Perform AUR publish manually.

### Automatic actions (GitHub Actions performs these)

1. Build matrix execution for release artifacts after tag push.
2. Release asset creation, checksum generation, and attestation generation.
3. npm workflow execution after release is published.
4. PyPI workflow execution after release is published.
5. Trusted-publishing token exchange through OIDC at publish time.

### If you want “more automatic”

You can remove required reviewers on environments, but that reduces safety.

- With required reviewers: staged/manual-approval publishing.
- Without required reviewers: publish continues automatically after trigger conditions pass.

For this repo, keep required reviewers enabled for `npm`, `testpypi`, and `pypi`.

## 4. GitHub Release Publishing Flow

Triggered by `v*` tag push:

1. Builds target matrix:
   - `x86_64-unknown-linux-gnu`
   - `aarch64-unknown-linux-gnu`
   - `x86_64-unknown-linux-musl`
   - `aarch64-unknown-linux-musl`
   - `x86_64-apple-darwin`
   - `aarch64-apple-darwin`
   - `x86_64-pc-windows-msvc`
   - `aarch64-pc-windows-msvc`
2. Publishes archives named `petiglyph-vX.Y.Z-<target>.(tar.gz|zip)`.
3. Publishes `SHA256SUMS`.
4. Emits artifact attestations.
5. Uses immutable action refs pinned to full commit SHAs.

Post-release checks:

```bash
# verify assets exist
gh release view v0.1.0 --json assets

# optional: download and verify checksums
gh release download v0.1.0 -D ./dist-release
(cd dist-release && sha256sum -c SHA256SUMS)
```

## 5. AUR Release Procedure

Prepare release-grade `PKGBUILD` + `.SRCINFO` from GitHub tag tarball:

```bash
./scripts/release_prepare_aur.sh 0.1.0
```

This script:

- sets immutable `source` URL to `.../archive/refs/tags/v0.1.0.tar.gz`
- computes real `sha256sums`
- enforces `depends=('ffmpeg')`
- regenerates `.SRCINFO`

Validate locally:

```bash
makepkg -sf
namcap PKGBUILD
namcap petiglyph-*.pkg.tar.zst
```

Publish:

```bash
# In your cloned AUR pkgbase repo
cp /path/to/petiglyph/PKGBUILD .
cp /path/to/petiglyph/.SRCINFO .
git add PKGBUILD .SRCINFO
git commit -m "petiglyph 0.1.0"
git push
```

## 6. npm Release Procedure

The release-published event triggers `.github/workflows/npm-publish.yml`.

Order used by workflow:

1. Download GitHub release assets.
2. Verify release integrity (`gh release verify`) and `SHA256SUMS`.
3. Stage binaries into `npm/*/bin` using `scripts/release_stage_npm_artifacts.sh`.
4. Validate `npm pack --dry-run` for each package.
5. Publish platform packages first.
6. Publish `petiglyph` meta package last.

Local preflight:

```bash
# dist-release should contain GitHub release archives
./scripts/release_npm_pack_test.sh dist-release
```

Manual checks after publish:

```bash
npm view petiglyph version
npm view petiglyph bin
npm i -g petiglyph
petiglyph --help
```

## 7. PyPI/TestPyPI Release Procedure

The release-published event triggers `.github/workflows/pypi-publish.yml`.

Flow:

1. Build wheels across configured targets.
   - Linux wheels are built via maturin manylinux container mode (`manylinux: 2014`), including `aarch64-unknown-linux-gnu`, so host cross-linker setup is not required on `ubuntu-latest`.
   - Musllinux wheels are not part of the current workflow.
2. Build sdist.
3. Publish to TestPyPI (environment approval).
4. Publish to PyPI (environment approval).

Local preflight:

```bash
python -m pip install -U maturin twine
maturin build --release --compatibility pypi --sdist
twine check target/wheels/*
```

Install checks:

```bash
python -m venv .venv-testpypi
. .venv-testpypi/bin/activate
pip install --index-url https://test.pypi.org/simple/ --extra-index-url https://pypi.org/simple petiglyph
petiglyph --help
```

## 8. Required Security Controls (Supply Chain)

Apply all of these for each release:

1. Use trusted publishing (OIDC) for npm/PyPI.
2. Keep long-lived publish tokens disabled by default.
3. Enable 2FA/passkeys on GitHub/npm/PyPI/AUR accounts.
4. Gate release jobs behind protected GitHub environments with required reviewers and no self-approval.
5. Publish checksums and verify them before downstream packaging.
6. Keep release tags signed and verified.
7. Keep GitHub Actions permissions minimal (`contents: read` by default, elevate per job only).
8. Pin every workflow action (including `actions/*`) to a full immutable commit SHA.
9. Keep release job isolated: no unrelated secrets in release environments.
10. Document rollback path (deprecate npm versions, yank bad PyPI versions, bump AUR `pkgrel`).

Current workflow implementation status:

- Action refs are pinned to immutable SHAs in all release workflows.
- npm publish flow verifies `gh release verify` and release `SHA256SUMS` before any `npm publish`.
- PyPI publish flow verifies `gh release verify` before the TestPyPI publish gate.

## 9. Platform Runtime Validation Before Announcement

Run these on actual machines (or CI runners) per OS:

### Linux/macOS

```bash
./scripts/clipboard_smoke.sh --bin ./target/release/petiglyph
```

### Windows

```powershell
pwsh -File .\scripts\clipboard_smoke.ps1 -PetiglyphPath .\target\release\petiglyph.exe
```

Release-note policy for now:

- Mark macOS and Windows ARM64 artifacts as limited-runtime-validation/`unstable` unless this release includes direct runtime testing for them.
- Explicitly state Windows/macOS binaries are unsigned and can trigger trust prompts.

## 10. Rollback Quick Actions

- GitHub artifacts: publish a fixed patch release (`vX.Y.(Z+1)`), avoid mutating published assets.
- AUR: fix `PKGBUILD`, increment `pkgrel` if same upstream version.
- npm: publish fixed patch; deprecate broken version.
- PyPI: publish fixed patch; yank bad version if necessary.

## References

- GitHub release events and refs:
  - https://docs.github.com/actions/reference/events-that-trigger-workflows
- GitHub environments/protection rules:
  - https://docs.github.com/actions/deployment/targeting-different-environments
  - https://docs.github.com/en/actions/reference/deployments-and-environments
- GitHub artifact attestations:
  - https://docs.github.com/en/actions/how-tos/secure-your-work/use-artifact-attestations/use-artifact-attestations
  - https://docs.github.com/actions/concepts/security/artifact-attestations
- GitHub source archive URL format:
  - https://docs.github.com/en/repositories/working-with-files/using-files/downloading-source-code-archives
- npm trusted publishing and provenance:
  - https://docs.npmjs.com/trusted-publishers/
- npm package.json (`os`, `cpu`, `libc`, `optionalDependencies`):
  - https://docs.npmjs.com/cli/v11/configuring-npm/package-json/
- npm token and 2FA security:
  - https://docs.npmjs.com/about-access-tokens/
  - https://docs.npmjs.com/creating-and-viewing-access-tokens
  - https://docs.npmjs.com/requiring-2fa-for-package-publishing-and-settings-modification/
- PyPI trusted publishing:
  - https://docs.pypi.org/trusted-publishers/
  - https://docs.pypi.org/trusted-publishers/adding-a-publisher/
  - https://docs.pypi.org/trusted-publishers/using-a-publisher/
  - https://docs.pypi.org/trusted-publishers/security-model/
- PyPI attestations:
  - https://docs.pypi.org/attestations/
  - https://docs.pypi.org/attestations/producing-attestations/
- AUR git/SSH interface details:
  - https://github.com/archlinux/aurweb
  - https://raw.githubusercontent.com/archlinux/aurweb/master/doc/git-interface.txt
- GitHub signed tags:
  - https://docs.github.com/authentication/managing-commit-signature-verification/signing-tags
