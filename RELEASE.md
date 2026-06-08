# Release

This is the canonical runbook for publishing `petiglyph` to GitHub Releases,
npm, PyPI/TestPyPI, and the AUR.

`Cargo.toml` is the release version source of truth. Use `X.Y.Z` below without
the leading `v`; Git tags use `vX.Y.Z`.

## One-Time Setup

- Keep release tag creation restricted; `main` itself does not need push protection for this small single-maintainer repo.
- Require approval for the GitHub environments `npm`, `testpypi`, and `pypi`.
- For the first npm publish, add a temporary npm automation token as the
  `NPM_PUBLISH_TOKEN` secret in the GitHub `npm` environment.
- After the `petiglyph` packages exist on npm, configure trusted publishing for
  `.github/workflows/npm-publish.yml`, for example with
  `./scripts/release_npm_trust.sh`, then remove the token.
- For bootstrap Python publishes, optional repository secrets
  `TEST_PYPI_API_TOKEN` and `PYPI_API_TOKEN` can be added to the matching GitHub
  environments. When absent, the workflow uses trusted publishing via OIDC.
- Configure TestPyPI and PyPI trusted publishing for
  `.github/workflows/pypi-publish.yml`, using their matching environments.
- Configure AUR SSH access and own the `petiglyph` package base.
- Keep 2FA or passkeys enabled for all publishing accounts.

Release workflows prefer OIDC when trusted publishing is configured. Use a
short-lived npm automation token only as the bootstrap path for the first
publish, then remove it once trusted publishing is active. The PyPI workflow
also accepts short-lived API tokens as a bootstrap path when trusted publishing
is not configured yet.

## Release Flow

The process has two deliberate triggers:

1. Pushing `vX.Y.Z` builds eight GitHub release archives and creates a draft
   release with checksums and attestations.
2. Publishing that GitHub Release starts the npm and PyPI workflows.

The AUR remains a separate manual publication.

After the release preparation is committed and all required GitHub environment
approvals are ready, the guarded end-to-end publisher can run the remaining
steps:

```bash
./scripts/release_all.sh vX.Y.Z
```

It tags the current `origin/main` commit, waits for and verifies the draft
GitHub Release, publishes it, waits for npm and PyPI, publishes the AUR package,
and verifies all public versions. Interactive runs pause for the draft notes to
be completed. For intentional non-interactive use, pass both
`--notes-file PATH` and `--yes`; the script rejects unreplaced template
placeholders. The GitHub release event starts npm and PyPI concurrently; the
script waits for both before publishing to the AUR.

The publisher is resumable. Rerunning the same command verifies that the
existing tag still resolves to the current release commit, skips completed
channels, and resumes failed or interrupted GitHub Actions workflows. npm
publication is idempotent per platform package, PyPI uses `skip-existing`, and
an already-matching AUR upstream version is left unchanged.

Published npm and PyPI files are immutable. A release-note typo can be corrected
by rerunning with `--notes-file`, but any source or binary correction requires a
new version and tag. Never move an existing published tag.

The tag is the cross-channel code identity. npm stages the exact verified
GitHub Release binaries; PyPI wheels and the AUR package are rebuilt from that
same immutable tag because their distribution formats require separate builds.

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
meta package. If the `NPM_PUBLISH_TOKEN` secret exists in the GitHub `npm`
environment, the workflow uses that token for bootstrap publishing; otherwise
it uses npm trusted publishing via OIDC.

Approve the `npm` environment when ready. Optional local archive validation:

```bash
./scripts/release_npm_pack_test.sh dist-release
```

### TestPyPI And PyPI

`.github/workflows/pypi-publish.yml` builds Linux GNU manylinux, macOS, and
Windows wheels plus an sdist. Musllinux wheels are not currently published.
If `TEST_PYPI_API_TOKEN` or `PYPI_API_TOKEN` are present in the corresponding
GitHub environments, the workflow uses those tokens; otherwise it uses trusted
publishing.

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

Then either copy `PKGBUILD` and `.SRCINFO` into the separate AUR package
repository, commit them, and push, or use the local publish helper:

```bash
./scripts/release_publish_aur.sh X.Y.Z
```

The helper regenerates release-ready AUR metadata, clones the AUR repo into
`../petiglyph-aur` when needed, copies `PKGBUILD` and `.SRCINFO`, commits when
the packaging changed, and pushes over your local AUR SSH key. The generated
package declares the required `ffmpeg` dependency.

For packaging-only AUR changes on the same upstream release, keep `pkgver`
unchanged and increment `pkgrel`:

```bash
./scripts/release_publish_aur.sh X.Y.Z --pkgrel 2
```

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
