# RELEASE-CHECKLIST

Purpose: short execution checklist for shipping `petiglyph` through GitHub Releases, AUR, npm, and PyPI/TestPyPI.

Canonical runbook: [RELEASE-GUIDE.md](RELEASE-GUIDE.md)

This checklist reflects repository state as of 2026-06-01. `Cargo.toml` package version is the release source of truth.

## 1. Version + Tree Prep

- [ ] Choose version `X.Y.Z`.
- [ ] Run version sync:

```bash
./scripts/release_sync_versions.sh X.Y.Z
```

- [ ] Verify synchronized metadata:

```bash
cargo metadata --no-deps --format-version 1 | jq -r '.packages[] | select(.name == "petiglyph") | .version'
rg -n '^(pkgver=|\s*"version":|version = )' Cargo.toml PKGBUILD npm/*/package.json
```

- [ ] Commit release-prep changes.
- [ ] Ensure clean/packageable tree:

```bash
./scripts/release_assert_clean_tree.sh
```

## 2. Local Quality Gates

- [ ] Run core gates:

```bash
cargo fmt --check
cargo check --locked
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked
cargo deny check
cargo audit
cargo tree --locked -e normal
./scripts/generate_third_party_licenses.sh
cargo run -- --help
cargo run -- tui </dev/null
```

- [ ] If `docs/THIRD_PARTY_LICENSES.md` changed, commit it in the release prep commit.

- [ ] Use pinned Rust toolchain from `rust-toolchain.toml` (`1.88.0`, with `clippy` and `rustfmt`).
- [ ] Smoke scratch project build/install/uninstall/doctor JSON flow (see runbook).

## 3. TUI + Runtime Smoke

- [ ] Validate `hty` CLI and run E2E harness:

```bash
hty --help
./scripts/tui_e2e_hty.sh
```

- [ ] Run runtime smoke scripts:

```bash
./scripts/clipboard_smoke.sh
pwsh -File .\scripts\clipboard_smoke.ps1
```

## 4. GitHub Release Artifacts

- [ ] Create and push signed tag:

```bash
git tag -s vX.Y.Z -m "petiglyph vX.Y.Z"
git push origin vX.Y.Z
```

- [ ] Wait for `.github/workflows/release.yml` success.
- [ ] Verify release assets + checksums + attestations:

```bash
gh release view vX.Y.Z --json assets,isDraft
gh release download vX.Y.Z -D ./dist-release
(cd dist-release && sha256sum -c SHA256SUMS)
gh release verify vX.Y.Z
```

- [ ] Keep release as draft until artifact review is complete.

## 5. AUR Publish (Manual)

- [ ] Prepare release PKGBUILD/SRCINFO:

```bash
./scripts/release_prepare_aur.sh X.Y.Z
```

- [ ] Validate locally:

```bash
makepkg -sf
namcap PKGBUILD
namcap petiglyph-*.pkg.tar.zst
```

- [ ] Push updated `PKGBUILD` + `.SRCINFO` to AUR repo.

## 6. Registry Publish (GitHub Release Published Event)

- [ ] Publish GitHub Release only when ready for npm/PyPI workflows to run.
- [ ] Approve environment gates when prompted (`npm`, `testpypi`, `pypi`).
- [ ] For PyPI flow, run TestPyPI install validation before approving `pypi`.

## 7. Post-Release Sanity

- [ ] Verify npm package visibility and basic install (`npm view`, `petiglyph --help`).
- [ ] Verify TestPyPI/PyPI install path and CLI startup.
- [ ] Confirm release notes include unsigned macOS/Windows warning and ARM64 runtime-validation status.

## 8. Rollback Quick Path

- [ ] GitHub assets issue: publish fixed patch release `vX.Y.(Z+1)`.
- [ ] AUR issue: fix `PKGBUILD`, bump `pkgrel` for same upstream if needed.
- [ ] npm issue: publish fixed patch, deprecate broken version.
- [ ] PyPI issue: publish fixed patch, yank broken release if required.
