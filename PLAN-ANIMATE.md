# Animate Glyph Feature Implementation Plan (Refined)

## Goal

Enable users to create and preview animated glyph sequences from existing project-local images in `icons/`, directly from the TUI, without regressing current composition, build, or codepoint behavior.

## Current Status (as of 2026-05-11)

Implemented today:

- `Animate Glyph` exists in the Glyphs UI as a focusable button.
- Pressing `Enter` on `Animate Glyph` shows a placeholder status message:
  `Animate Glyph is planned for Glyphs tools but is not implemented yet`.
- Placeholder behavior is unit-tested in `src/tests.rs` (`glyph_view_animate_button_runs_placeholder_action`).
- Manifest/runtime schema currently supports compositions and threshold overrides, but not animations.

Not implemented yet (this plan covers these):

- Animation schema in `petiglyph.toml`.
- Animation creation/import/configuration flow in TUI.
- Animation playback in preview.
- Animation deletion flow.
- Animation-specific CLI/contract/E2E coverage.

Animation modes:

1. Standard animation: one source image per frame.
2. Grid animation: one source image per frame, each frame split as `rows x cols`.

Both modes must be available from the same Glyphs view `Animate Glyph` tool. The
first choice after pressing `Animate Glyph` is the animation kind, so users do not
select frames before the TUI knows whether it is collecting normal glyph frames
or grid-compatible frame sources.

Primary preview target:

- Glyphs view Preview panel.

Secondary preview target (phase 6):

- Home panel Current project card.

---

## 0. Scope and Non-goals

### In scope

- Manifest schema for animations.
- TUI creation flow (choose type -> drag/paste animation frames -> configure -> persist).
- Glyphs-panel animation playback.
- Safe animation deletion flow.
- Tests at unit, contract, and hty E2E levels.

### User story

As a font author working in the Glyphs panel, I want to press `Animate Glyph` and
first choose whether I am creating a `Standard` animation or a `Grid` animation,
so that the rest of the flow only asks for inputs relevant to the glyph type I am
animating.

Standard glyph animation story:

1. User presses `Animate Glyph`.
2. TUI shows an animation-kind picker with `Standard glyph animation` and `Grid glyph animation`.
3. User chooses `Standard glyph animation`.
4. TUI shows an animation-specific `DRAG/PASTE IMAGES HERE` box.
5. User drags or pastes the frame images for this animation.
6. TUI imports those images into the active project `icons/` folder and adds only
   those imported parent source keys to the animation draft.
7. User confirms, names the animation, sets FPS, and creates it.
8. Preview cycles through the selected standard glyph frames.

Grid glyph animation story:

1. User presses `Animate Glyph`.
2. TUI shows the same animation-kind picker.
3. User chooses `Grid glyph animation`.
4. TUI shows the same animation-specific `DRAG/PASTE IMAGES HERE` box, labeled
   for grid animation frames.
5. User drags or pastes the grid frame images for this animation.
6. TUI imports those images into the active project `icons/` folder and adds only
   those imported parent source keys to the animation draft.
7. User confirms, names the animation, sets FPS, and configures grid options:
   rows, cols, left/right bleed, and top/bottom bleed.
8. On create, each frame source is guaranteed to have a matching grid composition
   definition with the selected rows, cols, and bleed settings, or creation fails
   with a specific mismatch message.
9. Preview cycles frame-by-frame while rendering each frame through the existing
   grid composition preview path.

This keeps the feature applicable to both standard glyphs and grid glyphs while
preventing ambiguous mixed-mode selections.

### Not in scope for initial rollout

- New CLI subcommands for animation CRUD.
- Reworking codepoint allocation logic.
- Broad changes to `generate_smart_sample` formatting unless tests prove a gap.

---

## 1. Baseline Constraints from Current Code

- `AnimateButton` exists but is placeholder-only in key handling (`src/tui.rs`).
- Composition pipeline is already stable and keyed by parent source key (`source_parent_key`) while composition children use synthetic `source_key` with `#compose:...` suffix (`src/build.rs`).
- `load_runtime_config` currently performs strict schema/range checks and is used by CLI, TUI, build, install, and doctor (`src/project.rs`).
- `Manifest` and `RuntimeConfig` currently do not include animation fields (`src/project.rs`).
- Home panel currently renders project status and drag-area placeholder, not animation state (`src/tui.rs`).

Implication: frame references in animation manifest must use parent source keys (file-level keys), never tile `#compose` keys.

---

## 2. Data Model and Manifest

### 2.1 New types (`src/project.rs`)

```rust
// Reuses the existing BleedLevel type used by CompositionDef.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct AnimationDef {
    pub(crate) name: String,
    #[serde(rename = "type")]
    pub(crate) animation_type: AnimationType,
    pub(crate) fps: u8,
    pub(crate) frames: Vec<String>, // parent source keys only
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) rows: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) cols: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) horizontal_bleed: Option<BleedLevel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) vertical_bleed: Option<BleedLevel>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AnimationType {
    Standard,
    Grid,
}
```

### 2.2 Manifest additions

- Add to `Manifest`:
  - `animations: Vec<AnimationDef>` with `#[serde(default, skip_serializing_if = "Vec::is_empty")]`.
- Add to `RuntimeConfig`:
  - `animations: Vec<AnimationDef>`.

### 2.3 Validation policy

Hard validation in `load_runtime_config`:

- `name.trim()` must be non-empty.
- animation names must be unique.
- `fps` must be `1..=30`.
- `frames` must be non-empty.
- frame keys must be non-empty strings.
- `Grid` requires `rows` and `cols` both present and `> 0`.
- `Grid` may carry `horizontal_bleed` and `vertical_bleed`; omitted values use
  the same defaults as grid composition (`horizontal_bleed = weak`,
  `vertical_bleed = off`).
- If `Grid` references existing compositions, the effective rows, cols,
  horizontal bleed, and vertical bleed must match the animation's grid settings
  during create flow.
- `Standard` must omit `rows`/`cols`; if present, reject with a specific
  validation error.
- `Standard` must omit bleed fields; if present, reject with a specific
  validation error.

Soft runtime validation (non-fatal):

- missing frame files are handled by consumers, not by `load_runtime_config`.
- in TUI preview, missing frames are skipped with a status hint and the
  animation remains editable.
- build/install flows keep their existing deterministic failure behavior when a
  required input is missing.
- doctor reports missing animation frame files as diagnostics (warning/error by
  mode), without changing manifest schema validation behavior.

Reason: keep CLI/TUI startup resilient when files are moved temporarily.

---

## 3. Interaction State Model (TUI)

Replace additive boolean mode growth with a single explicit mode enum.

### 3.1 New state (`src/tui.rs`)

```rust
enum GlyphToolMode {
    None,
    ChooseAnimationType,
    ImportAnimationFrames(AnimationType),
    SelectAnimationFrames(AnimationType),
    ConfigureGrid(GridConfig),
    ConfigureAnimation(AnimationConfig),
}
```

`App` additions:

- `glyph_tool_mode: GlyphToolMode`
- `animation_selection_order: Vec<String>` (parent source keys in explicit draft order)
- `animation_selection_set: BTreeSet<String>` (membership/dedup helper only)
- `animation_import_target: Option<AnimationType>`
- `animation_preview: Option<AnimationPreview>`

`AnimationConfig`:

- `selected_frames: Vec<String>` preserving explicit draft order
- `name_input: Input`
- `animation_type: AnimationType`
- `fps: u8`
- `rows: u32`
- `cols: u32`
- `horizontal_bleed: BleedLevel`
- `vertical_bleed: BleedLevel`
- `focus: AnimationConfigFocus`

`AnimationPreview`:

- `animation_name: String`
- `frame_index: usize`
- `last_frame_at: Instant`

### 3.2 Why this model

- Centralized routing avoids edge-cases from `selecting_for_grid` + `grid_config` + new booleans.
- Keeps `handle_glyphs_key` predictable and testable.

---

## 4. TUI Creation Flow

1. User focuses `Animate Glyph` tool.
2. Enter `GlyphToolMode::ChooseAnimationType`.
3. User chooses `Standard` or `Grid`.
4. Enter `GlyphToolMode::ImportAnimationFrames(selected_type)`.
5. The right-side panel shows an animation-specific `DRAG/PASTE IMAGES HERE` box
   and status text explaining that imported files will be used as frames for this
   animation draft.
6. User drags files onto the terminal or pastes image paths while this mode is active.
   - Imported files are copied into the current project's `icons/` directory using
     the same safety rules as the existing image intake path.
   - Imported parent source keys are appended to `animation_selection_order` if
     not already selected, and reflected in `animation_selection_set`.
   - Import order should be stable and visible.
   - Persisted frame order follows explicit draft order (import/selection order
     as shown to the user, from `animation_selection_order`).
   - If input order cannot be inferred from an intake source, fallback ordering
     is alphabetical by parent source key for deterministic behavior.
7. The animation import box is scoped to `GlyphToolMode::ImportAnimationFrames`.
   It must not consume Home panel drag/paste events unless the active view is
   Glyphs and the active mode is an animation import mode.
8. User can optionally enter `GlyphToolMode::SelectAnimationFrames(selected_type)`
   to add existing project-local images from the glyph list.
9. In glyph list, `Space` toggles selected parent source key.
   - For Standard: selectable rows are normal parent source keys.
   - For Grid: selectable rows are parent source keys that can be split into the
     configured grid, including existing grid parents and new frame sources that
     will receive composition definitions on create.
   - Composition child rows are never selectable as animation frames.
10. `Enter` with non-empty selection opens `ConfigureAnimation` with
    import/selection-ordered frames (or alphabetical fallback order when
    unavailable).
11. User sets:
   - Name (default: common stem fallback to `animation_<n>`).
   - FPS (`1..=30`, default `8`).
   - Rows/Cols only when Grid (default `2x2`).
   - Left/right bleed only when Grid (default `weak`, matching grid creation).
   - Top/bottom bleed only when Grid (default `off`, matching grid creation).
12. On Create:
   - Persist new `AnimationDef` to `manifest.animations`.
   - For Grid: persist selected rows, cols, and bleed settings on the animation
     definition, then ensure each frame has a composition definition.
     - If composition missing: add it.
     - If present and matches rows, cols, horizontal bleed, and vertical bleed: keep.
     - If present and any of rows, cols, horizontal bleed, or vertical bleed
       mismatch: fail create with explicit status message naming the mismatched
       setting and frame.
13. `reload_glyphs()`.
14. Clear tool mode/temporary selection.

### 4.1 Drag/paste routing rules

- Home panel drag/paste remains the base project image intake path.
- Animation drag/paste is active only when:
  - active view is Glyphs,
  - an active project is selected,
  - `glyph_tool_mode` is `ImportAnimationFrames(Standard)` or `ImportAnimationFrames(Grid)`.
- If the user is on Home, dragged/pasted images follow the existing Home behavior.
- If the user is on Glyphs but not in animation import mode, dragged/pasted images
  must route to the existing project-level image import behavior (same as Home),
  and must not silently become animation frames.
- Every imported animation frame must still become a normal project-local icon in
  `icons/`; the animation manifest stores the resulting parent source keys.
- The import status message should distinguish animation imports from base imports,
  e.g. `Added 4 images to animation draft "walk_cycle"` rather than only
  `Added 4 images`.

Cancellation:

- `Esc` from type picker/import/select/config returns to `GlyphToolMode::None`
  with status. Imported image files remain in `icons/`; only the temporary
  animation draft selection is discarded.
  - Discard means clearing both `animation_selection_order` and
    `animation_selection_set`.

---

## 5. Playback and Rendering

### 5.1 Glyphs preview

- If current selected row's parent source key belongs to one or more animations:
  - pick deterministic active animation (first by manifest order initially).
  - tick `animation_preview` using FPS timing.
- Frame render:
  - Standard: render selected frame as single glyph preview.
  - Grid: render current frame using existing `composition_preview_lines` path by frame parent key + rows/cols.
  - Bleed settings affect final generated TTF outlines, not the text preview;
    preview metadata should still show selected left/right and top/bottom bleed
    values so users know which grid settings are attached to the animation.

### 5.2 Event loop integration

Before each draw in `tui_workspace` loop:

- `app.update_animation_preview()`.

Reset preview when:

- selection changes to non-animated source,
- active project changes,
- glyph reload invalidates animation.

### 5.3 Home panel (phase 6)

Add compact animation summary in Current project card:

- `Animations: <count>`
- first animation metadata (`name`, `type`, `fps`, `frame_count`)
- optional tiny cycling preview if space allows.

Do not remove drag-area placeholder unless layout/space rules are explicit and tested.

---

## 6. Deletion Policy

Trigger:

- `D` in Glyphs view when current parent source key is part of animations.
  - Add/update visible key hints so delete-animation discoverability matches
    existing Glyphs actions.

Flow:

1. Resolve animations that include selected source key.
2. If zero: no-op status.
3. If one: confirm delete that animation.
4. If multiple: show picker dialog first, then confirm chosen animation.

Persist behavior:

- Remove only `AnimationDef`.
- Do not auto-remove `manifest.compositions` in initial rollout.

Reason: composition entries may be shared or user-authored outside this animation and unsafe cleanup can cause data loss.

---

## 7. Build and Sample Impact

### 7.1 Build pipeline

- No changes to codepoint assignment strategy.
- Grid animation reuses existing composition splitting and existing composition
  bleed behavior.
- Grid animation creation must write the selected bleed levels into generated
  `CompositionDef` entries for every animation frame.

### 7.2 `generate_smart_sample`

Initial rollout: no mandatory changes.

Optional follow-up only if contract tests show a readability regression for animation-heavy projects:

- add animation-aware block grouping while preserving current output semantics.

---

## 8. Testing Plan

### 8.1 Unit tests (`src/tests.rs`)

1. Manifest serde roundtrip for standard and grid animations.
2. `load_runtime_config` hard validation:
   - invalid fps,
   - empty name,
   - empty frames,
   - duplicate animation names,
   - grid missing rows/cols,
   - invalid rows/cols,
   - grid bleed defaults,
   - grid bleed serde roundtrip.
3. Frame persistence uses parent source keys only.
4. Multi-select and persisted frame ordering follows explicit draft order
   (import/selection order), with alphabetical fallback only when source order
   is unavailable.
5. Mode transitions:
   - animate button enters type picker,
   - choosing Standard enters standard animation frame import,
   - choosing Grid enters grid animation frame import,
   - optional existing-glyph selection can add frames to the active draft,
   - toggle with Space,
   - Enter opens config,
   - Esc cancels.
6. Animation import routing:
   - Home drag/paste remains project-level import,
   - Glyphs animation import routes dragged/pasted images into the active animation draft,
   - Glyphs outside animation import mode does not silently create animation frames.
7. Create flow persists manifest and reloads.
8. Placeholder animate-button test replaced with real behavior assertions.
9. Preview ticker logic tested via elapsed-duration stepping (deterministic, no sleeps).
10. Delete flow tests for single-match and multi-match resolution.

### 8.2 CLI contract tests (`tests/cli_contract.rs`)

1. `build` succeeds with animation manifest containing standard and grid animations.
2. Existing JSON envelope behavior unchanged.

### 8.3 hty E2E (`scripts/tui_e2e_hty.sh`)

1. Animate Glyph -> choose Standard -> drag/paste frames -> create -> manifest assertion.
2. Animate Glyph -> choose Grid -> drag/paste frames -> set bleed knobs -> create -> composition assertions.
3. Playback visible in preview text region (state-based wait, not fixed sleep).
4. Delete animation via `D` flow and verify manifest mutation.

---

## 9. Phased Delivery

Phase 1: Schema + validation + unit tests.

Phase 2: TUI state model refactor (`GlyphToolMode`) and integrate current grid flow without behavior change.

Phase 3: Animation creation flow in Glyphs panel.

Phase 4: Glyphs preview playback.

Phase 5: Deletion UX.

Phase 6: Home panel summary/mini-preview.

Phase 7: E2E hardening and README updates.

Each phase should land with passing tests before moving to next.

---

## 10. File Change Map

- `src/project.rs`
  - `AnimationDef`, `AnimationType`, manifest/runtime fields, validation.
- `src/tui.rs`
  - `GlyphToolMode`, animation selection/config/preview state, key routing, dialogs, preview rendering, deletion flow, optional Home panel animation summary.
- `src/tests.rs`
  - unit coverage for schema, transitions, create/delete, ticker behavior.
- `tests/cli_contract.rs`
  - build contract coverage with animation schema.
- `scripts/tui_e2e_hty.sh`
  - new journeys for create/playback/delete.
- `README.md`
  - manifest schema and keys/workflow documentation.

---

## 11. Acceptance Criteria

- Users can create standard and grid animations fully from TUI.
- Animation data survives restart via `petiglyph.toml`.
- Glyphs preview animates at configured FPS.
- Grid animation frames reuse existing composition pipeline, including
  left/right and top/bottom bleed settings.
- Delete removes animation entry without removing source images.
- No regressions in existing composition, build, install, doctor, and panel navigation flows.
