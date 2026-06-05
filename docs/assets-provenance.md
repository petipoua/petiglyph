# Assets Provenance

Last updated: 2026-06-05

## Repository Policy

- The repository does not ship third-party logo packs or unknown-provenance media fixtures.
- Public fixtures are stored under `test-assets/` and must be explicitly redistributable.
- Project-local runtime media (`icons/` in each user project) is user-provided and is not part of this repository.

## Current Repository Fixtures

- `test-assets/images/block-16.svg`: maintainer-created, original work.
- `test-assets/images/cross-16.svg`: maintainer-created, original work.
- `test-assets/images/triangle-128.svg`: maintainer-created, original work.
- `test-assets/images/ring-128.svg`: maintainer-created, original work.
- `test-assets/images/diamond-128.svg`: maintainer-created, original work.
- `test-assets/images/*.png`, `*.jpg`, `*.webp`, `*.avif` for the shape set: generated from maintainer-created SVG files.
- `test-assets/videos/*-rotate.(mp4|webm|gif|mkv|mov)`: generated from synthetic in-repo render pipelines (shape motion, max 60 frames). The ring fixture moves around the frame instead of rotating in place.
- `test-assets/frames/rotate-*.png`: extracted 60-frame PNG sequence from `test-assets/videos/triangle-rotate.mp4`.

License for all files above: treated as repository content under the project MIT license unless stated otherwise.
