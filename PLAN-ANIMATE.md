# Animate Glyph Feature Implementation Plan

## Goal

Enable users to group existing source images into timed **animation sequences** directly from the TUI. Two animation types are supported:

1. **Standard animated glyph**: Each frame is one glyph. The TUI preview cycles through frames at user-defined FPS.
2. **Grid animated glyph**: Each frame image is split into a rows×cols grid of glyphs. The TUI preview assembles each frame's grid and cycles through them at user-defined FPS.

The animation preview is rendered in:
- The **Glyphs view Preview block** (right panel)
- The **Home panel Current project area** (the sample/sample space of the installed fonts card)

---

## 1. Data Model

### New Types (`src/project.rs`)

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct AnimationDef {
    pub(crate) name: String,
    #[serde(rename = "type")]
    pub(crate) animation_type: AnimationType,
    pub(crate) fps: u8,
    pub(crate) frames: Vec<String>, // source_keys in frame order
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

### Manifest Additions

- `animations: Vec<AnimationDef>` added to `Manifest` with `#[serde(default, skip_serializing_if = "Vec::is_empty")]`.
- `animations: Vec<AnimationDef>` added to `RuntimeConfig`.

### Validation (`load_runtime_config`)

- `fps` must be 1–30.
- `frames` must be non-empty.
- For `Grid` type: `rows` and `cols` must be `Some` and > 0.
- All `frames` entries must reference source keys present in `icons/`.

### Example TOML

```toml
input_dir = "icons"
out_dir = "build"
font_name = "Demo"
glyph_size = 64
threshold = 64
codepoint_start = "U+100000"

[compositions]
"spin_01.png" = { rows = 2, cols = 2 }
"spin_02.png" = { rows = 2, cols = 2 }
"spin_03.png" = { rows = 2, cols = 2 }

[[animations]]
name = "spin"
type = "grid"
fps = 8
rows = 2
cols = 2
frames = ["spin_01.png", "spin_02.png", "spin_03.png"]

[[animations]]
name = "loading"
type = "standard"
fps = 10
frames = ["load_01.png", "load_02.png", "load_03.png", "load_04.png"]
```

---

## 2. TUI Multi-Select & Creation Flow

### New State (`src/tui.rs`)

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AnimationConfigFocus {
    Name,
    Type,
    Fps,
    Rows,
    Cols,
    Create,
}

#[derive(Debug, Clone)]
struct AnimationConfig {
    detected_frames: Vec<String>, // sorted alphabetically
    name: Input,
    animation_type: AnimationType,
    fps: u8,
    rows: u32,
    cols: u32,
    focus: AnimationConfigFocus,
}

#[derive(Debug, Clone)]
struct AnimationPreview {
    animation_name: String,
    frame_index: usize,
    last_frame_at: Instant,
}
```

Additions to `App`:
```rust
animation_config: Option<AnimationConfig>,
selecting_for_animation: bool,
animation_selection: BTreeSet<String>, // source_keys selected by user
animation_preview: Option<AnimationPreview>,
```

### User Journey

1. User drops images into `icons/`.
2. In **Glyphs** view, navigate to `GlyphsFocus::AnimateButton`, press `Enter`.
3. App enters `selecting_for_animation = true`.
4. Status bar: *"Multi-select frames with Space, then press Enter to configure."*
5. User navigates the glyph list. **`Space`** toggles selection of the current glyph. Selected items show a `☑` marker.
6. When satisfied, user presses **`Enter`**.
7. App collects selected source keys, **sorts them alphabetically**, and opens the `AnimationConfig` dialog.
8. Config dialog fields (navigate with arrows/digits, like `GridConfig`):
   - `Name`: text input (default = common stem prefix of selected frames).
   - `Type`: toggle `Standard` / `Grid` with Left/Right.
   - `FPS`: 1–30, default 8.
   - `Rows` / `Cols`: only active when `Type == Grid`; default 2×2.
9. User focuses **Create** and presses `Enter`.
10. App persists:
    - `AnimationDef` to `manifest.animations`.
    - If `Grid`: `CompositionDef` entries for **each frame image** to `manifest.compositions` (build reuse).
11. `reload_glyphs()` is called.
12. `animation_config` and `selecting_for_animation` are cleared.

### Key Handlers

- `handle_glyphs_key`: route `AnimateButton` to start multi-select mode.
- In multi-select mode: `Space` toggles selection, `Enter` opens config, `Esc` cancels.
- `handle_animation_config_key`: navigate focus, adjust values, toggle type, confirm with Enter, cancel with Esc.

---

## 3. Build System Changes

### Grid Animation Frames

- Grid animation frames are automatically added to `manifest.compositions` at creation time. This reuses the existing tile-splitting build path (`compose_tiles`) without introducing new build logic.
- The `generate_smart_sample` function is updated so that **animation composition frames are grouped by animation and rendered as discrete multiline grid blocks** (one block per frame), separated by blank lines, instead of mixing them with regular glyphs.

### Standard Animation Frames

- No build changes. Frames are regular glyphs.

### Codepoint Assignment

- No changes to glyph lock or codepoint assignment. Frames are treated as regular glyphs/compositions.

---

## 4. TUI Preview & Animation Playback

### Glyphs View Preview Block

When the selected glyph (or composition parent) is a member of an animation:

- `App.animation_preview` is initialized with the matching `AnimationDef`.
- `App::update_animation_preview()` advances `frame_index` based on `fps` timing.
- `draw_glyphs_view` renders the current frame:
  - **Standard**: `preview_lines()` for the current frame's glyph.
  - **Grid**: `composition_preview_lines()` assembled from the current frame's tiles.

When selection changes to a non-animated glyph, `animation_preview` is cleared and static preview returns.

### Home Panel Current Project Area

In `draw_welcome_view`, the **Current project** panel gains an **Animations** subsection below the build status lines (replacing or augmenting the drag-images placeholder when animations exist):

- Lists animation names: `spin (grid 2×2, 8 fps, 3 frames)`.
- The first animation (or selected animation) shows a **mini cycling preview** using the same `AnimationPreview` timer state.

This provides the "sample area" preview the user requested in the Home panel.

### Timer Update

In the main `tui_workspace` event loop, before each draw:

```rust
app.update_animation_preview();
```

```rust
fn update_animation_preview(&mut self) {
    let Some(preview) = &mut self.animation_preview else { return };
    let Some(anim) = self.config.animations.iter().find(|a| a.name == preview.animation_name) else { return };
    let frame_duration = Duration::from_millis(1000 / anim.fps.max(1) as u64);
    if Instant::now().duration_since(preview.last_frame_at) >= frame_duration {
        preview.frame_index = (preview.frame_index + 1) % anim.frames.len();
        preview.last_frame_at = Instant::now();
    }
}
```

---

## 5. Animation Deletion

- In **Glyphs** view, when a glyph that belongs to an animation is selected, pressing **`D`** deletes the **entire animation** (after a confirmation popup: *"Delete animation 'spin'? This removes the animation entry but keeps the source images."*).
- The app:
  1. Removes the `AnimationDef` from `manifest.animations`.
  2. If the animation was `Grid`, optionally removes the associated `CompositionDef` entries from `manifest.compositions`.
  3. Reloads glyphs.

---

## 6. Rendering Additions

### `draw_animation_config_ui`

Centered popup (similar to `draw_grid_config_ui`) showing:
- Header: *"Configure Animation (3 frames selected)"*
- Fields: `Name`, `Type`, `FPS`, `Rows`, `Cols`
- Detected frame list (alphabetical)
- Create button
- Footer controls hint

### Glyph List Indicators

In `draw_glyphs_view`, glyphs that are animation frames show a `~` marker (yellow) next to the name. Composition parents that are grid animation frames also show `~`.

### Footer Keys

Add to the Glyphs footer:
```
a animate  Space select frame  D delete animation
```

---

## 7. Testing Strategy

### Unit Tests (`src/tests.rs`)

1. **Manifest serde**: Round-trip `AnimationDef` through TOML for Standard and Grid.
2. **Validation**: `load_runtime_config` rejects `fps = 0`, empty `frames`, grid without rows/cols.
3. **Frame ordering**: Selected frames are sorted alphabetically before persisting.
4. **TUI transitions**:
   - `AnimateButton` enters multi-select mode.
   - `Space` toggles selection.
   - `Enter` with selection opens config.
   - `Esc` cancels.
   - Creating animation persists to manifest and reloads glyphs.
5. **Animation preview timer**: `update_animation_preview` advances at correct intervals.
6. **Deletion**: `D` removes animation from manifest.

### Integration Tests (`tests/cli_contract.rs`)

7. CLI `build` succeeds with animation manifest.
8. Sample text for grid animations shows frame grids as separate blocks.

### E2E Tests (`scripts/tui_e2e_hty.sh`)

9. Drop frames → multi-select → create standard animation → verify manifest.
10. Drop frames → multi-select → create grid animation → verify compositions added → build succeeds.
11. Select animation frame → preview cycles → press `D` → confirm deletion.

---

## 8. File Change Map

| File | Changes |
|------|---------|
| `src/project.rs` | Add `AnimationDef`, `AnimationType`; add `animations` to `Manifest` and `RuntimeConfig`; add validation rules. |
| `src/build.rs` | Update `generate_smart_sample` to render animation frame grids as discrete blocks. |
| `src/tui.rs` | Add `AnimationConfig`, `AnimationConfigFocus`, `AnimationPreview`; add multi-select state to `App`; add creation/deletion handlers; add `draw_animation_config_ui`; modify `draw_glyphs_view` for list markers and animated preview; modify `draw_welcome_view` for Home panel animation preview; add `update_animation_preview`; add footer keys. |
| `src/tests.rs` | Add unit tests for serde, validation, frame ordering, TUI transitions, timer, deletion. |
| `tests/cli_contract.rs` | Add build test with animation manifest. |
| `scripts/tui_e2e_hty.sh` | Add E2E journeys for standard/grid animation lifecycle. |
| `README.md` | Document animation manifest schema, TUI keys (`a`, `Space`, `D`), and workflow. |

---

## 9. Open Questions for Implementation Phase

- Should grid animation compositions be **hidden from the Glyphs list** to reduce clutter, or shown as regular composition entries? (Currently planned: shown as regular entries with `~` marker.)
- Should the Home panel animation preview auto-play the first animation, or require explicit selection? (Currently planned: auto-play first animation.)
