# TESTS

## TUI E2E (hty)

`./scripts/tui_e2e_hty.sh` is the process-level TUI E2E harness.

Run all journeys:

```bash
./scripts/tui_e2e_hty.sh
```

Run specific journeys:

```bash
./scripts/tui_e2e_hty.sh --journey 6,7,8
```

Watch sessions live while running:

```bash
./scripts/tui_e2e_hty.sh --watch
./scripts/tui_e2e_hty.sh --watch --watch-terminal alacritty
```

Run render diagnostics only:

```bash
./scripts/tui_e2e_hty.sh --render-probe-only
```

### Main Journey Types

1. Session smoke:
- Launch from an existing project and quit cleanly.

2. Home project management:
- Create a project from the Home panel input flow.

3. Build pipeline validation:
- Install from TUI shortcuts and assert `build/` artifacts are created.
- Rescan after adding a new source and assert rebuild includes the new file.

4. Glyph editing persistence:
- Change a selected glyph threshold in Glyphs.
- Verify `threshold_overrides` is written to `petiglyph.toml`.
- Clear the override and verify it is removed.

5. Workspace project selection:
- In workspace mode, select a project from the project list and open/build only that project.

6. Creation workflow popup: create glyph
- Import a still image, tweak it, and continue into Glyphs.

7. Creation workflow popup: create grid
- Import one image, configure rows/cols/bleed, and persist the composition in the manifest.

8. Creation workflow popup: create animated glyph
- Import a GIF, expand frames, configure the animation, and persist the animation definition.

9. Font lifecycle from TUI:
- Install via Home/shortcut into an isolated session `HOME`.

10. Full end-to-end user story:
- Create a project, run all creation workflows, tweak knobs in the popup and Glyphs panel, install, validate sample-area Enter copy behavior, create a second empty project, remove installed fonts, delete both projects, and quit.

## Notes

- The harness uses an isolated `HOME` per journey to avoid cross-test font/install state.
- FFmpeg first-run prompt state is pre-seeded so tests focus on TUI behavior.
- Assertions prefer filesystem outcomes and explicit waits over fixed sleeps.
- `--render-probe` helps diagnose terminal repaint/default-background issues observed in hty sessions.
