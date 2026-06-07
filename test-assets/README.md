# Test Assets

Purpose: small repository fixtures for manual testing and future test wiring.

Policy:
- Assets here must be safe to redistribute publicly.
- Prefer maintainer-created assets or clearly permissive licenses (CC0, public domain, or equivalent permissive terms).
- Do not add trademarked logos or unknown-provenance media.

Current provenance:
- `images/*.svg`: created in-repo by maintainers for test use.
- converted stills (`.png`, `.jpg`, `.webp`, `.avif`): generated from maintainer-created SVG sources.
- `videos/*-rotate.(mp4|webm|gif|mkv|mov)`: generated in-repo from synthetic shape render pipelines. The ring fixture moves around the frame instead of rotating in place, because a rotating ring is visually unchanged.

Notes:
- This folder is intentionally not wired into code paths yet.
- Project runtime still expects per-project source media in `images/` inside each petiglyph project.
- `frames/rotate-*.png` is a 60-frame extracted sequence from `videos/triangle-rotate.mp4` and is the canonical frame-sequence fixture.
