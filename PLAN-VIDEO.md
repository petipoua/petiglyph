# PLAN-VIDEO.md

## Goal
Add support for dropping/pasting **video files and GIF files** into the Home popup **DRAG/PASTE area** for both animated creation workflows:

- `Create animated glyph` (`AnimationType::Standard`)
- `Create animated grid glyph` (`AnimationType::Grid`)

The workflow must automatically split media into frame images and continue through the existing animation creation flow (selection/config/persist/build/install) without breaking current CLI/TUI contracts.

## Scope

### In Scope
- Home workflow import step for animated types accepts still images + GIF + video files.
- GIF/video inputs are expanded into per-frame image files inside project `icons/`.
- Expanded frames are auto-selected as animation draft frames (existing behavior for imported images remains).
- Existing animation config UI, manifest persistence, preview, and build/install pipeline continue to work with generated frame files.
- Clear user-facing status/errors for extraction failures and unsupported media.
- Tests for parsing, extraction, import accounting, ordering, and workflow behavior.
- README and packaging/runtime dependency updates.

### Out of Scope (for this phase)
- Per-frame variable timing in manifest (current model remains `fps` only).
- Audio handling.
- New standalone CLI command for video/gif import.
- Global support for video files as normal glyph sources in `build` (video remains animation-import-only).

## Current State (Codebase Facts)

1. Animated workflow import already runs in background:
- `App::import_dropped_images` routes animated workflows into `start_animation_frame_import`.
- Worker currently calls `import_image_files_to_input(..., ExistingImportPolicy::ReuseIdentical)`.
- Relevant code: `src/tui.rs` around `4669-4768`, `5066-5133`.

2. Drop importer only copies files with `is_supported_source(path)`:
- `import_image_files_to_input` filters by `is_supported_source`.
- Relevant code: `src/tui.rs` around `9200-9282`.

3. Supported sources are image extensions only (including `gif`):
- `build::is_supported_source`: `png jpg jpeg webp avif bmp gif svg`.
- Relevant code: `src/build.rs` around `890-901`.

4. Image decoding path treats `gif` as a single decoded image for preprocessing:
- `image_pipeline::load_source_rgba` uses `image::open` for non-SVG.
- Relevant code: `src/image_pipeline.rs` around `350-363`.

5. Animated Home popup/UI copy currently says “images” only:
- Placeholder labels and statuses use `IMAGES` wording.
- Relevant code: `src/tui.rs` around `7007`, `7217`, `7304`, `9435-9462`.

6. Animation persistence model is already source-key frame list (`frames: Vec<String>`), shared by standard/grid:
- No schema blocker for generated PNG frame keys.
- Relevant code: `src/project.rs` (`AnimationDef`, validation rules).

7. Packaging currently has no runtime dependencies declared in `PKGBUILD`:
- `depends=()`.

## Design Decisions

### D1: Keep `build` source support unchanged
Do **not** add video extensions to `build::is_supported_source`. Video files should not become regular glyph sources. This avoids breaking build scanning and avoids non-image decode failures.

### D2: Add a dedicated animation-media import pipeline
Keep generic image drop import logic as-is for non-animated workflows. Add a new animation-specific importer that can:
- accept still images,
- expand GIFs to frames,
- expand videos to frames,
- then copy/reuse generated frame files into `icons/`.

### D3: GIF and video become frame PNG sequences in `icons/`
Generated filenames should be deterministic and conflict-safe, so re-importing the same media reuses frames where possible.

### D4: Use `ffmpeg`/`ffprobe` for video extraction (and optionally GIF fallback)
Given project style already shells out to platform tools (`fc-cache`, `fc-match`), video decoding via `ffmpeg` is pragmatic and robust. Keep GIF extraction in-Rust if straightforward, otherwise use ffmpeg for parity.

Recommended split:
- GIF: in-Rust (`image::codecs::gif::GifDecoder` + `AnimationDecoder`) to avoid external dependency for GIF-only usage.
- Video: `ffmpeg` subprocess extraction.

### D5: Preserve current animation timing model
Do not auto-map source media timing to animation fps. Keep default `fps=8` and user-controlled config UI unchanged.

## Implementation Plan

## Phase 1: Add Animation Media Import Abstractions

1. Create new module `src/animation_media.rs`.
2. Add `mod animation_media;` in `src/main.rs`.
3. Define core types:
- `enum AnimationInputKind { StillImage, Gif, Video, Unsupported }`
- `struct AnimationMediaImportResult` with counters and `imported_source_keys`.
- `struct ExpandedFrame { source_key: String, origin: OriginMeta }` (origin metadata internal only).
4. Add extension classifiers:
- Still image: existing supported image extensions except GIF if treated separately.
- GIF: `gif`.
- Video: initial allowlist `mp4, mov, mkv, webm, avi, m4v`.

## Phase 2: Implement Frame Expansion Engine

1. Add function:
- `import_animation_media_to_input(input_dir, payload, existing_policy) -> Result<AnimationMediaImportResult>`

2. Reuse existing path parsing:
- keep `collect_dropped_paths`, `normalize_dropped_path_candidate` logic from `tui.rs`.
- refactor these helpers into shared location or expose minimal shared utility.

3. For each dropped path:
- if missing/non-file -> `skipped_missing += 1`.
- classify input kind.
- if unsupported -> `skipped_unsupported += 1`.
- if still image -> route through current copy/reuse path.
- if GIF/video -> expand frames to temporary RGBA/PNG files, then import those PNGs.

4. Temporary extraction strategy:
- create per-input temp dir (`std::env::temp_dir()/petiglyph-frames-<pid>-<nonce>` or `tempfile`).
- cleanup on success/failure.

5. GIF extraction (preferred in-Rust path):
- decode frames in display order.
- render each frame to RGBA honoring disposal/composition as provided by decoder.
- write PNGs as temporary files.

6. Video extraction (ffmpeg path):
- preflight `ffmpeg` availability.
- run command that writes one PNG per decoded frame in deterministic sequence order.
- collect outputs sorted lexicographically.
- treat zero output frames as explicit error.

7. File naming strategy in `icons/`:
- deterministic frame names from media stem + stable media hash + frame index.
- example format: `<slug_stem>--pgf-<hash8>-f000001.png`.
- hash seed: source file bytes hash (or size+mtime+path hash if performance concerns).
- use existing collision handling (`next_available_import_destination`) when needed.

8. Reimport behavior:
- Keep `ExistingImportPolicy::ReuseIdentical` for animated workflow.
- If deterministic name already exists and bytes match, reuse key and still include it in `imported_source_keys` so selection state remains correct.

## Phase 3: Wire Animated Workflow to New Importer

1. In `App::start_animation_frame_import` worker (`src/tui.rs`), replace call to `import_image_files_to_input` with new `import_animation_media_to_input`.
2. Keep loading glyphs after imports exactly as now (`load_interactive_glyphs_from_config`).
3. Ensure `finish_animation_import` continues to:
- add all returned `imported_source_keys` to selection order/set,
- increment `home_workflow_import_count` by selected frame count.

## Phase 4: UX and Messaging Updates

1. Update animated workflow copy from “images” to “images/GIFs/videos” where relevant:
- Home launcher status messages for keys `3` and `4`.
- Popup step text (`Import frame images`).
- Popup instruction line (`drop, paste, or drag files`).
- Drag area label (`DRAG/PASTE MEDIA HERE` or equivalent).
- Empty-state/status hints currently saying “frame image”.

2. Keep non-animated flows phrasing image-specific.

3. Add specific failure statuses:
- ffmpeg missing.
- media decode failed.
- media had zero extractable frames.
- extraction aborted due to frame limit.

4. Add richer import status for animation media:
- media files processed,
- frames extracted,
- frames added/reused/renamed,
- unsupported/missing counts.

## Phase 5: Safety and Performance Controls

1. Add configurable hard limits (constants):
- max frames per media input (e.g., 1200 default).
- max total extracted frames per import action (e.g., 3000 default).
- optional max decoded dimension guard.

2. If limits exceeded:
- stop extraction for offending media,
- surface actionable error/status,
- keep already-imported frames from previous files in the same drop payload.

3. Keep background thread behavior for responsiveness (already in place).

## Phase 6: Testing Plan

### Unit Tests (`src/tui.rs` tests and/or new module tests)
1. Classifier tests:
- still image, gif, video, unsupported extensions.

2. Deterministic frame naming tests:
- same media input -> same planned output names.
- different media same basename -> distinct hash namespace.

3. Import accounting tests:
- dropped video expands to N frame source keys.
- dropped gif expands to N frame source keys.
- mixed payload (image+gif+video+unsupported+missing).

4. Reimport idempotency tests:
- importing same media twice in animated workflow reuses existing files and preserves selection counts.

5. Error tests:
- ffmpeg missing returns clear message.
- decode failure returns clear message.
- zero frames extracted surfaces explicit error.

### Existing Behavior Regression Tests
1. Keep current tests passing for:
- non-animated image drop import and rename behavior,
- animated import task lifecycle,
- natural sorting before animation persist,
- workflow cancellation and configuration.

2. Update wording assertions impacted by string changes (drag placeholder and status messages).

### Integration / E2E
1. Add at least one headless integration test (or guarded test) for GIF frame expansion.
2. Add one guarded test for video extraction requiring ffmpeg.
3. Extend `scripts/tui_e2e_hty.sh` with an animated workflow journey:
- drop one small video fixture,
- wait for import completion,
- create animation,
- verify manifest animation frames > 1 and generated frame files exist.

## Phase 7: Documentation and Packaging

1. Update `README.md`:
- Home creation workflow text for animated media import.
- Notes section to distinguish:
  - global glyph source inputs (images),
  - animated workflow accepted media inputs (images + gif + video).
- Add note that video extraction depends on `ffmpeg` availability.

2. Update `PKGBUILD`:
- add runtime dependency for ffmpeg if video support is shipped through ffmpeg CLI.

3. Optional: add troubleshooting subsection:
- “Video import says ffmpeg not found”.
- “Media imported but 0 frames extracted”.

## File-Level Change Map

- `src/main.rs`
  - register new module.

- `src/animation_media.rs` (new)
  - media classification,
  - gif/video frame extraction,
  - deterministic frame naming,
  - copy/reuse integration and result accounting.

- `src/tui.rs`
  - swap animated importer call,
  - status/error text updates,
  - popup/drag labels updates,
  - optional small helper refactors for shared dropped-path parsing.

- `src/tests.rs` and `src/tui.rs` test module
  - new/updated tests for media import and wording.

- `README.md`
  - behavior docs.

- `PKGBUILD`
  - ffmpeg runtime dependency.

## Backward Compatibility and Data Model Impact

- No manifest schema change required.
- Existing `animations` entries remain valid.
- Existing projects keep working unchanged.
- New behavior only affects animated workflow imports.

## Risk Register

1. **Frame explosion / huge media**
- Mitigation: explicit frame limits and clear error.

2. **ffmpeg not installed on some systems**
- Mitigation: preflight check + actionable error + README/PKGBUILD dependency.

3. **Duplicate/unstable frame naming causing clutter**
- Mitigation: deterministic names with hash namespace + reuse-identical policy.

4. **GIF timing mismatch vs current constant FPS model**
- Mitigation: explicitly keep current model; user sets FPS manually.

5. **UX confusion between generic image import and animated media import**
- Mitigation: context-specific messaging in animated workflow only.

## Acceptance Criteria

1. In Home popup animated workflows, dropping/pasting:
- one `.gif` with multiple frames results in multiple selected draft frames.
- one video file results in multiple selected draft frames.
- mixed image+gif+video payload results in combined selected frame set.

2. Created standard/grid animation persists correctly in `petiglyph.toml` and previews/builds without regression.

3. Non-animated image import behavior remains unchanged.

4. Clear user-facing error when ffmpeg is unavailable for video extraction.

5. All existing tests pass plus new media-import tests.

## Suggested Delivery Order

1. Add new module + unit tests for classification and naming.
2. Implement GIF expansion path + tests.
3. Implement ffmpeg video expansion path + tests.
4. Wire TUI animated import to new module.
5. Update UX strings and status formatting.
6. Add/adjust E2E journey.
7. Update README + PKGBUILD.
8. Final regression run (`cargo test` + selected hty journey).
