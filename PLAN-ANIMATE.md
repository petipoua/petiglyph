# Animate Glyph Feature Implementation Plan (Refined)

## Goal

Enable users to create and preview animated glyph sequences from existing project-local images in `icons/`, directly from the TUI, without regressing current composition, build, or codepoint behavior.

Animation modes:

1. Standard animation: one source image per frame.
2. Grid animation: one source image per frame, each frame split as `rows x cols`.

Primary preview target:

- Glyphs view Preview panel.

Secondary preview target (phase 4):

- Home panel Current project card.

---

## 0. Scope and Non-goals

### In scope

- Manifest schema for animations.
- TUI creation flow (multi-select -> configure -> persist).
- Glyphs-panel animation playback.
- Safe animation deletion flow.
- Tests at unit, contract, and hty E2E levels.

### Not in scope for initial rollout

- New CLI subcommands for animation CRUD.
- Reworking codepoint allocation logic.
- Broad changes to `generate_smart_sample` formatting unless tests prove a gap.

---

## 1. Baseline Constraints from Current Code

- `AnimateButton` exists but is placeholder-only in key handling (`src/tui.rs`).
- Composition pipeline is already stable and keyed by parent source key (`source_parent_key`) while composition children use synthetic `source_key` with `#compose:...` suffix (`src/build.rs`).
- `load_runtime_config` currently performs strict schema/range checks and is used by CLI, TUI, build, install, and doctor (`src/project.rs`).
- Home panel currently renders project status and drag-area placeholder, not animation state (`src/tui.rs`).

Implication: frame references in animation manifest must use parent source keys (file-level keys), never tile `#compose` keys.

---

## 2. Data Model and Manifest

### 2.1 New types (`src/project.rs`)

```rust
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
- `Standard` must not require `rows`/`cols` (allow omitted; ignore if present only if we choose permissive mode, otherwise reject).

Soft runtime validation (non-fatal):

- frame file missing from current `icons/` set during preview/build contexts should not invalidate whole config load.
- missing frame is skipped in preview with status hint; animation remains editable.

Reason: keep CLI/TUI startup resilient when files are moved temporarily.

---

## 3. Interaction State Model (TUI)

Replace additive boolean mode growth with a single explicit mode enum.

### 3.1 New state (`src/tui.rs`)

```rust
enum GlyphToolMode {
    None,
    SelectGridSource,
    SelectAnimationFrames,
    ConfigureGrid(GridConfig),
    ConfigureAnimation(AnimationConfig),
}
```

`App` additions:

- `glyph_tool_mode: GlyphToolMode`
- `animation_selection: BTreeSet<String>` (parent source keys)
- `animation_preview: Option<AnimationPreview>`

`AnimationConfig`:

- `selected_frames: Vec<String>` sorted at open-time
- `name_input: Input`
- `animation_type: AnimationType`
- `fps: u8`
- `rows: u32`
- `cols: u32`
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
2. Enter `GlyphToolMode::SelectAnimationFrames`.
3. In glyph list, `Space` toggles selected parent source key.
4. `Enter` with non-empty selection opens `ConfigureAnimation` with alphabetically sorted frames.
5. User sets:
   - Name (default: common stem fallback to `animation_<n>`).
   - Type (`Standard` or `Grid`).
   - FPS (`1..=30`, default `8`).
   - Rows/Cols only when Grid (default `2x2`).
6. On Create:
   - Persist new `AnimationDef` to `manifest.animations`.
   - For Grid: ensure each frame has composition definition.
     - If composition missing: add it.
     - If present and matches: keep.
     - If present and mismatched: fail create with explicit status message.
7. `reload_glyphs()`.
8. Clear tool mode/temporary selection.

Cancellation:

- `Esc` from select/config returns to `GlyphToolMode::None` with status.

---

## 5. Playback and Rendering

### 5.1 Glyphs preview

- If current selected row's parent source key belongs to one or more animations:
  - pick deterministic active animation (first by manifest order initially).
  - tick `animation_preview` using FPS timing.
- Frame render:
  - Standard: render selected frame as single glyph preview.
  - Grid: render current frame using existing `composition_preview_lines` path by frame parent key + rows/cols.

### 5.2 Event loop integration

Before each draw in `tui_workspace` loop:

- `app.update_animation_preview()`.

Reset preview when:

- selection changes to non-animated source,
- active project changes,
- glyph reload invalidates animation.

### 5.3 Home panel (phase 4)

Add compact animation summary in Current project card:

- `Animations: <count>`
- first animation metadata (`name`, `type`, `fps`, `frame_count`)
- optional tiny cycling preview if space allows.

Do not remove drag-area placeholder unless layout/space rules are explicit and tested.

---

## 6. Deletion Policy

Trigger:

- `D` in Glyphs view when current parent source key is part of animations.

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
- Grid animation reuses existing composition splitting.

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
   - invalid rows/cols.
3. Frame persistence uses parent source keys only.
4. Multi-select ordering is alphabetical and deterministic.
5. Mode transitions:
   - enter select mode,
   - toggle with Space,
   - Enter opens config,
   - Esc cancels.
6. Create flow persists manifest and reloads.
7. Placeholder animate-button test replaced with real behavior assertions.
8. Preview ticker logic tested via elapsed-duration stepping (deterministic, no sleeps).
9. Delete flow tests for single-match and multi-match resolution.

### 8.2 CLI contract tests (`tests/cli_contract.rs`)

1. `build` succeeds with animation manifest containing standard and grid animations.
2. Existing JSON envelope behavior unchanged.

### 8.3 hty E2E (`scripts/tui_e2e_hty.sh`)

1. Multi-select -> standard animation create -> manifest assertion.
2. Multi-select -> grid animation create -> composition assertions.
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
- Grid animation frames reuse existing composition pipeline.
- Delete removes animation entry without removing source images.
- No regressions in existing composition, build, install, doctor, and panel navigation flows.
