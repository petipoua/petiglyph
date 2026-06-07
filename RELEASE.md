# Release

This is the canonical runbook for publishing `petiglyph` to GitHub Releases,
npm, PyPI/TestPyPI, and the AUR.

`Cargo.toml` is the release version source of truth. Use `X.Y.Z` below without
the leading `v`; Git tags use `vX.Y.Z`.

## One-Time Setup

- Keep release tag creation restricted; `main` itself does not need push protection for this small single-maintainer repo.
- Require approval for the GitHub environments `npm`, `testpypi`, and `pypi`.
- Configure npm trusted publishing for `.github/workflows/npm-publish.yml`.
- Configure TestPyPI and PyPI trusted publishing for
  `.github/workflows/pypi-publish.yml`, using their matching environments.
- Configure AUR SSH access and own the `petiglyph` package base.
- Keep 2FA or passkeys enabled for all publishing accounts.

Release workflows use OIDC. Do not add long-lived registry tokens unless an
explicit emergency procedure requires them.

## Release Flow

The process has two deliberate triggers:

1. Pushing `vX.Y.Z` builds eight GitHub release archives and creates a draft
   release with checksums and attestations.
2. Publishing that GitHub Release starts the npm and PyPI workflows.

The AUR remains a separate manual publication.

## 1. Prepare

Start from the release commit on `main` with no unrelated working-tree changes.

```bash
./scripts/release_sync_versions.sh X.Y.Z
./scripts/generate_third_party_licenses.sh
```

Review the resulting changes. The version sync updates `Cargo.toml`, `PKGBUILD`,
`.SRCINFO` when `makepkg` is available, all npm package versions and pins, and
the README JSON version sample. Commit any generated
`THIRD_PARTY_LICENSES.md` change with the release preparation.

Run the canonical local CI preflight:

```bash
./scripts/pf.sh
```

The preflight supports uncommitted changes and verifies the Rust checks, Cargo
package contents, distribution matrix, runtime smoke tests, all TUI E2E
journeys, and supply-chain checks.

Commit the release preparation, then verify release-only tree hygiene:

```bash
./scripts/release_assert_clean_tree.sh
```

This final check requires a clean tree and also rejects staged npm binaries and
stray package artifacts.

For platform runtime validation:

```bash
./scripts/clipboard_smoke.sh --bin ./target/release/petiglyph
```

On Windows:

```powershell
pwsh -File .\scripts\clipboard_smoke.ps1 -PetiglyphPath .\target\release\petiglyph.exe
```

## 2. Tag And Build

Create a signed annotated tag on the verified release commit:

```bash
git tag -s vX.Y.Z -m "petiglyph vX.Y.Z"
git push origin vX.Y.Z
```

`.github/workflows/release.yml` then:

- builds Linux GNU, Linux musl, macOS, and Windows archives for x86-64 and
  ARM64;
- smoke-tests each archive;
- generates `SHA256SUMS` and artifact attestations;
- creates a draft GitHub Release using `RELEASE_NOTES_TEMPLATE.md`.

Wait for the workflow to succeed, then verify the draft:

```bash
gh release view vX.Y.Z --json assets,isDraft
rm -rf dist-release
gh release download vX.Y.Z -D dist-release
(cd dist-release && sha256sum -c SHA256SUMS)
gh release verify vX.Y.Z
```

Review and complete the release notes. State any limited runtime validation and
note that macOS and Windows binaries are unsigned when that remains true.

Do not publish the GitHub Release until registry publication should begin.

## 3. Publish Registries

Publishing the draft GitHub Release triggers both registry workflows.

### npm

`.github/workflows/npm-publish.yml` downloads the published release archives,
verifies the GitHub artifact attestations and checksums, stages binaries from
the release archives, validates every package with `npm pack --dry-run`,
publishes the `petiglyph-*` platform packages, then publishes the `petiglyph`
meta package.

Approve the `npm` environment when ready. Optional local archive validation:

```bash
./scripts/release_npm_pack_test.sh dist-release
```

### TestPyPI And PyPI

`.github/workflows/pypi-publish.yml` builds Linux GNU manylinux, macOS, and
Windows wheels plus an sdist. Musllinux wheels are not currently published.

Approve `testpypi`, then validate the uploaded package in a clean environment:

```bash
python -m venv .venv-testpypi
. .venv-testpypi/bin/activate
python -m pip install \
  --index-url https://test.pypi.org/simple/ \
  --extra-index-url https://pypi.org/simple \
  petiglyph
petiglyph --help
deactivate
rm -rf .venv-testpypi
```

Approve `pypi` only after that validation succeeds.

## 4. Publish AUR

Prepare immutable tag-based AUR metadata:

```bash
./scripts/release_prepare_aur.sh X.Y.Z
makepkg -sf
namcap PKGBUILD
namcap petiglyph-*.pkg.tar.zst
```

Copy `PKGBUILD` and `.SRCINFO` into the AUR package repository, commit them, and
push. The generated package declares the required `ffmpeg` dependency.

## 5. Verify

Confirm all channels report `X.Y.Z` and launch the CLI:

```bash
gh release view vX.Y.Z
npm view petiglyph version
python -m pip index versions petiglyph
```

Test fresh npm and PyPI installs where practical, and verify the AUR package
page after its push.

## Recovery

Published artifacts should not be replaced in place.

- GitHub: fix the problem in a new patch release.
- npm: deprecate the broken version and publish a patch.
- PyPI: yank the broken version and publish a patch.
- AUR: correct `PKGBUILD`; increment `pkgrel` when the upstream version is
  unchanged.

If a draft release is wrong, delete the draft and tag only when nothing has
been published downstream and the tag correction is intentional.
