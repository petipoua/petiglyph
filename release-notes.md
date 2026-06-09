# petiglyph next patch release

## Summary

- Version: `TBD (next patch after 0.1.4)`
- Date: `2026-06-09`
- Release type: patch

This patch release focuses on Windows font installation reliability. Installed
fonts are now registered in the current-user Windows Fonts registry and loaded
through the Windows font APIs so they behave like real user-installed fonts
instead of only being copied into petiglyph-managed storage. The release also
documents the Windows Terminal fallback-family setup needed to render petiglyph
glyphs alongside a primary terminal font.

## Changes

- Windows font install now writes the installed family into
  `HKCU\\Software\\Microsoft\\Windows NT\\CurrentVersion\\Fonts` and registers
  the TTF with `AddFontResourceExW`, making the font discoverable as a
  user-installed Windows font.
- Windows font uninstall now removes the loaded font resource and deletes the
  matching current-user Fonts registry entry when present.
- Added README guidance for Windows Terminal 1.21+ to configure a petiglyph
  family as a fallback font after the primary terminal font, for example
  `"Cascadia Mono, my-font Petiglyph"`.

## Compatibility

- CLI/API breaking changes: none
- Migration required: none

## Distribution

Prebuilt GitHub and npm binaries are provided for:

- Linux GNU on x86-64 and ARM64
- Linux musl on x86-64 and ARM64
- macOS on Intel and Apple Silicon
- Windows on x86-64 and ARM64

PyPI provides Linux GNU manylinux, macOS, and Windows wheels plus a source
distribution. The AUR package requires `ffmpeg` and `fontconfig`.

macOS and Windows artifacts are unsigned.
