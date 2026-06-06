#[cfg(test)]
pub(crate) fn handle_key(app: &mut App, code: KeyCode) -> Result<()> {
    handle_key_event(
        app,
        KeyEvent::new(code, crossterm::event::KeyModifiers::NONE),
    )
}

#[cfg(test)]
pub(crate) fn handle_key_event_for_test(app: &mut App, key: KeyEvent) -> Result<()> {
    handle_key_event(app, key)
}

#[cfg(test)]
pub(crate) fn handle_paste_event_for_test(app: &mut App, payload: &str) -> Result<()> {
    handle_paste_event(app, payload)
}

#[cfg(test)]
pub(crate) fn render_ui_for_test(app: &App, width: u16, height: u16) -> Result<()> {
    let backend = ratatui::backend::TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).context("failed to initialize test terminal")?;
    terminal.draw(|frame| draw_ui(frame, app))?;
    Ok(())
}

fn is_keypad_plus_alias(key: &KeyEvent) -> bool {
    matches!(key.code, KeyCode::Char('k')) && key.state.contains(KeyEventState::KEYPAD)
}

fn is_keypad_minus_alias(key: &KeyEvent) -> bool {
    matches!(key.code, KeyCode::Char('m')) && key.state.contains(KeyEventState::KEYPAD)
}

fn adjust_selected_threshold_by(app: &mut App, step: i16) {
    if let Some(glyph) = selected_glyph(app) {
        let next = if step >= 0 {
            glyph.working_threshold.saturating_add(step as u8)
        } else {
            glyph.working_threshold.saturating_sub((-step) as u8)
        };
        set_selected_threshold(app, next);
    } else if app.active_project.is_none() {
        set_selected_threshold(app, 0);
    }
}

fn selected_row_supports_threshold(app: &App) -> bool {
    selected_threshold_sources(app).is_some()
}

fn selected_row_supports_fps(app: &App) -> bool {
    selected_animation_index(app).is_some()
}

fn preview_leftmost_control(
    supports_threshold: bool,
    supports_fps: bool,
    supports_invert: bool,
) -> Option<GlyphPreviewControl> {
    preview_controls_for_row(supports_threshold, supports_fps, supports_invert)
        .into_iter()
        .next()
}

fn preview_controls_for_row(
    supports_threshold: bool,
    supports_fps: bool,
    supports_invert: bool,
) -> Vec<GlyphPreviewControl> {
    let mut controls = Vec::new();
    if supports_threshold {
        controls.push(GlyphPreviewControl::Threshold);
    }
    if supports_fps {
        controls.push(GlyphPreviewControl::Fps);
    }
    if supports_invert {
        controls.push(GlyphPreviewControl::Invert);
    }
    controls
}

fn selected_row_threshold_value(app: &App) -> Option<u8> {
    let sources = selected_threshold_sources(app)?;
    let first = sources.first()?;
    app.glyphs
        .iter()
        .find(|glyph| glyph.glyph.source_parent_key == *first)
        .map(|glyph| glyph.working_threshold)
        .or(Some(app.config.base_threshold))
}

fn selected_row_fps_value(app: &App) -> Option<u8> {
    let idx = selected_animation_index(app)?;
    app.config
        .animations
        .get(idx)
        .map(|animation| animation.fps)
}

fn set_selected_animation_fps(app: &mut App, fps: u8) {
    let Some(idx) = selected_animation_index(app) else {
        app.status = Some("no animation selected".to_string());
        return;
    };
    let Some(animation_name) = app.config.animations.get(idx).map(|a| a.name.clone()) else {
        app.status = Some("no animation selected".to_string());
        return;
    };
    match persist_animation_fps(&app.manifest_path, &animation_name, fps) {
        Ok(true) => {
            if let Some(animation) = app.config.animations.get_mut(idx) {
                animation.fps = fps.clamp(1, 30);
            }
            app.status = Some(format!("updated animation `{animation_name}` fps -> {fps}"));
        }
        Ok(false) => {
            app.status = Some(format!("animation not found: `{animation_name}`"));
        }
        Err(err) => {
            app.status = Some(format!(
                "failed to update fps for `{animation_name}`: {err}"
            ));
        }
    }
}

fn handle_paste_event(app: &mut App, payload: &str) -> Result<()> {
    if !looks_like_path_payload(payload) {
        return Ok(());
    }
    app.import_dropped_images(payload)
}

fn handle_key_event(app: &mut App, key: KeyEvent) -> Result<()> {
    let code = key.code;
    tui_debug_log(
        "handle_key_event.enter",
        format!("{} {}", key_debug(&key), app_debug_state(app)),
    );
    if app.first_install_notice_open {
        return handle_first_install_notice_key(app, code);
    }
    if app.view == AppView::Welcome && app.delete_project_confirm_selection.is_some() {
        tui_debug_log(
            "handle_key_event.route_delete_confirm",
            app_debug_state(app),
        );
        let result = handle_welcome_key(app, key);
        tui_debug_log("handle_key_event.exit_delete_confirm", app_debug_state(app));
        return result;
    }
    if matches!(code, KeyCode::Char('v') | KeyCode::Char('V'))
        && !app.welcome_input_editing
        && app.renaming_input.is_none()
    {
        app.verbose_paths = !app.verbose_paths;
        app.status = Some(format!(
            "verbose paths {}",
            if app.verbose_paths {
                "enabled"
            } else {
                "disabled"
            }
        ));
        tui_debug_log("handle_key_event.exit_global", app_debug_state(app));
        return Ok(());
    }
    let is_global_panel_jump = matches!(code, KeyCode::Tab | KeyCode::BackTab)
        || (matches!(code, KeyCode::Char('1') | KeyCode::Char('2'))
            && !app.welcome_input_editing
            && app.renaming_input.is_none());

    if app.view == AppView::Welcome && !is_global_panel_jump {
        tui_debug_log("handle_key_event.route_welcome", app_debug_state(app));
        let result = handle_welcome_key(app, key);
        tui_debug_log("handle_key_event.exit_welcome", app_debug_state(app));
        return result;
    }

    if app.view == AppView::Glyphs && !is_global_panel_jump {
        tui_debug_log("handle_key_event.route_glyphs", app_debug_state(app));
        let result = handle_glyphs_key(app, key);
        tui_debug_log("handle_key_event.exit_glyphs", app_debug_state(app));
        return result;
    }

    match code {
        KeyCode::Esc | KeyCode::Char('q') => app.quit = true,
        KeyCode::Char('1') => {
            app.welcome_input_editing = false;
            if app.view == AppView::Glyphs && app.active_project.is_some() {
                app.welcome_focus = WelcomeFocus::InstallButton;
            }
            app.view = AppView::Welcome;
            app.grid_config = None;
            app.selecting_for_grid = false;
        }
        KeyCode::Char('2') => {
            app.welcome_input_editing = false;
            app.view = AppView::Glyphs;
            app.normalize_glyphs_focus();
        }
        KeyCode::Tab => {
            app.welcome_input_editing = false;
            app.view = match app.view {
                AppView::Welcome => AppView::Glyphs,
                AppView::Glyphs => AppView::Welcome,
            };
            if app.view == AppView::Glyphs {
                app.normalize_glyphs_focus();
            }
            if app.view == AppView::Welcome && app.active_project.is_some() {
                app.welcome_focus = WelcomeFocus::InstallButton;
            }
        }
        KeyCode::BackTab => {
            app.welcome_input_editing = false;
            app.view = match app.view {
                AppView::Welcome => AppView::Glyphs,
                AppView::Glyphs => AppView::Welcome,
            };
            if app.view == AppView::Glyphs {
                app.normalize_glyphs_focus();
            }
            if app.view == AppView::Welcome && app.active_project.is_some() {
                app.welcome_focus = WelcomeFocus::InstalledFontList;
            }
        }
        KeyCode::Char('R') => {
            if app.install_in_progress() {
                app.status =
                    Some("a background task is in progress; wait before rescanning".to_string());
                tui_debug_log("handle_key_event.exit_global", app_debug_state(app));
                return Ok(());
            }
            app.refresh_workspace_discovery()?;
            app.refresh_pua_usage_summary();
            app.reload_glyphs()?;
            app.view = if app.glyphs.is_empty() {
                AppView::Welcome
            } else {
                AppView::Glyphs
            };
        }
        KeyCode::Char('i') => {
            trigger_install_action(app)?;
        }
        KeyCode::Down => {
            if app.view == AppView::Glyphs {
                let row_count = app.visible_glyph_rows().len();
                if row_count > 0 {
                    app.selected_visible = (app.selected_visible + 1).min(row_count - 1);
                    app.clamp_glyph_selection();
                }
            }
        }
        KeyCode::Char('j') => {
            if app.view == AppView::Glyphs {
                let row_count = app.visible_glyph_rows().len();
                if row_count > 0 {
                    app.selected_visible = (app.selected_visible + 1).min(row_count - 1);
                    app.clamp_glyph_selection();
                }
            }
        }
        KeyCode::Up => {
            if app.view == AppView::Glyphs {
                app.selected_visible = app.selected_visible.saturating_sub(1);
                app.clamp_glyph_selection();
            }
        }
        KeyCode::Char('k') => {
            if app.view == AppView::Glyphs {
                app.selected_visible = app.selected_visible.saturating_sub(1);
                app.clamp_glyph_selection();
            }
        }
        KeyCode::Enter | KeyCode::Char(' ') => {
            if app.view == AppView::Glyphs {
                app.toggle_selected_composition_expansion();
            }
        }
        KeyCode::Char('c') => {
            if app.view == AppView::Glyphs {
                apply_default_composition_to_selected(app)?;
            }
        }
        KeyCode::Char('C') => {
            if app.view == AppView::Glyphs {
                clear_selected_composition(app)?;
            }
        }
        KeyCode::PageUp => {
            if app.view == AppView::Glyphs {
                if let Some(glyph) = selected_glyph(app) {
                    let next = glyph.working_threshold.saturating_add(10);
                    set_selected_threshold(app, next);
                } else if app.active_project.is_none() {
                    set_selected_threshold(app, 0);
                }
            }
        }
        KeyCode::PageDown => {
            if app.view == AppView::Glyphs {
                if let Some(glyph) = selected_glyph(app) {
                    let next = glyph.working_threshold.saturating_sub(10);
                    set_selected_threshold(app, next);
                } else if app.active_project.is_none() {
                    set_selected_threshold(app, 0);
                }
            }
        }
        KeyCode::Char('r') => {
            if app.view == AppView::Glyphs {
                remove_selected_threshold_override(app);
            }
        }
        _ => {}
    }
    tui_debug_log("handle_key_event.exit_global", app_debug_state(app));
    Ok(())
}

fn trigger_install_action(app: &mut App) -> Result<()> {
    app.start_install_font();
    Ok(())
}

fn trigger_uninstall_action(app: &mut App) -> Result<()> {
    app.start_uninstall_selected_installed_font()
}

fn draw_ui(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let accent = Color::Cyan;
    let muted = Color::DarkGray;

    if hty_full_repaint_enabled() {
        // hty-specific workaround: fully repaint every frame with plain ASCII
        // so terminal default-bg quirks do not leak black tiles.
        draw_ascii_full_repaint_frame(frame, area);
    } else {
        frame.render_widget(Clear, area);
    }

    if !terminal_size_supported(area) {
        draw_terminal_too_small(frame, area, accent, muted);
        return;
    }
    let area = centered_bounded_viewport(area);

    let root = if app.debug_enabled {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // Header
                Constraint::Min(0),    // Body
                Constraint::Length(1), // Footer keys
                Constraint::Length((DEBUG_LOG_VISIBLE_LINES as u16).saturating_add(2)),
            ])
            .split(area)
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // Header
                Constraint::Min(0),    // Body
                Constraint::Length(1), // Footer keys
            ])
            .split(area)
    };

    // Header
    let titles = [" 1 Home ", " 2 Glyphs "];
    let tabs = Tabs::new(titles.into_iter().map(Line::from).collect::<Vec<_>>())
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .title(Line::from(vec![
                    Span::styled(
                        " petiglyph ",
                        Style::default().fg(accent).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(format!(" v{} ", CLI_VERSION), Style::default().fg(muted)),
                ])),
        )
        .select(match app.view {
            AppView::Welcome => 0,
            AppView::Glyphs => 1,
        })
        .style(Style::default().fg(Color::White))
        .highlight_style(
            Style::default()
                .fg(Color::Black)
                .bg(accent)
                .add_modifier(Modifier::BOLD),
        )
        .divider("");

    frame.render_widget(tabs, root[0]);

    // Body
    let body_area = root[1];

    match app.view {
        AppView::Welcome => draw_welcome_view(frame, app, body_area, accent, muted),
        AppView::Glyphs => draw_glyphs_view(frame, app, body_area, accent, muted),
    }
    if app.view == AppView::Welcome {
        draw_home_creation_popup(frame, app, area, accent, muted);
    }
    draw_delete_project_confirmation_popup(frame, app, area, accent);
    draw_first_install_notice_popup(frame, app, area, accent, muted);

    // Footer
    let mut footer_spans =
        if app.welcome_input_editing
            || app.renaming_input.is_some()
            || app.delete_project_confirm_selection.is_some()
        {
            vec![
                Span::styled(" Enter ", Style::default().fg(accent)),
                Span::raw("confirm  "),
                Span::styled(" Esc ", Style::default().fg(accent)),
                Span::raw("cancel"),
            ]
        } else {
            let (navigate_label, action_label) = match app.view {
                AppView::Welcome => ("navigate  ", "action  "),
                AppView::Glyphs => ("glyph  ", "expand  "),
            };
            vec![
                Span::styled(" Tab ", Style::default().fg(accent)),
                Span::raw("panel  "),
                Span::styled(" \u{2191}/\u{2193} ", Style::default().fg(accent)),
                Span::raw(navigate_label),
                Span::styled(" Enter ", Style::default().fg(accent)),
                Span::raw(action_label),
                Span::styled(" q ", Style::default().fg(accent)),
                Span::raw("quit"),
            ]
        };

    if let Some(status) = &app.status {
        footer_spans.push(Span::styled(" | ", Style::default().fg(muted)));
        footer_spans.push(Span::styled(
            status.clone(),
            Style::default().fg(Color::LightRed),
        ));
    }

    if let (Some(spinner), Some(kind)) = (app.font_task_spinner_frame(), app.font_task_kind()) {
        footer_spans.push(Span::styled(" | ", Style::default().fg(muted)));
        footer_spans.push(Span::styled(
            format!("{spinner} {}", kind.footer_label()),
            if kind.is_uninstall() {
                Style::default().fg(Color::LightRed)
            } else {
                Style::default().fg(Color::Yellow)
            },
        ));
    }

    let footer = Paragraph::new(Line::from(footer_spans))
        .alignment(Alignment::Center)
        .style(Style::default().fg(muted));
    frame.render_widget(footer, root[2]);

    if app.debug_enabled {
        let debug_lines = if app.debug_log_lines.is_empty() {
            vec![Line::from(vec![Span::styled(
                "no debug pipeline logs yet",
                Style::default().fg(Color::DarkGray),
            )])]
        } else {
            app.debug_log_lines
                .iter()
                .map(|line| Line::from(Span::raw(line.clone())))
                .collect::<Vec<_>>()
        };
        let title = app
            .debug_log_path
            .as_ref()
            .map(|p| format!(" Debug Log ({}) ", p.display()))
            .unwrap_or_else(|| " Debug Log ".to_string());
        let debug_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::DarkGray))
            .title(Span::styled(title, Style::default().fg(Color::Cyan)));
        frame.render_widget(
            Paragraph::new(debug_lines)
                .block(debug_block)
                .wrap(Wrap { trim: false })
                .style(Style::default().fg(Color::Gray)),
            root[3],
        );
    }
}
