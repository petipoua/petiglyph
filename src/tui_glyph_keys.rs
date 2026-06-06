fn handle_animation_config_key(
    app: &mut App,
    key: KeyEvent,
    mut animation_config: AnimationConfig,
) -> Result<bool> {
    if app.animation_create_in_progress() {
        return Ok(true);
    }
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => {
            app.glyph_tool_mode = GlyphToolMode::None;
            app.clear_animation_draft();
            app.status = Some("animation configuration canceled".to_string());
            return Ok(true);
        }
        KeyCode::Left | KeyCode::Char('h') => {
            animation_config.focus = match animation_config.focus {
                AnimationConfigFocus::Fps => AnimationConfigFocus::Fps,
                AnimationConfigFocus::Rows => AnimationConfigFocus::Fps,
                AnimationConfigFocus::Cols => AnimationConfigFocus::Rows,
                AnimationConfigFocus::HorizontalBleed => AnimationConfigFocus::Cols,
                AnimationConfigFocus::VerticalBleed => AnimationConfigFocus::HorizontalBleed,
                AnimationConfigFocus::Create => {
                    if animation_config.animation_type == AnimationType::Grid {
                        AnimationConfigFocus::VerticalBleed
                    } else {
                        AnimationConfigFocus::Fps
                    }
                }
            };
        }
        KeyCode::Right | KeyCode::Char('l') => {
            animation_config.focus = match animation_config.focus {
                AnimationConfigFocus::Fps => {
                    if animation_config.animation_type == AnimationType::Grid {
                        AnimationConfigFocus::Rows
                    } else {
                        AnimationConfigFocus::Create
                    }
                }
                AnimationConfigFocus::Rows => AnimationConfigFocus::Cols,
                AnimationConfigFocus::Cols => AnimationConfigFocus::HorizontalBleed,
                AnimationConfigFocus::HorizontalBleed => AnimationConfigFocus::VerticalBleed,
                AnimationConfigFocus::VerticalBleed => AnimationConfigFocus::Create,
                AnimationConfigFocus::Create => AnimationConfigFocus::Create,
            };
        }
        KeyCode::Up | KeyCode::Char('k') => match animation_config.focus {
            AnimationConfigFocus::Fps => {
                animation_config.fps = animation_config.fps.saturating_add(1).clamp(1, 30)
            }
            AnimationConfigFocus::Rows => {
                animation_config.rows = animation_config.rows.saturating_add(1).max(1)
            }
            AnimationConfigFocus::Cols => {
                animation_config.cols = animation_config.cols.saturating_add(1).max(1)
            }
            AnimationConfigFocus::HorizontalBleed => {
                animation_config.horizontal_bleed =
                    next_bleed_level(animation_config.horizontal_bleed)
            }
            AnimationConfigFocus::VerticalBleed => {
                animation_config.vertical_bleed = next_bleed_level(animation_config.vertical_bleed)
            }
            _ => {}
        },
        KeyCode::Down | KeyCode::Char('j') => match animation_config.focus {
            AnimationConfigFocus::Fps => {
                animation_config.fps = animation_config.fps.saturating_sub(1).clamp(1, 30)
            }
            AnimationConfigFocus::Rows => {
                animation_config.rows = animation_config.rows.saturating_sub(1).max(1)
            }
            AnimationConfigFocus::Cols => {
                animation_config.cols = animation_config.cols.saturating_sub(1).max(1)
            }
            AnimationConfigFocus::HorizontalBleed => {
                animation_config.horizontal_bleed =
                    previous_bleed_level(animation_config.horizontal_bleed)
            }
            AnimationConfigFocus::VerticalBleed => {
                animation_config.vertical_bleed =
                    previous_bleed_level(animation_config.vertical_bleed)
            }
            _ => {}
        },
        KeyCode::Enter => {
            app.start_animation_create(animation_config);
            return Ok(true);
        }
        _ => {}
    }
    app.glyph_tool_mode = GlyphToolMode::ConfigureAnimation(animation_config);
    Ok(true)
}

fn handle_glyphs_key(app: &mut App, key: KeyEvent) -> Result<()> {
    let code = key.code;
    app.normalize_glyphs_focus();

    if is_keypad_plus_alias(&key) {
        adjust_selected_threshold_by(app, 1);
        return Ok(());
    }
    if is_keypad_minus_alias(&key) {
        adjust_selected_threshold_by(app, -1);
        return Ok(());
    }

    if let Some(mut config) = app.grid_config.clone() {
        let res = handle_grid_config_key(app, &mut config, key);
        app.grid_config = if app.grid_config.is_some() {
            Some(config)
        } else {
            None
        };
        return res;
    }

    if let GlyphToolMode::ChooseAnimationType { mut focus } = app.glyph_tool_mode.clone() {
        match code {
            KeyCode::Esc | KeyCode::Char('q') => {
                app.glyph_tool_mode = GlyphToolMode::None;
                app.clear_animation_draft();
                app.status = Some("animation creation canceled".to_string());
            }
            KeyCode::Left | KeyCode::Char('h') => {
                focus = AnimationTypeChoiceFocus::Standard;
                app.glyph_tool_mode = GlyphToolMode::ChooseAnimationType { focus };
            }
            KeyCode::Right | KeyCode::Char('l') => {
                focus = AnimationTypeChoiceFocus::Grid;
                app.glyph_tool_mode = GlyphToolMode::ChooseAnimationType { focus };
            }
            KeyCode::Enter => {
                let animation_type = match focus {
                    AnimationTypeChoiceFocus::Standard => AnimationType::Standard,
                    AnimationTypeChoiceFocus::Grid => AnimationType::Grid,
                };
                app.glyph_tool_mode = GlyphToolMode::SelectAnimationFrames(animation_type);
                app.selecting_for_animation_frames = true;
                app.status = Some(
                    "Select imported frame glyphs with Space, then Enter to configure".to_string(),
                );
            }
            _ => {}
        }
        return Ok(());
    }

    if let GlyphToolMode::ImportAnimationFrames = app.glyph_tool_mode.clone() {
        match code {
            KeyCode::Esc | KeyCode::Char('q') => {
                if app.animation_import_task.is_some() {
                    app.status = Some("animation frames are still loading".to_string());
                    return Ok(());
                }
                app.glyph_tool_mode = GlyphToolMode::None;
                app.clear_animation_draft();
                app.status = Some("animation import canceled".to_string());
            }
            KeyCode::Enter => {
                if app.animation_import_task.is_some() {
                    app.status = Some("animation frames are still loading".to_string());
                    return Ok(());
                }
                app.glyph_tool_mode = GlyphToolMode::ChooseAnimationType {
                    focus: AnimationTypeChoiceFocus::Standard,
                };
                app.status = Some("Choose animation type".to_string());
            }
            _ => {}
        }
        return Ok(());
    }

    if let GlyphToolMode::SelectAnimationFrames(animation_type) = app.glyph_tool_mode.clone() {
        match code {
            KeyCode::Esc | KeyCode::Char('q') => {
                app.glyph_tool_mode = GlyphToolMode::None;
                app.clear_animation_draft();
                app.status = Some("animation frame selection canceled".to_string());
            }
            KeyCode::Char(' ') => {
                if let Some(selected_source_key) = selected_source_parent_key(app) {
                    if !app.animation_imported_set.contains(&selected_source_key) {
                        app.status = Some(
                            "only media imported in this animation flow can be used as frames"
                                .to_string(),
                        );
                        return Ok(());
                    }
                    if app.animation_selection_set.contains(&selected_source_key) {
                        app.animation_selection_set.remove(&selected_source_key);
                        app.animation_selection_order
                            .retain(|k| k != &selected_source_key);
                    } else {
                        app.animation_selection_set
                            .insert(selected_source_key.clone());
                        app.animation_selection_order.push(selected_source_key);
                    }
                }
            }
            KeyCode::Enter => {
                if app.animation_selection_order.is_empty() {
                    app.status = Some("select at least one frame".to_string());
                } else {
                    app.start_animation_config(animation_type);
                    app.selecting_for_animation_frames = false;
                }
            }
            _ => {}
        }
        return Ok(());
    }

    if let GlyphToolMode::ConfigureAnimation(animation_config) = app.glyph_tool_mode.clone() {
        handle_animation_config_key(app, key, animation_config)?;
        return Ok(());
    }

    match code {
        KeyCode::Esc => {
            if app.selecting_for_grid {
                app.selecting_for_grid = false;
                app.status = Some("grid selection canceled".to_string());
            } else if app.selecting_for_animation_frames {
                app.selecting_for_animation_frames = false;
                app.glyph_tool_mode = GlyphToolMode::None;
                app.clear_animation_draft();
                app.status = Some("animation selection canceled".to_string());
            } else if app.glyphs_focus == GlyphsFocus::Preview {
                app.glyphs_focus = GlyphsFocus::List;
            } else {
                app.quit = true;
            }
        }
        KeyCode::Char('q') => {
            app.quit = true;
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if app.glyphs_focus == GlyphsFocus::InstallButton {
                app.glyphs_focus = GlyphsFocus::List;
            } else if app.glyphs_focus == GlyphsFocus::List {
                let row_count = app.visible_glyph_rows().len();
                if row_count > 0 {
                    app.selected_visible = (app.selected_visible + 1).min(row_count - 1);
                    app.clamp_glyph_selection();
                }
            } else {
                match app.glyph_preview_control {
                    GlyphPreviewControl::Threshold => {
                        if let Some(value) = selected_row_threshold_value(app) {
                            set_selected_threshold(app, value.saturating_sub(1));
                        }
                    }
                    GlyphPreviewControl::Fps => {
                        if let Some(value) = selected_row_fps_value(app) {
                            set_selected_animation_fps(app, value.saturating_sub(1).clamp(1, 30));
                        }
                    }
                    GlyphPreviewControl::Invert => {
                        toggle_selected_invert(app);
                    }
                }
            }
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if app.glyphs_focus == GlyphsFocus::List {
                if app.selected_visible == 0 {
                    app.glyphs_focus = GlyphsFocus::InstallButton;
                } else {
                    app.selected_visible = app.selected_visible.saturating_sub(1);
                    app.clamp_glyph_selection();
                }
            } else if app.glyphs_focus == GlyphsFocus::InstallButton {
                // keep focus on the button
            } else {
                match app.glyph_preview_control {
                    GlyphPreviewControl::Threshold => {
                        if let Some(value) = selected_row_threshold_value(app) {
                            set_selected_threshold(app, value.saturating_add(1));
                        }
                    }
                    GlyphPreviewControl::Fps => {
                        if let Some(value) = selected_row_fps_value(app) {
                            set_selected_animation_fps(app, value.saturating_add(1).clamp(1, 30));
                        }
                    }
                    GlyphPreviewControl::Invert => {
                        toggle_selected_invert(app);
                    }
                }
            }
        }
        KeyCode::Left | KeyCode::Char('h') | KeyCode::Char('-') => {
            if matches!(code, KeyCode::Char('-')) {
                adjust_selected_threshold_by(app, -1);
            } else if app.glyphs_focus == GlyphsFocus::InstallButton {
                // no-op while install button is focused
            } else if app.glyphs_focus == GlyphsFocus::List {
                if matches!(
                    app.selected_visible_row(),
                    Some(VisibleGlyphRow::CompositionChild { .. })
                ) {
                    app.status = Some(
                        "grid tile thresholds are disabled; edit the grid parent instead"
                            .to_string(),
                    );
                } else if selected_row_supports_threshold(app)
                    || selected_row_supports_fps(app)
                    || selected_row_supports_invert(app)
                {
                    app.glyphs_focus = GlyphsFocus::Preview;
                    if let Some(leftmost) = preview_leftmost_control(
                        selected_row_supports_threshold(app),
                        selected_row_supports_fps(app),
                        selected_row_supports_invert(app),
                    ) {
                        app.glyph_preview_control = leftmost;
                    }
                } else if app.active_project.is_none() {
                    adjust_selected_threshold_by(app, -1);
                }
            } else {
                let controls = preview_controls_for_row(
                    selected_row_supports_threshold(app),
                    selected_row_supports_fps(app),
                    selected_row_supports_invert(app),
                );
                let Some(current_idx) = controls
                    .iter()
                    .position(|control| *control == app.glyph_preview_control)
                else {
                    app.glyphs_focus = GlyphsFocus::List;
                    return Ok(());
                };
                if current_idx == 0 {
                    app.glyphs_focus = GlyphsFocus::List;
                } else {
                    app.glyph_preview_control = controls[current_idx - 1];
                }
            }
        }
        KeyCode::Right | KeyCode::Char('l') | KeyCode::Char('+') | KeyCode::Char('=') => {
            if matches!(code, KeyCode::Char('+') | KeyCode::Char('=')) {
                adjust_selected_threshold_by(app, 1);
            } else if app.glyphs_focus == GlyphsFocus::InstallButton {
                // no-op while install button is focused
            } else if app.glyphs_focus == GlyphsFocus::List {
                if matches!(
                    app.selected_visible_row(),
                    Some(VisibleGlyphRow::CompositionChild { .. })
                ) {
                    app.status = Some(
                        "grid tile thresholds are disabled; edit the grid parent instead"
                            .to_string(),
                    );
                } else if selected_row_supports_threshold(app)
                    || selected_row_supports_fps(app)
                    || selected_row_supports_invert(app)
                {
                    app.glyphs_focus = GlyphsFocus::Preview;
                    if let Some(leftmost) = preview_leftmost_control(
                        selected_row_supports_threshold(app),
                        selected_row_supports_fps(app),
                        selected_row_supports_invert(app),
                    ) {
                        app.glyph_preview_control = leftmost;
                    }
                } else if app.active_project.is_none() {
                    adjust_selected_threshold_by(app, 1);
                }
            } else {
                let controls = preview_controls_for_row(
                    selected_row_supports_threshold(app),
                    selected_row_supports_fps(app),
                    selected_row_supports_invert(app),
                );
                if let Some(current_idx) = controls
                    .iter()
                    .position(|control| *control == app.glyph_preview_control)
                {
                    if let Some(next) = controls.get(current_idx + 1) {
                        app.glyph_preview_control = *next;
                    }
                } else if let Some(first) = controls.first() {
                    app.glyph_preview_control = *first;
                }
            }
        }
        KeyCode::Enter | KeyCode::Char(' ') => {
            if app.glyphs_focus == GlyphsFocus::InstallButton {
                app.view = AppView::Welcome;
                app.welcome_focus = WelcomeFocus::InstallButton;
                app.welcome_input_editing = false;
                trigger_install_action(app)?;
                return Ok(());
            }
            if app.glyphs_focus == GlyphsFocus::Preview
                && app.glyph_preview_control == GlyphPreviewControl::Invert
            {
                toggle_selected_invert(app);
                return Ok(());
            }
            if app.selecting_for_grid {
                if matches!(
                    app.selected_visible_row(),
                    Some(VisibleGlyphRow::CompositionChild { .. })
                ) {
                    app.status = Some(
                        "Select a standalone glyph or composition parent (children cannot be selected)"
                            .to_string(),
                    );
                    return Ok(());
                }
                if let Some(selected_source_key) = selected_source_parent_key(app) {
                    let source_key = if app.config.compositions.contains_key(&selected_source_key) {
                        duplicate_selected_parent_source_for_grid(app, &selected_source_key)?
                    } else {
                        selected_source_key
                    };
                    app.grid_config = Some(GridConfig {
                        source_key,
                        rows: 2,
                        cols: 2,
                        horizontal_bleed: BleedLevel::Weak,
                        vertical_bleed: BleedLevel::Off,
                        focus: GridConfigFocus::Rows,
                    });
                    app.selecting_for_grid = false;
                    app.status = Some(
                        "Configure grid: use arrows to change rows/cols, Right to Create"
                            .to_string(),
                    );
                }
            } else {
                app.toggle_selected_composition_expansion();
            }
        }
        KeyCode::Char('c') => {
            apply_default_composition_to_selected(app)?;
        }
        KeyCode::Char('C') => {
            clear_selected_composition(app)?;
        }
        KeyCode::Char('D') => {
            if let Some(
                VisibleGlyphRow::AnimationParent { animation_idx }
                | VisibleGlyphRow::AnimationFrame { animation_idx, .. },
            ) = app.selected_visible_row()
            {
                if let Some(target) = app
                    .config
                    .animations
                    .get(animation_idx)
                    .map(|animation| animation.name.clone())
                    && remove_animation_definition(&app.manifest_path, &target)?
                {
                    app.reload_glyphs()?;
                    app.refresh_workspace_discovery()?;
                    app.status = Some(format!("deleted animation `{target}`"));
                }
                return Ok(());
            }
            let Some(source_key) = selected_source_parent_key(app) else {
                app.status = Some("no glyph selected".to_string());
                return Ok(());
            };
            let matches = app
                .config
                .animations
                .iter()
                .filter(|a| a.frames.iter().any(|f| f == &source_key))
                .map(|a| a.name.clone())
                .collect::<Vec<_>>();
            if matches.is_empty() {
                app.status = Some("no animation linked to selected glyph".to_string());
            } else {
                let target = matches[0].clone();
                if remove_animation_definition(&app.manifest_path, &target)? {
                    app.reload_glyphs()?;
                    app.refresh_workspace_discovery()?;
                    app.status = Some(format!("deleted animation `{target}`"));
                }
            }
        }
        KeyCode::Char('R') => {
            if app.install_in_progress() {
                app.status =
                    Some("a background task is in progress; wait before rescanning".to_string());
                return Ok(());
            }
            app.refresh_workspace_discovery()?;
            app.refresh_pua_usage_summary();
            if app.active_project.is_some() {
                app.reload_glyphs()?;
            }
        }
        KeyCode::Char('i') => {
            trigger_install_action(app)?;
        }
        KeyCode::PageUp => {
            if let Some(threshold) = selected_row_threshold_value(app) {
                let next = threshold.saturating_add(10);
                set_selected_threshold(app, next);
            }
        }
        KeyCode::PageDown => {
            if let Some(threshold) = selected_row_threshold_value(app) {
                let next = threshold.saturating_sub(10);
                set_selected_threshold(app, next);
            }
        }
        KeyCode::Char('r') => {
            remove_selected_threshold_override(app);
        }
        _ => {}
    }
    Ok(())
}

fn handle_rename_mode_key(app: &mut App, code: KeyCode) -> Result<()> {
    match code {
        KeyCode::Esc => {
            app.renaming_input = None;
            app.renaming_original = None;
            app.status = Some("rename canceled".to_string());
        }
        KeyCode::Enter => {
            app.confirm_rename()?;
        }
        KeyCode::Char(ch) if is_valid_project_name_char(ch) => {
            if let Some(input) = app.renaming_input.as_mut() {
                input.handle_event(&Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
            }
        }
        KeyCode::Backspace
        | KeyCode::Delete
        | KeyCode::Left
        | KeyCode::Right
        | KeyCode::Home
        | KeyCode::End => {
            if let Some(input) = app.renaming_input.as_mut() {
                input.handle_event(&Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
            }
        }
        _ => {}
    }
    Ok(())
}

fn handle_welcome_key(app: &mut App, key: KeyEvent) -> Result<()> {
    let code = key.code;
    let home_project_actions_enabled = app.active_project.is_some();
    tui_debug_log(
        "welcome.handle.enter",
        format!("{} {}", key_debug(&key), app_debug_state(app)),
    );
    if app.delete_project_confirm_selection.is_some() {
        return handle_delete_project_confirmation_key(app, code);
    }
    if !matches!(app.home_workflow, HomeWorkflow::Launcher) {
        return handle_home_creation_key(app, key);
    }
    if app.renaming_input.is_some() {
        return handle_rename_mode_key(app, code);
    }
    if app.project_switch_task.is_some() && !matches!(code, KeyCode::Esc | KeyCode::Char('q')) {
        app.status = Some("project switch in progress...".to_string());
        return Ok(());
    }
    match code {
        KeyCode::Esc => {
            tui_debug_log("welcome.esc.before", app_debug_state(app));
            if app.welcome_input_editing {
                app.welcome_input_editing = false;
                app.status = None;
                tui_debug_log("welcome.esc.unfocus_input", app_debug_state(app));
            } else {
                app.quit = true;
                tui_debug_log("welcome.esc.quit", app_debug_state(app));
            }
        }
        KeyCode::Char('q') if !app.welcome_input_editing => {
            app.quit = true;
        }
        KeyCode::Char('1') if !app.welcome_input_editing && app.active_project.is_some() => {
            app.start_home_workflow(HomeCreationKind::Glyph);
            app.status = Some(
                "create glyph: import image(s), press Enter for tweaking, then continue"
                    .to_string(),
            );
        }
        KeyCode::Char('2') if !app.welcome_input_editing && app.active_project.is_some() => {
            app.start_home_workflow(HomeCreationKind::Grid);
            app.status = Some(
                "create grid: drop one image, Enter for tweaking, then configure grid".to_string(),
            );
        }
        KeyCode::Char('3') if !app.welcome_input_editing && app.active_project.is_some() => {
            app.start_home_workflow(HomeCreationKind::AnimatedGlyph);
            app.status = Some(
                "create animated glyph: import frame media, Enter for tweaking, then configure"
                    .to_string(),
            );
        }
        KeyCode::Char('4') if !app.welcome_input_editing && app.active_project.is_some() => {
            app.start_home_workflow(HomeCreationKind::AnimatedGridGlyph);
            app.status = Some(
                "create animated grid glyph: import frame media, Enter for tweaking, then configure"
                    .to_string(),
            );
        }
        KeyCode::Char('R') if !app.welcome_input_editing => {
            if app.install_in_progress() {
                app.status =
                    Some("a background task is in progress; wait before rescanning".to_string());
                return Ok(());
            }
            app.refresh_workspace_discovery()?;
            app.refresh_pua_usage_summary();
            if app.active_project.is_some() {
                app.reload_glyphs()?;
            }
        }
        KeyCode::Char('i') if !app.welcome_input_editing => {
            trigger_install_action(app)?;
        }
        KeyCode::Up | KeyCode::Char('k') if !app.welcome_input_editing => {
            app.welcome_focus = match app.welcome_focus {
                WelcomeFocus::VerbosePathsToggle => WelcomeFocus::VerbosePathsToggle,
                WelcomeFocus::ProjectList => {
                    if app.selected_project > 0 {
                        app.selected_project -= 1;
                        WelcomeFocus::ProjectList
                    } else {
                        WelcomeFocus::VerbosePathsToggle
                    }
                }
                WelcomeFocus::CreateInput if !app.projects.is_empty() => {
                    app.selected_project = app.projects.len() - 1;
                    WelcomeFocus::ProjectList
                }
                WelcomeFocus::CreateInput => WelcomeFocus::VerbosePathsToggle,
                WelcomeFocus::HomeCreateButtons => match app.home_launcher_focus {
                    HomeLauncherFocus::CreateGlyph => WelcomeFocus::InstallButton,
                    HomeLauncherFocus::CreateGrid => WelcomeFocus::DeleteProjectButton,
                    HomeLauncherFocus::CreateAnimatedGlyph => {
                        app.home_launcher_focus = HomeLauncherFocus::CreateGlyph;
                        WelcomeFocus::HomeCreateButtons
                    }
                    HomeLauncherFocus::CreateAnimatedGridGlyph => {
                        app.home_launcher_focus = HomeLauncherFocus::CreateGrid;
                        WelcomeFocus::HomeCreateButtons
                    }
                },
                WelcomeFocus::BuildButton => WelcomeFocus::VerbosePathsToggle,
                WelcomeFocus::InstallButton => WelcomeFocus::VerbosePathsToggle,
                WelcomeFocus::DeleteProjectButton => WelcomeFocus::VerbosePathsToggle,
                WelcomeFocus::InstalledFontList => {
                    if app.selected_installed_font_sub_index > 0 {
                        app.selected_installed_font_sub_index -= 1;
                        app.installed_font_horizontal_focus_uninstall = false;
                        WelcomeFocus::InstalledFontList
                    } else if app.selected_installed_font > 0 {
                        app.selected_installed_font -= 1;
                        app.selected_installed_font_sub_index = app
                            .installed_font_sub_row_count(app.selected_installed_font)
                            .saturating_sub(1);
                        app.installed_font_horizontal_focus_uninstall = false;
                        WelcomeFocus::InstalledFontList
                    } else if app.active_project.is_some() {
                        WelcomeFocus::InstallButton
                    } else if !app.projects.is_empty() {
                        WelcomeFocus::ProjectList
                    } else {
                        WelcomeFocus::VerbosePathsToggle
                    }
                }
            };
        }
        KeyCode::Down | KeyCode::Char('j') if !app.welcome_input_editing => {
            app.welcome_focus = match app.welcome_focus {
                WelcomeFocus::VerbosePathsToggle => {
                    if app.active_project.is_some() {
                        WelcomeFocus::InstallButton
                    } else if !app.projects.is_empty() {
                        WelcomeFocus::ProjectList
                    } else {
                        WelcomeFocus::CreateInput
                    }
                }
                WelcomeFocus::ProjectList => {
                    if app.selected_project + 1 < app.projects.len() {
                        app.selected_project += 1;
                        WelcomeFocus::ProjectList
                    } else {
                        WelcomeFocus::CreateInput
                    }
                }
                WelcomeFocus::CreateInput => {
                    if !app.installed_fonts.is_empty() {
                        app.selected_installed_font = 0;
                        app.selected_installed_font_sub_index = 0;
                        app.installed_font_horizontal_focus_uninstall = false;
                        WelcomeFocus::InstalledFontList
                    } else {
                        WelcomeFocus::CreateInput
                    }
                }
                WelcomeFocus::BuildButton => {
                    if app.active_project.is_some() {
                        app.home_launcher_focus = HomeLauncherFocus::CreateGlyph;
                        WelcomeFocus::HomeCreateButtons
                    } else if app.installed_fonts.is_empty() {
                        WelcomeFocus::BuildButton
                    } else {
                        app.selected_installed_font = 0;
                        app.selected_installed_font_sub_index = 0;
                        app.installed_font_horizontal_focus_uninstall = false;
                        WelcomeFocus::InstalledFontList
                    }
                }
                WelcomeFocus::InstallButton => {
                    if app.active_project.is_some() {
                        app.home_launcher_focus = HomeLauncherFocus::CreateGlyph;
                        WelcomeFocus::HomeCreateButtons
                    } else if app.installed_fonts.is_empty() {
                        WelcomeFocus::InstallButton
                    } else {
                        app.selected_installed_font = 0;
                        app.selected_installed_font_sub_index = 0;
                        app.installed_font_horizontal_focus_uninstall = false;
                        WelcomeFocus::InstalledFontList
                    }
                }
                WelcomeFocus::DeleteProjectButton => {
                    if app.active_project.is_some() {
                        app.home_launcher_focus = HomeLauncherFocus::CreateGrid;
                        WelcomeFocus::HomeCreateButtons
                    } else if app.installed_fonts.is_empty() {
                        WelcomeFocus::DeleteProjectButton
                    } else {
                        app.selected_installed_font = 0;
                        app.selected_installed_font_sub_index = 0;
                        app.installed_font_horizontal_focus_uninstall = false;
                        WelcomeFocus::InstalledFontList
                    }
                }
                WelcomeFocus::HomeCreateButtons => match app.home_launcher_focus {
                    HomeLauncherFocus::CreateGlyph => {
                        app.home_launcher_focus = HomeLauncherFocus::CreateAnimatedGlyph;
                        WelcomeFocus::HomeCreateButtons
                    }
                    HomeLauncherFocus::CreateGrid => {
                        app.home_launcher_focus = HomeLauncherFocus::CreateAnimatedGridGlyph;
                        WelcomeFocus::HomeCreateButtons
                    }
                    HomeLauncherFocus::CreateAnimatedGlyph
                    | HomeLauncherFocus::CreateAnimatedGridGlyph => {
                        if app.installed_fonts.is_empty() {
                            WelcomeFocus::HomeCreateButtons
                        } else {
                            app.selected_installed_font = 0;
                            app.selected_installed_font_sub_index = 0;
                            app.installed_font_horizontal_focus_uninstall = false;
                            WelcomeFocus::InstalledFontList
                        }
                    }
                },
                WelcomeFocus::InstalledFontList => {
                    if app.installed_font_horizontal_focus_uninstall {
                        // Pressing down on Uninstall button goes to sample line (sub-index 1)
                        app.installed_font_horizontal_focus_uninstall = false;
                        app.selected_installed_font_sub_index = 1.min(
                            app.installed_font_sub_row_count(app.selected_installed_font)
                                .saturating_sub(1),
                        );
                        WelcomeFocus::InstalledFontList
                    } else {
                        let sub_count =
                            app.installed_font_sub_row_count(app.selected_installed_font);
                        if app.selected_installed_font_sub_index + 1 < sub_count {
                            app.selected_installed_font_sub_index += 1;
                            WelcomeFocus::InstalledFontList
                        } else if app.selected_installed_font + 1 < app.installed_fonts.len() {
                            app.selected_installed_font += 1;
                            app.selected_installed_font_sub_index = 0;
                            WelcomeFocus::InstalledFontList
                        } else {
                            WelcomeFocus::InstalledFontList
                        }
                    }
                }
            };
        }
        KeyCode::Left | KeyCode::Char('h') if !app.welcome_input_editing => {
            app.welcome_focus = match app.welcome_focus {
                WelcomeFocus::VerbosePathsToggle => {
                    if app.projects.is_empty() {
                        WelcomeFocus::CreateInput
                    } else {
                        app.selected_project = 0;
                        WelcomeFocus::ProjectList
                    }
                }
                WelcomeFocus::BuildButton => WelcomeFocus::CreateInput,
                WelcomeFocus::InstallButton => WelcomeFocus::CreateInput,
                WelcomeFocus::DeleteProjectButton => WelcomeFocus::InstallButton,
                WelcomeFocus::HomeCreateButtons => match app.home_launcher_focus {
                    HomeLauncherFocus::CreateGlyph | HomeLauncherFocus::CreateAnimatedGlyph => {
                        WelcomeFocus::CreateInput
                    }
                    HomeLauncherFocus::CreateGrid => {
                        app.home_launcher_focus = HomeLauncherFocus::CreateGlyph;
                        WelcomeFocus::HomeCreateButtons
                    }
                    HomeLauncherFocus::CreateAnimatedGridGlyph => {
                        app.home_launcher_focus = HomeLauncherFocus::CreateAnimatedGlyph;
                        WelcomeFocus::HomeCreateButtons
                    }
                },
                WelcomeFocus::InstalledFontList => {
                    if app.selected_installed_font_sub_index == 0 {
                        app.installed_font_horizontal_focus_uninstall = false;
                    }
                    WelcomeFocus::InstalledFontList
                }
                WelcomeFocus::ProjectList => WelcomeFocus::ProjectList,
                WelcomeFocus::CreateInput => WelcomeFocus::CreateInput,
            };
        }
        KeyCode::Right | KeyCode::Char('l') if !app.welcome_input_editing => {
            app.welcome_focus = match app.welcome_focus {
                WelcomeFocus::VerbosePathsToggle => WelcomeFocus::VerbosePathsToggle,
                WelcomeFocus::CreateInput => {
                    if home_project_actions_enabled {
                        app.home_launcher_focus = HomeLauncherFocus::CreateAnimatedGlyph;
                        WelcomeFocus::HomeCreateButtons
                    } else {
                        WelcomeFocus::VerbosePathsToggle
                    }
                }
                WelcomeFocus::ProjectList => {
                    if home_project_actions_enabled {
                        WelcomeFocus::InstallButton
                    } else {
                        WelcomeFocus::ProjectList
                    }
                }
                WelcomeFocus::BuildButton => {
                    if home_project_actions_enabled {
                        WelcomeFocus::InstallButton
                    } else {
                        WelcomeFocus::CreateInput
                    }
                }
                WelcomeFocus::InstallButton => {
                    if !home_project_actions_enabled {
                        WelcomeFocus::CreateInput
                    } else if app.active_project_can_be_deleted() {
                        WelcomeFocus::DeleteProjectButton
                    } else {
                        WelcomeFocus::InstallButton
                    }
                }
                WelcomeFocus::DeleteProjectButton => WelcomeFocus::DeleteProjectButton,
                WelcomeFocus::HomeCreateButtons => {
                    app.home_launcher_focus = match app.home_launcher_focus {
                        HomeLauncherFocus::CreateGlyph => HomeLauncherFocus::CreateGrid,
                        HomeLauncherFocus::CreateGrid => HomeLauncherFocus::CreateGrid,
                        HomeLauncherFocus::CreateAnimatedGlyph => {
                            HomeLauncherFocus::CreateAnimatedGridGlyph
                        }
                        HomeLauncherFocus::CreateAnimatedGridGlyph => {
                            HomeLauncherFocus::CreateAnimatedGridGlyph
                        }
                    };
                    WelcomeFocus::HomeCreateButtons
                }
                WelcomeFocus::InstalledFontList => {
                    if app.selected_installed_font_sub_index == 0 {
                        app.installed_font_horizontal_focus_uninstall = true;
                    }
                    WelcomeFocus::InstalledFontList
                }
            };
        }
        KeyCode::Enter => match app.welcome_focus {
            WelcomeFocus::VerbosePathsToggle => {
                app.welcome_input_editing = false;
                app.verbose_paths = !app.verbose_paths;
                app.status = Some(format!(
                    "verbose paths {}",
                    if app.verbose_paths {
                        "enabled"
                    } else {
                        "disabled"
                    }
                ));
            }
            WelcomeFocus::ProjectList => {
                app.welcome_input_editing = false;
                if app.project_switch_task.is_some() {
                    app.status = Some("project switch in progress...".to_string());
                    return Ok(());
                }
                if let Some(project) = app.projects.get(app.selected_project) {
                    if project.manifest_warning.is_some() {
                        app.status = Some(
                            "cannot open project: manifest is malformed; fix petiglyph.toml"
                                .to_string(),
                        );
                        return Ok(());
                    }
                    let is_active = app
                        .active_project
                        .as_ref()
                        .is_some_and(|active| active == &project.manifest_path);
                    if is_active {
                        app.renaming_input = Some(Input::new(app.config.font_name.clone()));
                        app.renaming_original = Some(app.config.font_name.clone());
                        app.status = Some("renaming project...".to_string());
                    } else {
                        app.start_project_switch_task(
                            project.manifest_path.clone(),
                            project.font_name.clone(),
                        )?;
                    }
                }
            }
            WelcomeFocus::CreateInput => {
                if app.welcome_input_editing {
                    app.welcome_input_editing = false;
                    app.status = None;
                    if !app.create_input.value().trim().is_empty() {
                        app.submit_create()?;
                    }
                } else {
                    app.welcome_input_editing = true;
                    app.status = None;
                }
            }
            WelcomeFocus::BuildButton => {
                app.welcome_input_editing = false;
                trigger_install_action(app)?;
            }
            WelcomeFocus::InstallButton => {
                app.welcome_input_editing = false;
                trigger_install_action(app)?;
            }
            WelcomeFocus::DeleteProjectButton => {
                app.welcome_input_editing = false;
                app.begin_delete_project_confirmation()?;
            }
            WelcomeFocus::HomeCreateButtons => {
                let kind = match app.home_launcher_focus {
                    HomeLauncherFocus::CreateGlyph => HomeCreationKind::Glyph,
                    HomeLauncherFocus::CreateGrid => HomeCreationKind::Grid,
                    HomeLauncherFocus::CreateAnimatedGlyph => HomeCreationKind::AnimatedGlyph,
                    HomeLauncherFocus::CreateAnimatedGridGlyph => {
                        HomeCreationKind::AnimatedGridGlyph
                    }
                };
                app.start_home_workflow(kind);
            }
            WelcomeFocus::InstalledFontList => {
                app.welcome_input_editing = false;
                if app.installed_font_horizontal_focus_uninstall {
                    trigger_uninstall_action(app)?;
                } else {
                    // Copy to clipboard
                    if let Some(font) = app.installed_fonts.get(app.selected_installed_font) {
                        let content = if app.selected_installed_font_sub_index == 0 {
                            font.path.display().to_string()
                        } else {
                            let sample_count = font.blocks.len();
                            let sub = app.selected_installed_font_sub_index - 1;
                            if sub < sample_count {
                                font.blocks
                                    .get(sub)
                                    .map(|block| block.export.clone())
                                    .unwrap_or_default()
                            } else {
                                let anim_idx = sub - sample_count;
                                font.animation_exports
                                    .get(anim_idx)
                                    .cloned()
                                    .unwrap_or_default()
                            }
                        };

                        if !content.is_empty() {
                            match copy_to_clipboard(&content) {
                                Ok(()) => {
                                    let row_id = if app.selected_installed_font_sub_index == 0 {
                                        "path".to_string()
                                    } else {
                                        let sample_count = font.blocks.len();
                                        let sub = app.selected_installed_font_sub_index - 1;
                                        if sub < sample_count {
                                            format!("sample-{sub}")
                                        } else {
                                            format!("animation-{}", sub - sample_count)
                                        }
                                    };
                                    app.last_copy_notification = Some((
                                        Instant::now(),
                                        format!("{}-{}", app.selected_installed_font, row_id),
                                    ));
                                    app.status = Some("copied to clipboard".to_string());
                                }
                                Err(err) => {
                                    app.status = Some(format!("clipboard copy failed: {err}"));
                                }
                            }
                        }
                    }
                }
            }
        },
        _ => {
            if app.welcome_focus == WelcomeFocus::CreateInput && app.welcome_input_editing {
                if let KeyCode::Char(ch) = code
                    && !is_valid_project_name_char(ch)
                {
                    return Ok(());
                }
                app.create_input.handle_event(&Event::Key(key));
                tui_debug_log("welcome.input.handled_by_tui_input", app_debug_state(app));
            }
        }
    }
    tui_debug_log("welcome.handle.exit", app_debug_state(app));
    Ok(())
}

fn handle_delete_project_confirmation_key(app: &mut App, code: KeyCode) -> Result<()> {
    match code {
        KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('q') => {
            app.cancel_delete_project_confirmation();
        }
        KeyCode::Left | KeyCode::Char('h') => {
            if let Some(selection) = app.delete_project_confirm_selection.as_mut() {
                *selection = DELETE_CONFIRM_CANCEL_INDEX;
            }
            app.status = None;
        }
        KeyCode::Right | KeyCode::Char('l') => {
            if let Some(selection) = app.delete_project_confirm_selection.as_mut() {
                *selection = DELETE_CONFIRM_DELETE_INDEX;
            }
            app.status = None;
        }
        KeyCode::Enter | KeyCode::Char('y') => match app.delete_project_confirm_selection {
            Some(DELETE_CONFIRM_CANCEL_INDEX) => {
                app.cancel_delete_project_confirmation();
            }
            Some(DELETE_CONFIRM_DELETE_INDEX) => {
                app.confirm_delete_project()?;
            }
            Some(_) => {}
            None => {}
        },
        _ => {}
    }
    tui_debug_log("welcome.delete_confirm.exit", app_debug_state(app));
    Ok(())
}
