# @petiglyph/petiglyph-linux-arm64-gnu

petiglyph native binary package for linux/arm64 (glibc).

This package is installed as an optional dependency of the `petiglyph` npm meta package. It contains only the native `petiglyph` binary for its target platform plus package metadata.

Use the meta package unless you are testing platform packaging directly:

```bash
npm install -g petiglyph
petiglyph --help
```

`ffmpeg` is required separately for video and animated media import workflows.
