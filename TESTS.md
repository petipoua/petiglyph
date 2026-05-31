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
- In workspace mode, select a project from the project list and open/build only that project.

3. Build pipeline validation:
- Trigger build from TUI shortcuts and assert `build/` artifacts are created.
- Rescan after adding a new source and assert rebuild includes the new file.

4. Glyph editing persistence:
- Change a selected glyph threshold in Glyphs.
- Verify `threshold_overrides` is written to `petiglyph.toml`.
- Clear the override and verify it is removed.

5. Creation workflow popup (Home):
- `Create glyph`: import path payload, tweak step, continue to Glyphs.
- `Create grid`: import, tweak step, grid config, persist rows/cols/bleed in manifest.
- `Create animated glyph`: GIF import to extracted frame PNGs, animation config, persist animation definition.

6. Font lifecycle from TUI:
- Install via Home/shortcut into isolated session `HOME`.

7. Full end-to-end user story:
- Create project, run all creation workflows (standard, grid, animated, animated-grid), and tweak knobs in popup + Glyphs panel.
- Install, validate sample-area Enter copy behavior, create a second empty project, remove installed fonts, delete both projects, quit.

## Notes

- The harness uses an isolated `HOME` per journey to avoid cross-test font/install state.
- FFmpeg first-run prompt state is pre-seeded so tests focus on TUI behavior.
- Assertions prefer filesystem outcomes and explicit waits over fixed sleeps.
- `--render-probe` helps diagnose terminal repaint/default-background issues observed in hty sessions.
