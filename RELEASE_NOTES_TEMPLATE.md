# petiglyph v<version>

## Summary

- Version: `<version>`
- Date: `<YYYY-MM-DD>`
- Release type: `<patch|minor|major>`

`<one short paragraph explaining what this release is mainly about>`

## Changes

- `<change 1>`
- `<change 2>`
- `<change 3>`

## Compatibility

- CLI/API breaking changes: `<none|details>`
- Migration required: `<none|details>`

## Distribution

Prebuilt GitHub and npm binaries are provided for:

- Linux GNU on x86-64 and ARM64
- Linux musl on x86-64 and ARM64
- macOS on Intel and Apple Silicon
- Windows on x86-64 and ARM64

PyPI provides Linux GNU manylinux, macOS, and Windows wheels plus a source
distribution. The AUR package requires `ffmpeg` and `fontconfig`.

macOS and Windows artifacts are unsigned unless the release explicitly states
otherwise.
