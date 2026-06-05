fn is_animated_home_creation(kind: HomeCreationKind) -> bool {
    matches!(
        kind,
        HomeCreationKind::AnimatedGlyph | HomeCreationKind::AnimatedGridGlyph
    )
}

fn home_import_missing_sources_message(kind: HomeCreationKind) -> &'static str {
    match kind {
        HomeCreationKind::Glyph => "drop at least one source image in the popup, then press Enter",
        HomeCreationKind::Grid => {
            "create grid: drop exactly one image in the popup, then press Enter"
        }
        HomeCreationKind::AnimatedGlyph | HomeCreationKind::AnimatedGridGlyph => {
            "drop at least one frame media file in the popup, then press Enter"
        }
    }
}

fn home_enter_tweaking_message(kind: HomeCreationKind) -> &'static str {
    match kind {
        HomeCreationKind::Glyph => {
            "tweak grayscale/threshold + preview in popup, then use Next until Finish"
        }
        HomeCreationKind::Grid => {
            "tweak grayscale/threshold + preview in popup, then Continue to grid settings"
        }
        HomeCreationKind::AnimatedGlyph => {
            "tweak grayscale/threshold + preview in popup, then Continue to animation settings"
        }
        HomeCreationKind::AnimatedGridGlyph => {
            "tweak grayscale/threshold + preview in popup, then Continue to animated grid settings"
        }
    }
}

fn animation_import_processing_options(
    settings: &AnimationImportSettingsState,
) -> animation_media::AnimationImportProcessingOptions {
    animation_media::AnimationImportProcessingOptions {
        grayscale_enabled: settings.grayscale_enabled,
        grayscale: settings.grayscale_options,
    }
}

fn grayscale_options_are_default(options: animation_media::AnimationGrayscaleOptions) -> bool {
    options == animation_media::AnimationGrayscaleOptions::default()
}

fn signed_filename_value(value: i16) -> String {
    if value < 0 {
        format!("m{}", i32::from(value).unsigned_abs())
    } else {
        format!("p{}", value)
    }
}

fn grayscale_luminance_byte(r: u8, g: u8, b: u8) -> u8 {
    // Integer approximation of BT.601 luma.
    (((77u16 * r as u16) + (150u16 * g as u16) + (29u16 * b as u16)) >> 8) as u8
}

fn apply_grayscale_adjustments_for_preview(
    value: u8,
    options: animation_media::AnimationGrayscaleOptions,
) -> u8 {
    let gamma = (options.gamma_percent as f32 / 100.0).clamp(0.50, 2.00);
    let mut pixel = (value as f32 / 255.0).powf(1.0 / gamma) * 255.0;
    let contrast_factor = 1.0 + (options.contrast as f32 / 100.0);
    pixel = ((pixel - 128.0) * contrast_factor) + 128.0;
    pixel += options.brightness as f32;
    pixel.round().clamp(0.0, 255.0) as u8
}

fn apply_live_grayscale_processing(image: &mut RgbaImage, settings: &AnimationImportSettingsState) {
    if !settings.grayscale_enabled {
        return;
    }
    let options = settings.grayscale_options;
    for pixel in image.pixels_mut() {
        let luma = grayscale_luminance_byte(pixel[0], pixel[1], pixel[2]);
        let adjusted = apply_grayscale_adjustments_for_preview(luma, options);
        pixel[0] = adjusted;
        pixel[1] = adjusted;
        pixel[2] = adjusted;
    }
}

fn live_preview_coverage_key(
    source_path: &Path,
    glyph_size: u32,
    settings: &AnimationImportSettingsState,
) -> Option<LivePreviewCoverageKey> {
    if !source_path.is_file() || !is_supported_source(source_path) {
        return None;
    }
    let metadata = source_path.metadata().ok()?;
    let file_modified_ns = metadata.modified().ok().and_then(|modified| {
        modified
            .duration_since(UNIX_EPOCH)
            .ok()
            .map(|duration| duration.as_nanos())
    });
    Some(LivePreviewCoverageKey {
        source_path: source_path.to_path_buf(),
        file_len: metadata.len(),
        file_modified_ns,
        glyph_size,
        grayscale_enabled: settings.grayscale_enabled,
        grayscale_brightness: settings.grayscale_options.brightness,
        grayscale_contrast: settings.grayscale_options.contrast,
        grayscale_gamma_percent: settings.grayscale_options.gamma_percent,
    })
}

fn live_import_source_coverage_uncached(
    source_path: &Path,
    glyph_size: u32,
    settings: &AnimationImportSettingsState,
) -> Option<Vec<u8>> {
    let mut image = load_source_rgba(source_path, glyph_size).ok()?;
    apply_live_grayscale_processing(&mut image, settings);
    coverage_map_from_image(&image, glyph_size).ok()
}

fn render_test_image_from_single_glyph(glyph: &InteractiveGlyph) -> Result<RgbaImage> {
    render_test_image_from_coverage(
        &glyph.glyph.coverage,
        glyph.glyph.width,
        glyph.glyph.height,
        glyph.working_threshold,
        glyph.working_invert,
        &glyph.glyph.source_parent_key,
    )
}

fn render_test_image_from_composition_tiles(
    rows: usize,
    cols: usize,
    tiles: &[&InteractiveGlyph],
) -> Result<Option<RgbaImage>> {
    let Some(first_tile) = tiles.first() else {
        return Ok(None);
    };
    if rows == 0 || cols == 0 {
        return Ok(None);
    }

    let tile_w = usize::try_from(first_tile.glyph.width).context("tile width overflow")?;
    let tile_h = usize::try_from(first_tile.glyph.height).context("tile height overflow")?;
    let out_w = tile_w
        .checked_mul(cols)
        .ok_or_else(|| anyhow::anyhow!("composition width overflow"))?;
    let out_h = tile_h
        .checked_mul(rows)
        .ok_or_else(|| anyhow::anyhow!("composition height overflow"))?;

    let mut image = RgbaImage::from_pixel(out_w as u32, out_h as u32, Rgba([255, 255, 255, 0]));
    let mut wrote_pixels = false;

    for tile in tiles {
        let Some(tile_info) = tile.glyph.composition_tile else {
            continue;
        };
        if tile_info.row >= rows || tile_info.col >= cols {
            continue;
        }
        let width = usize::try_from(tile.glyph.width).context("tile width overflow")?;
        let height = usize::try_from(tile.glyph.height).context("tile height overflow")?;
        let expected = width
            .checked_mul(height)
            .ok_or_else(|| anyhow::anyhow!("tile pixel count overflow"))?;
        if tile.glyph.coverage.len() != expected {
            bail!(
                "tile coverage mismatch for {} (expected {}, got {})",
                tile.glyph.source_key,
                expected,
                tile.glyph.coverage.len()
            );
        }
        let x_offset = tile_info
            .col
            .checked_mul(tile_w)
            .ok_or_else(|| anyhow::anyhow!("tile x offset overflow"))?;
        let y_offset = tile_info
            .row
            .checked_mul(tile_h)
            .ok_or_else(|| anyhow::anyhow!("tile y offset overflow"))?;

        for y in 0..height {
            for x in 0..width {
                let idx = y
                    .checked_mul(width)
                    .and_then(|v| v.checked_add(x))
                    .ok_or_else(|| anyhow::anyhow!("tile raster index overflow"))?;
                let on = (tile.glyph.coverage[idx] >= tile.working_threshold) ^ tile.working_invert;
                if on {
                    image.put_pixel(
                        (x_offset + x) as u32,
                        (y_offset + y) as u32,
                        Rgba([0, 0, 0, 255]),
                    );
                    wrote_pixels = true;
                }
            }
        }
    }
    if wrote_pixels {
        Ok(Some(image))
    } else {
        Ok(None)
    }
}

fn render_test_image_from_coverage(
    coverage: &[u8],
    width_u32: u32,
    height_u32: u32,
    threshold: u8,
    invert: bool,
    context_key: &str,
) -> Result<RgbaImage> {
    let width = usize::try_from(width_u32).context("glyph width overflow")?;
    let height = usize::try_from(height_u32).context("glyph height overflow")?;
    let expected = width
        .checked_mul(height)
        .ok_or_else(|| anyhow::anyhow!("glyph pixel count overflow"))?;
    if coverage.len() != expected {
        bail!(
            "glyph coverage mismatch for {} (expected {}, got {})",
            context_key,
            expected,
            coverage.len()
        );
    }

    let mut image = RgbaImage::from_pixel(width_u32, height_u32, Rgba([255, 255, 255, 0]));
    for y in 0..height {
        for x in 0..width {
            let idx = y
                .checked_mul(width)
                .and_then(|v| v.checked_add(x))
                .ok_or_else(|| anyhow::anyhow!("glyph raster index overflow"))?;
            let on = (coverage[idx] >= threshold) ^ invert;
            if on {
                image.put_pixel(x as u32, y as u32, Rgba([0, 0, 0, 255]));
            }
        }
    }
    Ok(image)
}

fn import_settings_visible_focuses(kind: HomeCreationKind) -> Vec<AnimationImportSettingsFocus> {
    let mut focuses = vec![
        AnimationImportSettingsFocus::GrayscaleToggle,
        AnimationImportSettingsFocus::GrayscaleOptionsButton,
        AnimationImportSettingsFocus::Threshold,
    ];
    if matches!(
        kind,
        HomeCreationKind::AnimatedGlyph | HomeCreationKind::AnimatedGridGlyph
    ) {
        focuses.push(AnimationImportSettingsFocus::FramesButton);
    }
    if !matches!(kind, HomeCreationKind::Glyph) {
        focuses.push(AnimationImportSettingsFocus::ExportTestImageButton);
    }
    focuses.push(AnimationImportSettingsFocus::Continue);
    if matches!(kind, HomeCreationKind::Glyph) {
        focuses.push(AnimationImportSettingsFocus::Back);
        focuses.push(AnimationImportSettingsFocus::SkipAll);
    }
    focuses
}

fn normalize_import_settings_focus(
    settings: &mut AnimationImportSettingsState,
    kind: HomeCreationKind,
) {
    let visible = import_settings_visible_focuses(kind);
    if !visible.contains(&settings.focus) {
        settings.focus = AnimationImportSettingsFocus::Continue;
    }
}

fn move_import_settings_focus(
    settings: &mut AnimationImportSettingsState,
    kind: HomeCreationKind,
    direction: i8,
) {
    let visible = import_settings_visible_focuses(kind);
    let current = visible
        .iter()
        .position(|focus| *focus == settings.focus)
        .unwrap_or_else(|| {
            visible
                .iter()
                .position(|focus| *focus == AnimationImportSettingsFocus::Continue)
                .unwrap_or(0)
        });
    let next = if direction < 0 {
        current.saturating_sub(1)
    } else {
        (current + 1).min(visible.len().saturating_sub(1))
    };
    settings.focus = visible[next];
}

fn import_settings_enter_continues_workflow(
    settings: &AnimationImportSettingsState,
    kind: HomeCreationKind,
) -> bool {
    if settings.grayscale_editor.is_some() {
        return false;
    }

    match settings.focus {
        AnimationImportSettingsFocus::Continue
        | AnimationImportSettingsFocus::Back
        | AnimationImportSettingsFocus::SkipAll
        | AnimationImportSettingsFocus::Threshold => true,
        AnimationImportSettingsFocus::FramesButton => matches!(
            kind,
            HomeCreationKind::AnimatedGlyph | HomeCreationKind::AnimatedGridGlyph
        ),
        AnimationImportSettingsFocus::GrayscaleToggle
        | AnimationImportSettingsFocus::GrayscaleOptionsButton
        | AnimationImportSettingsFocus::ExportTestImageButton => false,
    }
}

fn rotate_grayscale_knob_left(editor: &mut GrayscaleOptionsEditor) {
    editor.focus = match editor.focus {
        GrayscaleKnobFocus::Brightness => GrayscaleKnobFocus::Brightness,
        GrayscaleKnobFocus::Contrast => GrayscaleKnobFocus::Brightness,
        GrayscaleKnobFocus::Gamma => GrayscaleKnobFocus::Contrast,
    };
}

fn rotate_grayscale_knob_right(editor: &mut GrayscaleOptionsEditor) {
    editor.focus = match editor.focus {
        GrayscaleKnobFocus::Brightness => GrayscaleKnobFocus::Contrast,
        GrayscaleKnobFocus::Contrast => GrayscaleKnobFocus::Gamma,
        GrayscaleKnobFocus::Gamma => GrayscaleKnobFocus::Gamma,
    };
}

fn adjust_grayscale_editor(editor: &mut GrayscaleOptionsEditor, direction: i16) {
    match editor.focus {
        GrayscaleKnobFocus::Brightness => {
            let next = editor.draft.brightness.saturating_add(direction);
            editor.draft.brightness =
                next.clamp(GRAYSCALE_BRIGHTNESS_MIN, GRAYSCALE_BRIGHTNESS_MAX);
        }
        GrayscaleKnobFocus::Contrast => {
            let next = editor.draft.contrast.saturating_add(direction);
            editor.draft.contrast = next.clamp(GRAYSCALE_CONTRAST_MIN, GRAYSCALE_CONTRAST_MAX);
        }
        GrayscaleKnobFocus::Gamma => {
            let step = i32::from(direction) * 5;
            let next = i32::from(editor.draft.gamma_percent) + step;
            editor.draft.gamma_percent = next.clamp(
                i32::from(GRAYSCALE_GAMMA_MIN),
                i32::from(GRAYSCALE_GAMMA_MAX),
            ) as u16;
        }
    }
}

fn handle_animation_import_settings_key(app: &mut App, key: KeyEvent) -> Result<bool> {
    if app.animation_create_in_progress() {
        return Ok(true);
    }

    if let Some(editor) = app.animation_import_settings.grayscale_editor.as_mut() {
        match key.code {
            KeyCode::Esc => {
                app.animation_import_settings.grayscale_options = editor.original;
                app.animation_import_settings.grayscale_editor = None;
                app.status = Some("grayscale options edit canceled".to_string());
                return Ok(true);
            }
            KeyCode::Enter => {
                app.animation_import_settings.grayscale_options = editor.draft;
                app.animation_import_settings.grayscale_editor = None;
                app.status = Some("grayscale options updated".to_string());
                return Ok(true);
            }
            KeyCode::Left | KeyCode::Char('h') => {
                rotate_grayscale_knob_left(editor);
                return Ok(true);
            }
            KeyCode::Right | KeyCode::Char('l') => {
                rotate_grayscale_knob_right(editor);
                return Ok(true);
            }
            KeyCode::Up | KeyCode::Char('k') => {
                adjust_grayscale_editor(editor, 1);
                return Ok(true);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                adjust_grayscale_editor(editor, -1);
                return Ok(true);
            }
            _ => return Ok(false),
        }
    }

    let kind = match app.home_workflow {
        HomeWorkflow::Tweaking(kind) => kind,
        _ => HomeCreationKind::AnimatedGlyph,
    };
    normalize_import_settings_focus(&mut app.animation_import_settings, kind);

    match key.code {
        KeyCode::Left | KeyCode::Char('h') => {
            move_import_settings_focus(&mut app.animation_import_settings, kind, -1);
            Ok(true)
        }
        KeyCode::Right | KeyCode::Char('l') => {
            move_import_settings_focus(&mut app.animation_import_settings, kind, 1);
            Ok(true)
        }
        KeyCode::Up | KeyCode::Char('k') | KeyCode::Down | KeyCode::Char('j') => {
            if app.animation_import_settings.focus == AnimationImportSettingsFocus::GrayscaleToggle
            {
                app.animation_import_settings.grayscale_enabled =
                    !app.animation_import_settings.grayscale_enabled;
                app.status = Some(format!(
                    "grayscale {} for imported GIF/video frames",
                    if app.animation_import_settings.grayscale_enabled {
                        "enabled"
                    } else {
                        "disabled"
                    }
                ));
                return Ok(true);
            }
            if app.animation_import_settings.focus == AnimationImportSettingsFocus::Threshold {
                let step: i8 = if matches!(key.code, KeyCode::Up | KeyCode::Char('k')) {
                    1
                } else {
                    -1
                };
                app.animation_import_settings.threshold = app
                    .animation_import_settings
                    .threshold
                    .saturating_add_signed(step);
                let marker = if app.animation_import_settings.threshold == app.config.base_threshold
                {
                    "default"
                } else {
                    "custom"
                };
                app.status = Some(format!(
                    "preview threshold set to {} ({marker})",
                    app.animation_import_settings.threshold
                ));
                return Ok(true);
            }
            if app.animation_import_settings.focus == AnimationImportSettingsFocus::FramesButton
                && matches!(
                    app.home_workflow,
                    HomeWorkflow::Tweaking(HomeCreationKind::AnimatedGlyph)
                        | HomeWorkflow::Tweaking(HomeCreationKind::AnimatedGridGlyph)
                )
            {
                if app.animation_selection_order.is_empty() {
                    app.status = Some("import at least one animation frame first".to_string());
                    return Ok(true);
                }
                let step: i32 = if matches!(key.code, KeyCode::Up | KeyCode::Char('k')) {
                    1
                } else {
                    -1
                };
                let total = app.animation_selection_order.len();
                let current = app.animation_import_settings.preview_frame_index % total;
                let next = ((current as i32 + step).rem_euclid(total as i32)) as usize;
                app.animation_import_settings.preview_frame_index = next;
                app.status = Some(format!("preview frame {}/{}", next + 1, total));
                return Ok(true);
            }
            if app.animation_import_settings.focus
                == AnimationImportSettingsFocus::ExportTestImageButton
                && matches!(
                    app.home_workflow,
                    HomeWorkflow::Tweaking(HomeCreationKind::AnimatedGlyph)
                        | HomeWorkflow::Tweaking(HomeCreationKind::AnimatedGridGlyph)
                )
            {
                let step = if matches!(key.code, KeyCode::Up | KeyCode::Char('k')) {
                    1
                } else {
                    -1
                };
                let next = i32::from(app.animation_import_settings.export_frame_count) + step;
                app.animation_import_settings.export_frame_count = next.clamp(
                    i32::from(EXPORT_TEST_FRAMES_MIN),
                    i32::from(EXPORT_TEST_FRAMES_MAX),
                ) as u16;
                app.status = Some(format!(
                    "test-image export frame count set to {}",
                    app.animation_import_settings.export_frame_count
                ));
                return Ok(true);
            }
            Ok(true)
        }
        KeyCode::Enter => match app.animation_import_settings.focus {
            AnimationImportSettingsFocus::GrayscaleToggle => {
                app.animation_import_settings.grayscale_enabled =
                    !app.animation_import_settings.grayscale_enabled;
                app.status = Some(format!(
                    "grayscale {} for imported GIF/video frames",
                    if app.animation_import_settings.grayscale_enabled {
                        "enabled"
                    } else {
                        "disabled"
                    }
                ));
                Ok(true)
            }
            AnimationImportSettingsFocus::GrayscaleOptionsButton => {
                let current = app.animation_import_settings.grayscale_options;
                app.animation_import_settings.grayscale_editor = Some(GrayscaleOptionsEditor {
                    original: current,
                    draft: current,
                    focus: GrayscaleKnobFocus::Brightness,
                });
                app.status = Some(
                    "editing grayscale options: \u{2190}/\u{2192} choose knob, \u{2191}/\u{2193} change, Enter apply, Esc cancel"
                        .to_string(),
                );
                Ok(true)
            }
            AnimationImportSettingsFocus::Threshold => Ok(false),
            AnimationImportSettingsFocus::FramesButton => Ok(false),
            AnimationImportSettingsFocus::ExportTestImageButton => {
                app.export_animation_import_test_image()?;
                Ok(true)
            }
            AnimationImportSettingsFocus::Continue => Ok(false),
            AnimationImportSettingsFocus::Back => Ok(false),
            AnimationImportSettingsFocus::SkipAll => Ok(false),
        },
        _ => Ok(false),
    }
}

fn handle_home_creation_key(app: &mut App, key: KeyEvent) -> Result<()> {
    match app.home_workflow {
        HomeWorkflow::Import(kind) => match key.code {
            KeyCode::Esc => {
                app.cancel_home_workflow()?;
                app.status = Some("home creation workflow canceled".to_string());
            }
            KeyCode::Enter => {
                if is_animated_home_creation(kind) && app.animation_import_task.is_some() {
                    app.status = Some("animation frames are still loading".to_string());
                    return Ok(());
                }
                if !is_animated_home_creation(kind) && app.home_import_task.is_some() {
                    app.status = Some("images are still loading".to_string());
                    return Ok(());
                }
                if !app.has_imported_home_sources(kind) {
                    app.status = Some(home_import_missing_sources_message(kind).to_string());
                    return Ok(());
                }
                app.home_workflow = HomeWorkflow::Tweaking(kind);
                if matches!(kind, HomeCreationKind::Glyph) {
                    app.rebuild_home_tweak_queue_for_glyph();
                    app.home_workflow_tweak_source_index = 0;
                    app.sync_threshold_to_current_glyph_tweak_source();
                }
                app.home_workflow_error = None;
                app.status = Some(home_enter_tweaking_message(kind).to_string());
            }
            _ => {}
        },
        HomeWorkflow::Tweaking(kind) => {
            let enter_continues_workflow = matches!(key.code, KeyCode::Enter)
                && import_settings_enter_continues_workflow(&app.animation_import_settings, kind);
            if handle_animation_import_settings_key(app, key)? {
                return Ok(());
            }
            if enter_continues_workflow {
                return continue_home_creation_tweaking_enter(app, kind);
            }
            match key.code {
                KeyCode::Esc => {
                    app.cancel_home_workflow()?;
                    app.status = Some("home creation workflow canceled".to_string());
                }
                KeyCode::Enter => continue_home_creation_tweaking_enter(app, kind)?,
                _ => {}
            }
        }
        HomeWorkflow::ConfigureGrid => {
            if let Some(mut config) = app.grid_config.clone() {
                let res = handle_grid_config_key(app, &mut config, key);
                app.grid_config = if app.grid_config.is_some() {
                    Some(config)
                } else {
                    None
                };
                return res;
            }
            app.home_workflow = HomeWorkflow::Tweaking(HomeCreationKind::Grid);
        }
        HomeWorkflow::ConfigureAnimation(animation_type) => {
            if let GlyphToolMode::ConfigureAnimation(config) = app.glyph_tool_mode.clone() {
                handle_animation_config_key(app, key, config)?;
                if matches!(app.home_workflow, HomeWorkflow::ConfigureAnimation(_))
                    && matches!(app.glyph_tool_mode, GlyphToolMode::None)
                {
                    app.home_workflow = HomeWorkflow::Tweaking(match animation_type {
                        AnimationType::Standard => HomeCreationKind::AnimatedGlyph,
                        AnimationType::Grid => HomeCreationKind::AnimatedGridGlyph,
                    });
                }
                return Ok(());
            }
            app.home_workflow = HomeWorkflow::Tweaking(match animation_type {
                AnimationType::Standard => HomeCreationKind::AnimatedGlyph,
                AnimationType::Grid => HomeCreationKind::AnimatedGridGlyph,
            });
        }
        _ => {
            handle_glyphs_key(app, key)?;
            if app.grid_config.is_none() && app.selecting_for_grid {
                app.selecting_for_grid = false;
            }
            if matches!(app.glyph_tool_mode, GlyphToolMode::None)
                && matches!(app.home_workflow, HomeWorkflow::Launcher)
                && app.grid_config.is_none()
            {
                app.complete_home_workflow_to_glyphs();
            }
        }
    }
    Ok(())
}

fn continue_home_creation_tweaking_enter(app: &mut App, kind: HomeCreationKind) -> Result<()> {
    app.poll_home_import_task();
    if app.home_import_task.is_some() {
        app.status = Some("images are still loading".to_string());
        return Ok(());
    }
    if matches!(kind, HomeCreationKind::Glyph) {
        let is_back = app.animation_import_settings.focus == AnimationImportSettingsFocus::Back;
        let is_skip_all =
            app.animation_import_settings.focus == AnimationImportSettingsFocus::SkipAll;
        let current_source = app.current_glyph_tweak_source_key().cloned();
        if is_back {
            if app.home_workflow_tweak_source_index > 0 {
                app.home_workflow_tweak_source_index =
                    app.home_workflow_tweak_source_index.saturating_sub(1);
                app.sync_threshold_to_current_glyph_tweak_source();
                let current = app.home_workflow_tweak_source_index.saturating_add(1);
                let total = app.home_workflow_tweak_source_queue.len();
                app.status = Some(format!("previous preview ({current}/{total})"));
            } else {
                app.status = Some("already at first preview".to_string());
            }
            return Ok(());
        }
        if is_skip_all {
            let remaining_start = app
                .home_workflow_tweak_source_index
                .min(app.home_workflow_tweak_source_queue.len());
            let remaining = app.home_workflow_tweak_source_queue[remaining_start..].to_vec();
            if !remaining.is_empty() {
                let original = app.animation_import_settings.threshold;
                app.animation_import_settings.threshold = app.config.base_threshold;
                persist_creation_workflow_threshold(app, remaining)?;
                app.animation_import_settings.threshold = original;
            }
            app.complete_home_glyph_creation_to_glyphs();
            app.status = Some("skipped remaining previews with default settings".to_string());
            return Ok(());
        }

        if let Some(source_key) = current_source {
            let is_final_preview = app.home_workflow_tweak_source_index.saturating_add(1)
                >= app.home_workflow_tweak_source_queue.len();
            persist_creation_workflow_threshold(app, vec![source_key])?;
            if is_final_preview {
                return continue_home_workflow_after_tweaking(app, kind);
            }
            app.home_workflow_tweak_source_index = app
                .home_workflow_tweak_source_index
                .saturating_add(1)
                .min(app.home_workflow_tweak_source_queue.len());
            app.sync_threshold_to_current_glyph_tweak_source();
        }
        if app.current_glyph_tweak_source_key().is_some() {
            let current = app.home_workflow_tweak_source_index.saturating_add(1);
            let total = app.home_workflow_tweak_source_queue.len();
            app.status = Some(format!("next preview ({current}/{total})"));
            return Ok(());
        }
    }
    continue_home_workflow_after_tweaking(app, kind)
}

fn continue_home_workflow_after_tweaking(app: &mut App, kind: HomeCreationKind) -> Result<()> {
    match kind {
        HomeCreationKind::Glyph => {
            if app.home_workflow_tweak_source_queue.is_empty() {
                persist_creation_workflow_threshold(
                    app,
                    creation_workflow_threshold_sources(app, kind),
                )?;
            }
            app.complete_home_glyph_creation_to_glyphs();
        }
        HomeCreationKind::Grid => {
            let Some(source_key) = app.home_workflow_grid_source_key.clone() else {
                app.status = Some(
                    "create grid: drop exactly one image in the popup, then press Enter"
                        .to_string(),
                );
                return Ok(());
            };
            persist_creation_workflow_threshold(
                app,
                creation_workflow_threshold_sources(app, kind),
            )?;
            app.grid_config = Some(GridConfig {
                source_key,
                rows: 2,
                cols: 2,
                horizontal_bleed: BleedLevel::Weak,
                vertical_bleed: BleedLevel::Off,
                focus: GridConfigFocus::Rows,
            });
            app.home_workflow = HomeWorkflow::ConfigureGrid;
            app.home_workflow_error = None;
            app.status =
                Some("configure grid in popup: rows, cols, bleed, then Create Grid".to_string());
        }
        HomeCreationKind::AnimatedGlyph => {
            if app.animation_import_task.is_some() {
                app.status = Some("animation frames are still loading".to_string());
                return Ok(());
            }
            if app.animation_selection_order.is_empty() {
                app.status = Some(
                    "drop at least one frame media file in the popup, then press Enter".to_string(),
                );
                return Ok(());
            }
            persist_creation_workflow_threshold(
                app,
                creation_workflow_threshold_sources(app, kind),
            )?;
            app.start_animation_config(AnimationType::Standard);
            app.home_workflow = HomeWorkflow::ConfigureAnimation(AnimationType::Standard);
            app.status = Some("configure animated glyph in popup, then Create".to_string());
        }
        HomeCreationKind::AnimatedGridGlyph => {
            if app.animation_import_task.is_some() {
                app.status = Some("animation frames are still loading".to_string());
                return Ok(());
            }
            if app.animation_selection_order.is_empty() {
                app.status = Some(
                    "drop at least one frame media file in the popup, then press Enter".to_string(),
                );
                return Ok(());
            }
            persist_creation_workflow_threshold(
                app,
                creation_workflow_threshold_sources(app, kind),
            )?;
            app.start_animation_config(AnimationType::Grid);
            app.home_workflow = HomeWorkflow::ConfigureAnimation(AnimationType::Grid);
            app.status = Some("configure animated grid glyph in popup, then Create".to_string());
        }
    }
    Ok(())
}

fn creation_workflow_threshold_sources(app: &App, kind: HomeCreationKind) -> Vec<String> {
    match kind {
        HomeCreationKind::Glyph => app.home_workflow_recent_imported_source_keys.clone(),
        HomeCreationKind::Grid => app
            .home_workflow_grid_source_key
            .iter()
            .cloned()
            .collect::<Vec<_>>(),
        HomeCreationKind::AnimatedGlyph | HomeCreationKind::AnimatedGridGlyph => {
            app.animation_selection_order.clone()
        }
    }
}

fn persist_creation_workflow_threshold(app: &mut App, source_keys: Vec<String>) -> Result<()> {
    if app.active_project.is_none() {
        return Ok(());
    }

    let threshold = app.animation_import_settings.threshold;
    let threshold_override = if threshold == app.config.base_threshold {
        None
    } else {
        Some(threshold)
    };
    let sources = source_keys.into_iter().collect::<BTreeSet<_>>();
    for source_key in &sources {
        persist_threshold_override(&app.manifest_path, source_key, threshold_override)
            .with_context(|| format!("failed to save threshold for {source_key}"))?;
        match threshold_override {
            Some(value) => {
                app.config
                    .threshold_overrides
                    .insert(source_key.clone(), value);
            }
            None => {
                app.config.threshold_overrides.remove(source_key);
            }
        }
    }

    for glyph in &mut app.glyphs {
        if sources.contains(&glyph.glyph.source_parent_key) {
            glyph.working_threshold = threshold;
            glyph.saved_threshold = threshold_override;
        }
    }
    Ok(())
}

fn handle_first_install_notice_key(app: &mut App, code: KeyCode) -> Result<()> {
    if matches!(code, KeyCode::Enter | KeyCode::Esc | KeyCode::Char(' ')) {
        app.first_install_notice_open = false;
    }
    tui_debug_log("first_install_notice.exit", app_debug_state(app));
    Ok(())
}

