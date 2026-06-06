fn hty_full_repaint_enabled() -> bool {
    *HTY_FULL_REPAINT_ENABLED.get_or_init(|| {
        std::env::var_os(HTY_FULL_REPAINT_ENV)
            .map(|value| {
                let value = value.to_string_lossy();
                !(value.eq_ignore_ascii_case("0")
                    || value.eq_ignore_ascii_case("false")
                    || value.eq_ignore_ascii_case("off")
                    || value.is_empty())
            })
            .unwrap_or(false)
    })
}

fn draw_ascii_full_repaint_frame(frame: &mut Frame, area: Rect) {
    let width = usize::from(area.width);
    let height = usize::from(area.height);
    let line = " ".repeat(width);
    let lines = (0..height)
        .map(|_| Line::from(line.clone()))
        .collect::<Vec<_>>();
    let hty_bg = Color::Rgb(40, 44, 52);
    frame.render_widget(
        Paragraph::new(lines).style(Style::default().bg(hty_bg).fg(hty_bg)),
        area,
    );
}

fn draw_delete_project_confirmation_popup(frame: &mut Frame, app: &App, area: Rect, accent: Color) {
    let Some(selection) = app.delete_project_confirm_selection else {
        return;
    };

    let project_label = app
        .active_project
        .as_ref()
        .and_then(|manifest| manifest.parent())
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .unwrap_or("current project");
    let popup = centered_popup_rect(area, 94, 7);
    frame.render_widget(Clear, popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::LightRed))
        .title(Span::styled(
            " Confirm Deletion ",
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        ));
    let selected_style = Style::default()
        .fg(Color::Black)
        .bg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    let danger_style = Style::default()
        .fg(Color::White)
        .bg(Color::Red)
        .add_modifier(Modifier::BOLD);
    let idle_style = Style::default().fg(Color::White).bg(Color::DarkGray);
    let buttons_row = Line::from(vec![
        Span::styled(
            " CANCEL ",
            if selection == DELETE_CONFIRM_CANCEL_INDEX {
                selected_style
            } else {
                idle_style
            },
        ),
        Span::raw("  "),
        Span::styled(
            " DELETE ",
            if selection == DELETE_CONFIRM_DELETE_INDEX {
                danger_style
            } else {
                idle_style
            },
        ),
    ])
    .alignment(Alignment::Center);

    let lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::raw("  "),
            Span::styled(
                format!("Delete project `{project_label}`?"),
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(""),
        buttons_row,
        Line::from(""),
    ];
    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false }),
        popup,
    );
}

fn draw_first_install_notice_popup(
    frame: &mut Frame,
    app: &App,
    area: Rect,
    accent: Color,
    muted: Color,
) {
    if !app.first_install_notice_open {
        return;
    }

    let restart_target = detected_terminal_name()
        .map(|name| format!("all {name} terminals"))
        .unwrap_or_else(|| "all terminals".to_string());
    let popup = centered_popup_rect(area, 106, 16);
    frame.render_widget(Clear, popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::Yellow))
        .title(Span::styled(
            " First Install Guidance ",
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        ));
    let lines = vec![
        Line::from(""),
        Line::from(vec![Span::styled(
            format!("Restart {restart_target} to load the newly installed glyphs."),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(""),
        Line::from(Span::styled(
            "If glyphs still appear as errors or [?] after restarting the terminals, reboot your computer.",
            Style::default().fg(Color::White),
        )),
        Line::from(Span::styled(
            "This fully resets the processes that may still be using the old font state.",
            Style::default().fg(Color::White),
        )),
        Line::from(Span::styled(
            "Then relaunch petiglyph and check the sample/preview again.",
            Style::default().fg(Color::White),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Press Enter, Esc, or Space to dismiss this message.",
            Style::default().fg(muted),
        )),
    ];
    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true }),
        popup,
    );
}

fn format_status_from_error(manifest_path: &Path, error_text: &str) -> String {
    if let Some(warning) = incompatible_artifact_warning(error_text, Some(manifest_path)) {
        return warning;
    }
    error_text.to_string()
}

fn terminal_size_supported(area: Rect) -> bool {
    area.width >= TUI_MIN_WIDTH && area.height >= TUI_MIN_HEIGHT
}

fn centered_bounded_viewport(area: Rect) -> Rect {
    let viewport_width = area.width.min(TUI_MAX_WIDTH);
    let viewport_height = area.height.min(TUI_MAX_HEIGHT);
    let x = area.x + area.width.saturating_sub(viewport_width) / 2;
    let y = area.y + area.height.saturating_sub(viewport_height) / 2;
    Rect::new(x, y, viewport_width, viewport_height)
}

fn centered_popup_rect(area: Rect, max_width: u16, height: u16) -> Rect {
    let width = area
        .width
        .min(area.width.saturating_sub(6).min(max_width).max(42));
    let height = area
        .height
        .min(height.min(area.height.saturating_sub(2)).max(6));
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    Rect::new(x, y, width, height)
}

fn draw_terminal_too_small(frame: &mut Frame, area: Rect, accent: Color, muted: Color) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(muted))
        .title(Span::styled(
            " petiglyph ",
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        ));
    let text = vec![
        Line::from(""),
        Line::from(Span::styled(
            "Terminal too small",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(format!(
            "Need at least {}x{} (W x H)",
            TUI_MIN_WIDTH, TUI_MIN_HEIGHT
        )),
        Line::from(format!("Current: {}x{}", area.width, area.height)),
        Line::from(""),
        Line::from(Span::styled(
            "Resize terminal to continue.",
            Style::default().fg(muted),
        )),
        Line::from(Span::styled(
            "Press q or Esc to quit.",
            Style::default().fg(muted),
        )),
    ];
    let panel = Paragraph::new(text)
        .block(block)
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: true });
    frame.render_widget(panel, area);
}

fn draw_blocked_project_view(
    frame: &mut Frame,
    area: ratatui::layout::Rect,
    title: &'static str,
    accent: Color,
    muted: Color,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(muted))
        .title(Span::styled(title, Style::default().fg(accent)));
    let text = vec![
        Line::from(""),
        Line::from(vec![
            Span::raw("  "),
            Span::styled(
                "Select or create a project in Home.",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::raw("  "),
            Span::styled("Press ", Style::default().fg(muted)),
            Span::styled(
                "1",
                Style::default().fg(accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" to return to Home.", Style::default().fg(muted)),
        ]),
    ];
    frame.render_widget(Paragraph::new(text).block(block), area);
}

fn draw_animation_panel_ui(frame: &mut Frame, app: &App, area: Rect, accent: Color, muted: Color) {
    let text = match &app.glyph_tool_mode {
        GlyphToolMode::None => return,
        GlyphToolMode::ChooseAnimationType { focus } => {
            let standard = if *focus == AnimationTypeChoiceFocus::Standard {
                "[Standard]"
            } else {
                " Standard "
            };
            let grid = if *focus == AnimationTypeChoiceFocus::Grid {
                "[Grid]"
            } else {
                " Grid "
            };
            vec![
                Line::from("Choose animation type"),
                Line::from(""),
                Line::from(format!("  {standard}   {grid}")),
                Line::from(""),
                Line::from("Left/Right to choose, Enter to continue, q/Esc to cancel."),
            ]
        }
        GlyphToolMode::ImportAnimationFrames => {
            let mut lines = vec![Line::from("Importing animation frames"), Line::from("")];
            if let Some(spinner) = app.animation_import_spinner_frame() {
                lines.push(Line::from(format!("{spinner} Loading animation frames...")));
            } else {
                let box_width = 26usize;
                let inner = box_width.saturating_sub(2);
                let top = format!("╭{}╮", dashed_pattern(inner));
                let bottom = format!("╰{}╯", dashed_pattern(inner));
                let label = center_label("DRAG/PASTE MEDIA", inner);
                let border_style = Style::default().fg(accent);
                let label_style = Style::default().fg(accent).add_modifier(Modifier::BOLD);
                lines.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(top, border_style),
                ]));
                lines.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled("│", border_style),
                    Span::styled(label, label_style),
                    Span::styled("│", border_style),
                ]));
                lines.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(bottom, border_style),
                ]));
                lines.push(Line::from("Press Enter when done, q/Esc to cancel."));
            }
            lines.push(Line::from(format!(
                "Current draft frames: {}",
                app.animation_selection_order.len()
            )));
            lines
        }
        GlyphToolMode::SelectAnimationFrames(animation_type) => vec![
            Line::from(format!("Selecting {:?} animation frames", animation_type)),
            Line::from(""),
            Line::from("Space toggles selected imported glyph row as frame."),
            Line::from("Enter to configure, q/Esc to cancel."),
            Line::from(format!(
                "Current draft frames: {}",
                app.animation_selection_order.len()
            )),
        ],
        GlyphToolMode::ConfigureAnimation(config) => {
            let type_label = match config.animation_type {
                AnimationType::Standard => "standard",
                AnimationType::Grid => "grid",
            };
            let focus_label = |target: AnimationConfigFocus, label: String| {
                if config.focus == target {
                    format!("[{label}]")
                } else {
                    label
                }
            };
            let mut lines = vec![
                Line::from(format!("Configure {type_label} animation")),
                Line::from(""),
                Line::from(format!("Name: {}", config.animation_name)),
                Line::from(focus_label(
                    AnimationConfigFocus::Fps,
                    format!("FPS: {}", config.fps),
                )),
            ];
            if config.animation_type == AnimationType::Grid {
                lines.push(Line::from(focus_label(
                    AnimationConfigFocus::Rows,
                    format!("Rows: {}", config.rows),
                )));
                lines.push(Line::from(focus_label(
                    AnimationConfigFocus::Cols,
                    format!("Cols: {}", config.cols),
                )));
                lines.push(Line::from(focus_label(
                    AnimationConfigFocus::HorizontalBleed,
                    format!("L/R bleed: {}", bleed_level_label(config.horizontal_bleed)),
                )));
                lines.push(Line::from(focus_label(
                    AnimationConfigFocus::VerticalBleed,
                    format!("T/B bleed: {}", bleed_level_label(config.vertical_bleed)),
                )));
            }
            lines.push(Line::from(format!(
                "Frames: {}",
                config.selected_frames.len()
            )));
            lines.push(Line::from(focus_label(
                AnimationConfigFocus::Create,
                "Create animation".to_string(),
            )));
            lines.push(Line::from(
                "Left/Right move focus, Up/Down adjust, Enter creates, q/Esc cancels.",
            ));
            lines
        }
    };
    let block = Block::default()
        .title(Span::styled(" Animation ", Style::default().fg(accent)))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(muted));
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(text)
            .block(block)
            .wrap(Wrap { trim: true })
            .style(Style::default().fg(Color::White)),
        area,
    );
}

fn draw_home_import_drop_ui(
    frame: &mut Frame,
    app: &App,
    area: Rect,
    accent: Color,
    muted: Color,
    kind: HomeCreationKind,
) {
    let title = match kind {
        HomeCreationKind::AnimatedGlyph | HomeCreationKind::AnimatedGridGlyph => " Import Step ",
        HomeCreationKind::Glyph | HomeCreationKind::Grid => " Import Step ",
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(muted))
        .title(Span::styled(title, Style::default().fg(accent)));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .margin(1)
        .split(inner);

    let animation_media_mode = matches!(
        kind,
        HomeCreationKind::AnimatedGlyph | HomeCreationKind::AnimatedGridGlyph
    );
    let windows_picker_mode = windows_creation_workflow_uses_picker();
    let imported_count = if animation_media_mode {
        app.animation_selection_order.len()
    } else {
        app.home_workflow_import_count
    };
    let processing_spinner = if animation_media_mode {
        app.animation_import_spinner_frame()
    } else {
        app.home_import_spinner_frame()
    };
    let inline_notice = if matches!(kind, HomeCreationKind::Grid) {
        app.home_workflow_grid_inline_notice.as_deref()
    } else {
        None
    };
    let drag_lines = drag_images_here_lines(
        layout[0].width,
        layout[0].height,
        accent,
        imported_count,
        animation_media_mode,
        windows_picker_mode,
        processing_spinner,
        inline_notice,
    );
    if drag_lines.is_empty() {
        let fallback =
            creation_workflow_import_fallback_label(animation_media_mode, windows_picker_mode);
        frame.render_widget(Paragraph::new(fallback), layout[0]);
    } else {
        frame.render_widget(
            Paragraph::new(drag_lines).wrap(Wrap { trim: false }),
            layout[0],
        );
    }

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" Enter ", Style::default().fg(accent)),
            Span::styled(
                import_step_enter_help(),
                Style::default().fg(muted),
            ),
        ])),
        layout[1],
    );
}

fn home_workflow_preview_lines(
    app: &App,
    kind: HomeCreationKind,
    max_w: u16,
    max_h: u16,
) -> (String, Vec<Line<'static>>) {
    let source_key = match kind {
        HomeCreationKind::Glyph => app
            .current_glyph_tweak_source_key()
            .or_else(|| app.home_workflow_recent_imported_source_keys.last()),
        HomeCreationKind::Grid => app.home_workflow_grid_source_key.as_ref(),
        HomeCreationKind::AnimatedGlyph | HomeCreationKind::AnimatedGridGlyph => {
            if app.animation_selection_order.is_empty() {
                None
            } else {
                let idx = app.animation_import_settings.preview_frame_index
                    % app.animation_selection_order.len();
                app.animation_selection_order.get(idx)
            }
        }
    };
    let Some(source_key) = source_key else {
        return (
            "No source selected".to_string(),
            vec![Line::from("    [Import at least one source first]")],
        );
    };
    if let Some(def) = app.config.compositions.get(source_key) {
        let rows = def.rows;
        let cols = emitted_composition_cols(def.cols);
        let tiles = app
            .glyphs
            .iter()
            .filter(|glyph| {
                glyph.glyph.source_parent_key == *source_key
                    && glyph.glyph.composition_tile.is_some()
            })
            .collect::<Vec<_>>();
        if !tiles.is_empty() {
            return (
                format!("Source: {}", source_display_name(source_key)),
                composition_preview_lines_stable_frame(
                    &tiles,
                    app.animation_import_settings.threshold,
                    tiles
                        .first()
                        .map(|glyph| glyph.working_invert)
                        .unwrap_or(false),
                    rows,
                    cols,
                    max_w,
                    max_h,
                ),
            );
        }
    }

    let source_path = app.config.input_dir.join(source_key);
    if let Some(coverage) = app.live_import_source_coverage(&source_path) {
        let invert = app
            .config
            .invert_overrides
            .get(source_key)
            .copied()
            .unwrap_or(false);
        let preview_lines = if matches!(
            kind,
            HomeCreationKind::AnimatedGlyph | HomeCreationKind::AnimatedGridGlyph
        ) {
            preview_lines_from_coverage_full_frame(
                &coverage,
                app.config.glyph_size,
                app.config.glyph_size,
                app.animation_import_settings.threshold,
                invert,
                max_w,
                max_h,
            )
        } else {
            preview_lines_from_coverage_stable_frame(
                &coverage,
                app.config.glyph_size,
                app.config.glyph_size,
                app.animation_import_settings.threshold,
                invert,
                max_w,
                max_h,
            )
        };
        return (
            format!("Source: {}", source_display_name(source_key)),
            preview_lines,
        );
    }

    let glyph = app
        .glyphs
        .iter()
        .find(|glyph| {
            glyph.glyph.source_parent_key == *source_key && glyph.glyph.composition_tile.is_none()
        })
        .or_else(|| {
            app.glyphs
                .iter()
                .find(|glyph| glyph.glyph.source_parent_key == *source_key)
        });
    let Some(glyph) = glyph else {
        return (
            format!("Source: {}", source_display_name(source_key)),
            vec![Line::from("    [Preview not available yet]")],
        );
    };
    (
        format!("Source: {}", source_display_name(source_key)),
        preview_lines_stable_frame(
            &glyph.glyph,
            app.animation_import_settings.threshold,
            glyph.working_invert,
            max_w,
            max_h,
        ),
    )
}

fn glyph_tweak_progress_label(app: &App) -> String {
    let total = app.home_workflow_tweak_source_queue.len().max(1);
    let current = app
        .home_workflow_tweak_source_index
        .saturating_add(1)
        .min(total);
    format!("Image {current}/{total}  ")
}

fn glyph_tweak_continue_finishes_workflow(app: &App) -> bool {
    app.home_workflow_tweak_source_index.saturating_add(1)
        >= app.home_workflow_tweak_source_queue.len()
}

fn glyph_tweak_continue_label(app: &App) -> &'static str {
    if glyph_tweak_continue_finishes_workflow(app) {
        "Finish"
    } else {
        "Next"
    }
}

fn draw_animation_import_workflow_ui(
    frame: &mut Frame,
    app: &App,
    area: Rect,
    accent: Color,
    muted: Color,
    kind: HomeCreationKind,
) {
    let title = " Tweaking Step ";
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(muted))
        .title(Span::styled(title, Style::default().fg(accent)));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let content_area = Rect {
        x: inner.x.saturating_add(1),
        y: inner.y.saturating_add(1),
        width: inner.width.saturating_sub(2),
        height: inner.height.saturating_sub(2),
    };
    let fixed_controls_height = 3u16 + 3 + 1;
    let max_preview_area_height = content_area
        .height
        .saturating_sub(fixed_controls_height)
        .max(3);
    let preview_inner_width = content_area.width.saturating_sub(2);
    let preview_max_h = max_preview_area_height
        .saturating_sub(3) // borders plus the source/threshold header line.
        .max(1);
    let (preview_title, preview_content) = home_workflow_preview_lines(
        app,
        kind,
        preview_inner_width.saturating_sub(4) / 2,
        preview_max_h,
    );
    let threshold_marker = if app.animation_import_settings.threshold == app.config.base_threshold {
        "default"
    } else {
        "custom*"
    };
    let mut preview_lines = vec![Line::from(vec![
        Span::styled("  ", Style::default()),
        Span::styled(preview_title, Style::default().fg(accent)),
        Span::raw("  "),
        Span::styled(
            if matches!(kind, HomeCreationKind::Glyph) {
                glyph_tweak_progress_label(app)
            } else {
                String::new()
            },
            Style::default().fg(muted),
        ),
        Span::styled(
            format!(
                "Threshold: {} ({threshold_marker})",
                app.animation_import_settings.threshold
            ),
            Style::default().fg(Color::White),
        ),
    ])];
    preview_lines.extend(preview_content);
    let preview_area_height = (preview_lines.len() as u16)
        .saturating_add(2)
        .clamp(3, max_preview_area_height);
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(preview_area_height),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .split(content_area);

    let preview_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(muted))
        .title(Span::styled(" Preview ", Style::default().fg(accent)));
    let preview_inner = preview_block.inner(layout[0]);
    frame.render_widget(preview_block, layout[0]);
    frame.render_widget(
        Paragraph::new(preview_lines).wrap(Wrap { trim: false }),
        preview_inner,
    );

    let focused_style = home_panel_button_style(true, accent);
    let idle_style = home_panel_button_style(false, accent);
    let continue_style = if app.animation_import_settings.focus
        == AnimationImportSettingsFocus::Continue
        && app.animation_import_settings.grayscale_editor.is_none()
    {
        focused_style
    } else {
        idle_style
    };
    let button_width = 18;
    let visible_focuses = import_settings_visible_focuses(kind);
    let mut row_constraints = Vec::new();
    for index in 0..visible_focuses.len() {
        if index > 0 {
            row_constraints.push(Constraint::Length(2));
        }
        row_constraints.push(Constraint::Length(button_width));
    }
    row_constraints.push(Constraint::Min(0));
    let row = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(row_constraints)
        .split(layout[1]);

    for (index, focus) in visible_focuses.iter().enumerate() {
        let area = row[index * 2];
        let selected = app.animation_import_settings.focus == *focus
            && app.animation_import_settings.grayscale_editor.is_none();
        let mut style = if selected { focused_style } else { idle_style };
        let label = match focus {
            AnimationImportSettingsFocus::GrayscaleToggle => {
                if app.animation_import_settings.grayscale_enabled {
                    padded_button_label("Gray: ON")
                } else {
                    padded_button_label("Gray: OFF")
                }
            }
            AnimationImportSettingsFocus::GrayscaleOptionsButton => {
                style = if selected || app.animation_import_settings.grayscale_editor.is_some() {
                    focused_style
                } else {
                    idle_style
                };
                let dirty = if grayscale_options_are_default(
                    app.animation_import_settings.grayscale_options,
                ) {
                    ""
                } else {
                    " *"
                };
                padded_button_label(format!("Gray Options{dirty}"))
            }
            AnimationImportSettingsFocus::Threshold => {
                let dirty = if app.animation_import_settings.threshold == app.config.base_threshold
                {
                    ""
                } else {
                    " *"
                };
                padded_button_label(format!(
                    "Th: {}{dirty}",
                    app.animation_import_settings.threshold
                ))
            }
            AnimationImportSettingsFocus::FramesButton => {
                if !app.animation_selection_order.is_empty() {
                    let total = app.animation_selection_order.len();
                    let idx = app.animation_import_settings.preview_frame_index % total;
                    padded_button_label(format!("Frames: {}/{}", idx + 1, total))
                } else {
                    padded_button_label("Frames: -")
                }
            }
            AnimationImportSettingsFocus::ExportTestImageButton => padded_button_label("Export Test"),
            AnimationImportSettingsFocus::Continue => {
                style = if selected {
                    focused_style
                } else {
                    continue_style
                };
                if matches!(kind, HomeCreationKind::Glyph) {
                    padded_button_label(glyph_tweak_continue_label(app))
                } else {
                    padded_button_label("Continue")
                }
            }
            AnimationImportSettingsFocus::Back => padded_button_label("Back"),
            AnimationImportSettingsFocus::SkipAll => padded_button_label("Skip All"),
        };
        render_home_panel_button(frame, area, Line::from(vec![Span::styled(label, style)]));
    }

    let options_line = if let Some(editor) = &app.animation_import_settings.grayscale_editor {
        let knobs = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(20),
                Constraint::Length(2),
                Constraint::Length(20),
                Constraint::Length(2),
                Constraint::Length(20),
                Constraint::Min(0),
            ])
            .split(layout[2]);
        let knob_style = |target: GrayscaleKnobFocus| {
            if editor.focus == target {
                focused_style
            } else {
                idle_style
            }
        };
        render_home_panel_button(
            frame,
            knobs[0],
            Line::from(vec![Span::styled(
                format!(" Brightness: {:+} ", editor.draft.brightness),
                knob_style(GrayscaleKnobFocus::Brightness),
            )]),
        );
        render_home_panel_button(
            frame,
            knobs[2],
            Line::from(vec![Span::styled(
                format!(" Contrast: {:+} ", editor.draft.contrast),
                knob_style(GrayscaleKnobFocus::Contrast),
            )]),
        );
        render_home_panel_button(
            frame,
            knobs[4],
            Line::from(vec![Span::styled(
                format!(" Gamma: {:.2} ", editor.draft.gamma_percent as f32 / 100.0),
                knob_style(GrayscaleKnobFocus::Gamma),
            )]),
        );
        Line::from(vec![
            Span::styled(" Captive edit: ", Style::default().fg(accent)),
            Span::styled(
                "Left/Right choose knob, Up/Down adjust, Enter apply, q/Esc cancel",
                Style::default().fg(muted),
            ),
        ])
    } else {
        if let Some(path) = &app.animation_import_settings.last_exported_test_image {
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(" Last export: ", Style::default().fg(accent)),
                    Span::styled(
                        path.display().to_string(),
                        Style::default().fg(Color::White),
                    ),
                ]))
                .wrap(Wrap { trim: true }),
                layout[2],
            );
        }
        let summary = app.animation_import_settings.grayscale_options;
        let marker = if grayscale_options_are_default(summary) {
            "default"
        } else {
            "modified*"
        };
        Line::from(vec![
            Span::styled(" Grayscale profile: ", Style::default().fg(accent)),
            Span::styled(
                format!(
                    "B {:+}, C {:+}, G {:.2} ({marker})",
                    summary.brightness,
                    summary.contrast,
                    summary.gamma_percent as f32 / 100.0
                ),
                Style::default().fg(Color::White),
            ),
            Span::raw("  "),
            if matches!(
                kind,
                HomeCreationKind::AnimatedGlyph | HomeCreationKind::AnimatedGridGlyph
            ) {
                Span::styled(
                    format!(
                        "Export count: {} (focus Export + Up/Down)  ",
                        app.animation_import_settings.export_frame_count
                    ),
                    Style::default().fg(accent),
                )
            } else {
                Span::raw("")
            },
            Span::styled(
                "Left/Right focus, Up/Down changes toggle/threshold/frame/export count, Enter toggle/open/export/continue.",
                Style::default().fg(muted),
            ),
        ])
    };
    frame.render_widget(
        Paragraph::new(options_line).wrap(Wrap { trim: true }),
        layout[3],
    );
}

fn draw_home_creation_area(frame: &mut Frame, app: &App, area: Rect, accent: Color, muted: Color) {
    match app.home_workflow {
        HomeWorkflow::Launcher => {
            let focus = app.home_launcher_focus;
            let button = |label: &str, selected: bool, focused: bool| -> Span<'static> {
                let style = if selected && focused {
                    Style::default()
                        .fg(Color::Black)
                        .bg(accent)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                        .fg(Color::White)
                        .bg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD)
                };
                Span::styled(format!(" {label} "), style)
            };
            let create_buttons_focused = app.welcome_focus == WelcomeFocus::HomeCreateButtons;
            frame.render_widget(
                Paragraph::new(vec![
                    Line::from(""),
                    Line::from(vec![
                        button(
                            "Create glyph",
                            focus == HomeLauncherFocus::CreateGlyph,
                            create_buttons_focused,
                        ),
                        Span::raw("  "),
                        button(
                            "Create grid",
                            focus == HomeLauncherFocus::CreateGrid,
                            create_buttons_focused,
                        ),
                    ]),
                    Line::from(""),
                    Line::from(vec![
                        button(
                            "Create animated glyph",
                            focus == HomeLauncherFocus::CreateAnimatedGlyph,
                            create_buttons_focused,
                        ),
                        Span::raw("  "),
                        button(
                            "Create animated grid glyph",
                            focus == HomeLauncherFocus::CreateAnimatedGridGlyph,
                            create_buttons_focused,
                        ),
                    ]),
                ])
                .alignment(Alignment::Center)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_type(BorderType::Rounded)
                        .border_style(Style::default().fg(muted))
                        .title(Span::styled(
                            " Creation Workflows ",
                            Style::default().fg(accent).add_modifier(Modifier::BOLD),
                        )),
                ),
                area,
            );
        }
        _ => {}
    }
}
