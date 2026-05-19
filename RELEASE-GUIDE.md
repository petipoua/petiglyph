# RELEASE-GUIDE

Last updated: 2026-05-19

This guide is the operator runbook for publishing `petiglyph` to:

- GitHub Releases (canonical binaries)
- AUR (`petiglyph` source package)
- npm (`petiglyph` meta package + `@petiglyph/*` platform packages)
- PyPI (maturin `bin` wheels + sdist)

It complements `RELEASE-CHECKLIST.md` and reflects the release automation now in this repository.

## 1. What Is Implemented In Repo

- GitHub artifact release workflow: `.github/workflows/release.yml`
  - Builds 8 targets.
  - Packages archives per target.
  - Generates `SHA256SUMS`.
  - Publishes release assets.
  - Generates GitHub artifact attestations.
- npm publish workflow: `.github/workflows/npm-publish.yml`
  - Trigger: GitHub release `published`.
  - Downloads release archives.
  - Stages native binaries into `npm/*` packages.
  - Publishes platform packages, then meta package.
- PyPI/TestPyPI publish workflow: `.github/workflows/pypi-publish.yml`
  - Trigger: GitHub release `published`.
  - Builds wheels/sdist with maturin.
  - Publishes to TestPyPI then PyPI using trusted publishing.
- Packaging/release helper scripts:
  - `scripts/release_sync_versions.sh`
  - `scripts/release_stage_npm_artifacts.sh`
  - `scripts/release_npm_pack_test.sh`
  - `scripts/release_prepare_aur.sh`
- PyPI metadata: `pyproject.toml`
- npm package layout: `npm/` (meta + 8 platform packages)
- License file: `LICENSE`

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
4. Create signed annotated tag:

```bash
git tag -s v0.1.0 -m "petiglyph v0.1.0"
git push origin v0.1.0
```

5. The tag push triggers `.github/workflows/release.yml`.

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
2. Stage binaries into `npm/*/bin` using `scripts/release_stage_npm_artifacts.sh`.
3. Validate `npm pack --dry-run` for each package.
4. Publish platform packages first.
5. Publish `petiglyph` meta package last.

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
8. Pin third-party actions to a specific commit SHA before production hardening (current workflow still uses major tags).
9. Keep release job isolated: no unrelated secrets in release environments.
10. Document rollback path (deprecate npm versions, yank bad PyPI versions, bump AUR `pkgrel`).

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

- Mark macOS and Windows ARM64 artifacts as `unstable` until runtime-tested.
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
