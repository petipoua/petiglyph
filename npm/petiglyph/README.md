# petiglyph (npm)

This meta package exposes the `petiglyph` CLI by selecting the native optional package for the current platform at runtime.

Supported npm native packages:

- `petiglyph-linux-x64-gnu`
- `petiglyph-linux-arm64-gnu`
- `petiglyph-linux-x64-musl`
- `petiglyph-linux-arm64-musl`
- `petiglyph-darwin-x64`
- `petiglyph-darwin-arm64`
- `petiglyph-win32-x64-msvc`
- `petiglyph-win32-arm64-msvc`

If optional dependencies are disabled or your platform is unsupported, install from GitHub Releases instead:

- https://github.com/petipoua/petiglyph/releases

`ffmpeg` must be available on `PATH` before `petiglyph` starts. It is used for AVIF import conversion and video/animated media workflows.
