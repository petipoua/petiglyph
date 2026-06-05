fn draw_glyphs_view(
    frame: &mut Frame,
    app: &App,
    area: ratatui::layout::Rect,
    accent: Color,
    muted: Color,
) {
    let area = Rect::new(
        area.x,
        area.y,
        area.width,
        area.height.min(GLYPHS_PANEL_MAX_HEIGHT),
    );
    if app.active_project.is_none() {
        draw_blocked_project_view(frame, area, " Glyphs ", accent, muted);
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
        .split(area);

    let left_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(chunks[0]);

    let is_install_focused = app.glyphs_focus == GlyphsFocus::InstallButton;
    let install_label = if app.current_project_is_installed() {
        " Reinstall "
    } else {
        " Install "
    };
    let install_style = if is_install_focused {
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
    frame.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(install_label, install_style)]))
            .alignment(Alignment::Center),
        left_chunks[0],
    );

    let mut list_title = vec![Span::styled(" Glyphs ", Style::default().fg(accent))];
    if app.selecting_for_grid {
        list_title.push(Span::styled(
            " select a glyph for the grid ",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
    } else if app.selecting_for_animation_frames {
        list_title.push(Span::styled(
            " select frames (Space toggle) ",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
    }

    let list_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(muted))
        .title(Line::from(list_title));

    let visible_rows = app.visible_glyph_rows();
    let mut list_state = ListState::default();
    if !visible_rows.is_empty() {
        list_state.select(Some(app.selected_visible.min(visible_rows.len() - 1)));
    }

    let list_highlight_style = if app.glyphs_focus == GlyphsFocus::List {
        Style::default()
            .fg(Color::Black)
            .bg(if app.selecting_for_grid {
                Color::Yellow
            } else if app.selecting_for_animation_frames {
                Color::Yellow
            } else {
                accent
            })
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White).bg(Color::DarkGray)
    };

    let items: Vec<ListItem> = if visible_rows.is_empty() {
        vec![ListItem::new(" No glyphs found. ")]
    } else {
        visible_rows
            .iter()
            .map(|row| match row {
                VisibleGlyphRow::AnimationParent { animation_idx } => {
                    let animation = &app.config.animations[*animation_idx];
                    let expanded = app.expanded_animations.contains(&animation.name);
                    let arrow = if expanded { "[-]" } else { "[+]" };
                    ListItem::new(Line::from(vec![
                        Span::styled(arrow, Style::default().fg(accent)),
                        Span::raw(" "),
                        Span::styled(" @", Style::default().fg(Color::Magenta)),
                        Span::raw(" "),
                        Span::styled(
                            animation.name.clone(),
                            Style::default()
                                .fg(Color::White)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::raw(" "),
                        Span::styled(
                            format!(
                                "({:?}, {} fps, {} frames, {}, th {})",
                                animation.animation_type,
                                animation.fps,
                                animation.frames.len(),
                                animation_grayscale_summary_label(animation),
                                animation_threshold_summary_label(app, animation),
                            ),
                            Style::default().fg(muted),
                        ),
                    ]))
                }
                VisibleGlyphRow::AnimationFrame {
                    frame_idx,
                    source_key,
                    glyph_idx,
                    ..
                } => {
                    let marker = if app.animation_selection_set.contains(source_key) {
                        " +"
                    } else {
                        "  "
                    };
                    let codepoint = glyph_idx
                        .map(|idx| glyph_codepoint_label(app, &idx))
                        .unwrap_or_else(|| "missing".to_string());
                    ListItem::new(Line::from(vec![
                        Span::styled(marker, Style::default().fg(Color::Yellow)),
                        Span::raw("    "),
                        Span::styled(format!("f{} ", frame_idx + 1), Style::default().fg(muted)),
                        Span::styled(format!("{} ", codepoint), Style::default().fg(muted)),
                        Span::raw(source_display_name(source_key)),
                    ]))
                }
                VisibleGlyphRow::Single { glyph_idx } => {
                    let glyph = &app.glyphs[*glyph_idx];
                    let is_selected_for_animation = app
                        .animation_selection_set
                        .contains(&glyph.glyph.source_parent_key);
                    let marker = if glyph.saved_threshold.is_some() {
                        " *"
                    } else if is_selected_for_animation {
                        " +"
                    } else {
                        "  "
                    };
                    ListItem::new(Line::from(vec![
                        Span::styled(marker, Style::default().fg(Color::Yellow)),
                        Span::styled(
                            format!(" {} ", glyph_codepoint_label(app, glyph_idx)),
                            Style::default().fg(muted),
                        ),
                        Span::raw(glyph.glyph.glyph_name.clone()),
                    ]))
                }
                VisibleGlyphRow::CompositionParent {
                    source_key,
                    rows,
                    cols,
                    ..
                } => {
                    let marker = if app.animation_selection_set.contains(source_key) {
                        "+"
                    } else {
                        " "
                    };
                    let expanded = app.expanded_compositions.contains(source_key);
                    let arrow = if expanded { "[-]" } else { "[+]" };
                    let label = source_display_name(source_key);
                    ListItem::new(Line::from(vec![
                        Span::styled(format!(" {marker}"), Style::default().fg(Color::Yellow)),
                        Span::styled(arrow, Style::default().fg(accent)),
                        Span::raw(" "),
                        Span::styled(label, Style::default().fg(Color::White)),
                        Span::raw(" "),
                        Span::styled(
                            format!("(grid {}x{})", rows, cols),
                            Style::default().fg(Color::Yellow),
                        ),
                    ]))
                }
                VisibleGlyphRow::CompositionChild {
                    glyph_idx,
                    row,
                    col,
                    ..
                } => {
                    let glyph = &app.glyphs[*glyph_idx];
                    let marker = if glyph.saved_threshold.is_some() {
                        " *"
                    } else {
                        "  "
                    };
                    ListItem::new(Line::from(vec![
                        Span::styled(marker, Style::default().fg(Color::Yellow)),
                        Span::raw("    "),
                        Span::styled(
                            format!("{} ", glyph_codepoint_label(app, glyph_idx)),
                            Style::default().fg(muted),
                        ),
                        Span::raw(format!(
                            "[r{},c{}] {}",
                            row + 1,
                            col + 1,
                            glyph.glyph.glyph_name
                        )),
                    ]))
                }
            })
            .collect()
    };

    let list = List::new(items)
        .block(list_block)
        .highlight_style(list_highlight_style)
        .highlight_symbol(" \u{2023} ");

    frame.render_stateful_widget(list, left_chunks[1], &mut list_state);

    if let Some(config) = &app.grid_config {
        draw_grid_config_ui(frame, app, config, chunks[1], accent, muted);
        return;
    }
    if !matches!(app.glyph_tool_mode, GlyphToolMode::None) {
        draw_animation_panel_ui(frame, app, chunks[1], accent, muted);
        return;
    }

    let preview_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(muted))
        .title(Span::styled(" Preview ", Style::default().fg(accent)));

    let preview_area = preview_block.inner(chunks[1]);

    let active_animation = app.selected_animation_for_preview();
    let selected_row = visible_rows.get(
        app.selected_visible
            .min(visible_rows.len().saturating_sub(1)),
    );
    let active_animation_frame = active_animation.and_then(|animation| {
        animation_frame_source_for_preview(selected_row, animation, app.animation_preview.as_ref())
    });

    let mut preview_content = if visible_rows.is_empty() {
        vec![
            Line::from(""),
            Line::from("  Add or drag images into this project."),
        ]
    } else {
        match &visible_rows[app.selected_visible.min(visible_rows.len() - 1)] {
            VisibleGlyphRow::AnimationParent { animation_idx } => {
                let animation = &app.config.animations[*animation_idx];
                let has_non_uniform_thresholds =
                    animation_has_non_uniform_frame_thresholds(app, animation);
                let has_non_uniform_invert = animation_has_non_uniform_frame_invert(app, animation);
                let threshold_summary = animation_threshold_summary_label(app, animation);
                let grayscale_summary = animation_grayscale_summary_label(animation);
                vec![
                    Line::from(""),
                    Line::from(vec![
                        Span::raw("  Animation: "),
                        Span::styled(
                            animation.name.clone(),
                            Style::default()
                                .fg(Color::White)
                                .add_modifier(Modifier::BOLD),
                        ),
                    ]),
                    Line::from(vec![
                        Span::raw("  Type: "),
                        Span::styled(
                            format!("{:?}", animation.animation_type),
                            Style::default().fg(muted),
                        ),
                        Span::raw("  FPS: "),
                        Span::styled(animation.fps.to_string(), Style::default().fg(accent)),
                        Span::raw("  Frames: "),
                        Span::styled(
                            animation.frames.len().to_string(),
                            Style::default().fg(accent),
                        ),
                        Span::raw("  "),
                        Span::styled(grayscale_summary, Style::default().fg(muted)),
                        Span::raw("  Threshold: "),
                        Span::styled(threshold_summary, Style::default().fg(accent)),
                    ]),
                    if has_non_uniform_thresholds {
                        Line::from(vec![
                            Span::raw("  Thresholds: "),
                            Span::styled(
                                "frame-specific overrides active (var. threshold)",
                                Style::default().fg(Color::Yellow),
                            ),
                        ])
                    } else {
                        Line::from(vec![
                            Span::raw("  Thresholds: "),
                            Span::styled("uniform across frames", Style::default().fg(muted)),
                        ])
                    },
                    if has_non_uniform_invert {
                        Line::from(vec![
                            Span::raw("  Invert: "),
                            Span::styled(
                                "frame-specific overrides active",
                                Style::default().fg(Color::Yellow),
                            ),
                        ])
                    } else {
                        Line::from(vec![
                            Span::raw("  Invert: "),
                            Span::styled("uniform across frames", Style::default().fg(muted)),
                        ])
                    },
                    Line::from(""),
                ]
            }
            VisibleGlyphRow::AnimationFrame {
                source_key,
                glyph_idx,
                ..
            } => {
                let file_label = glyph_idx
                    .and_then(|idx| app.glyphs.get(idx))
                    .map(|active| {
                        if app.verbose_paths {
                            active.glyph.source_path.to_string_lossy().to_string()
                        } else {
                            source_display_name(source_key)
                        }
                    })
                    .unwrap_or_else(|| source_display_name(source_key));
                vec![
                    Line::from(""),
                    Line::from(vec![
                        Span::raw("  Frame: "),
                        Span::styled(file_label, Style::default().fg(Color::White)),
                    ]),
                    Line::from(vec![
                        Span::raw("  Animation: "),
                        Span::styled(
                            active_animation
                                .map(|a| {
                                    format!(
                                        "{} ({:?}, {} fps, {} frames, {}, th {})",
                                        a.name,
                                        a.animation_type,
                                        a.fps,
                                        a.frames.len(),
                                        animation_grayscale_summary_label(a),
                                        animation_threshold_summary_label(app, a)
                                    )
                                })
                                .unwrap_or_else(|| "none".to_string()),
                            Style::default().fg(muted),
                        ),
                    ]),
                    Line::from(""),
                ]
            }
            VisibleGlyphRow::Single { glyph_idx }
            | VisibleGlyphRow::CompositionChild { glyph_idx, .. } => {
                let active = &app.glyphs[*glyph_idx];
                vec![
                    Line::from(""),
                    Line::from(vec![
                        Span::raw("  File: "),
                        Span::styled(
                            if app.verbose_paths {
                                active.glyph.source_path.to_string_lossy().to_string()
                            } else {
                                active
                                    .glyph
                                    .source_path
                                    .file_name()
                                    .and_then(|name| name.to_str())
                                    .map(ToOwned::to_owned)
                                    .unwrap_or_else(|| active.glyph.glyph_name.clone())
                            },
                            Style::default().fg(Color::White),
                        ),
                    ]),
                    Line::from(vec![
                        Span::raw("  Threshold: "),
                        Span::styled(
                            format!("{:3}", active.working_threshold),
                            Style::default().fg(accent).add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            if active.saved_threshold.is_some() {
                                " (overridden)"
                            } else {
                                " (default)"
                            },
                            Style::default().fg(muted),
                        ),
                    ]),
                    Line::from(vec![
                        Span::raw("  Invert: "),
                        Span::styled(
                            if active.working_invert { "on" } else { "off" },
                            Style::default().fg(accent).add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            if active.saved_invert {
                                " (overridden)"
                            } else {
                                " (default)"
                            },
                            Style::default().fg(muted),
                        ),
                    ]),
                    Line::from(vec![
                        Span::raw("  Animation: "),
                        Span::styled(
                            active_animation
                                .map(|a| {
                                    format!(
                                        "{} ({:?}, {} fps, {} frames, {}, th {})",
                                        a.name,
                                        a.animation_type,
                                        a.fps,
                                        a.frames.len(),
                                        animation_grayscale_summary_label(a),
                                        animation_threshold_summary_label(app, a)
                                    )
                                })
                                .unwrap_or_else(|| "none".to_string()),
                            Style::default().fg(muted),
                        ),
                    ]),
                    Line::from(""),
                ]
            }
            VisibleGlyphRow::CompositionParent {
                source_key,
                rows,
                cols,
                ..
            } => {
                let bleed_hint = app
                    .config
                    .compositions
                    .get(source_key)
                    .map(|def| (def.horizontal_bleed, def.vertical_bleed));
                let threshold_hint = app
                    .glyphs
                    .iter()
                    .find(|g| g.glyph.source_parent_key == *source_key)
                    .map(|g| (g.working_threshold, g.saved_threshold.is_some()));
                let invert_hint = app
                    .glyphs
                    .iter()
                    .find(|g| g.glyph.source_parent_key == *source_key)
                    .map(|g| g.working_invert);
                let threshold_line = if let Some((threshold, overridden)) = threshold_hint {
                    Line::from(vec![
                        Span::raw("  Threshold: "),
                        Span::styled(
                            format!("{:3}", threshold),
                            Style::default().fg(accent).add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            if overridden {
                                " (overridden)"
                            } else {
                                " (default)"
                            },
                            Style::default().fg(muted),
                        ),
                    ])
                } else {
                    Line::from(vec![
                        Span::raw("  Threshold: "),
                        Span::styled("n/a", Style::default().fg(muted)),
                    ])
                };
                let invert_line = Line::from(vec![
                    Span::raw("  Invert: "),
                    Span::styled(
                        if invert_hint.unwrap_or(false) {
                            "on"
                        } else {
                            "off"
                        },
                        Style::default().fg(accent).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        if invert_hint.unwrap_or(false) {
                            " (overridden)"
                        } else {
                            " (default)"
                        },
                        Style::default().fg(muted),
                    ),
                ]);
                vec![
                    Line::from(""),
                    Line::from(vec![
                        Span::raw("  Composition: "),
                        Span::styled(
                            source_display_name(source_key),
                            Style::default().fg(Color::White),
                        ),
                        Span::raw(" "),
                        Span::styled(
                            format!("({rows}x{cols})"),
                            Style::default().fg(Color::Yellow),
                        ),
                        Span::raw(" "),
                        Span::styled("[L/R:", Style::default().fg(muted)),
                        Span::styled(
                            match bleed_hint {
                                Some((level, _)) => bleed_level_label(level),
                                None => "n/a",
                            },
                            Style::default()
                                .fg(match bleed_hint {
                                    Some((BleedLevel::Off, _)) => Color::LightRed,
                                    Some((BleedLevel::Weak, _)) => Color::Green,
                                    Some((BleedLevel::Strong, _)) => Color::Yellow,
                                    None => muted,
                                })
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(" T/B:", Style::default().fg(muted)),
                        Span::styled(
                            match bleed_hint {
                                Some((_, level)) => bleed_level_label(level),
                                None => "n/a",
                            },
                            Style::default()
                                .fg(match bleed_hint {
                                    Some((_, BleedLevel::Off)) => Color::LightRed,
                                    Some((_, BleedLevel::Weak)) => Color::Green,
                                    Some((_, BleedLevel::Strong)) => Color::Yellow,
                                    None => muted,
                                })
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled("]", Style::default().fg(muted)),
                    ]),
                    threshold_line,
                    invert_line,
                    Line::from(vec![
                        Span::raw("  Preview: "),
                        Span::styled("Assembled composition", Style::default().fg(muted)),
                    ]),
                    Line::from(vec![
                        Span::raw("  Animation: "),
                        Span::styled(
                            active_animation
                                .map(|a| {
                                    format!(
                                        "{} ({:?}, {} fps, {} frames, {}, th {})",
                                        a.name,
                                        a.animation_type,
                                        a.fps,
                                        a.frames.len(),
                                        animation_grayscale_summary_label(a),
                                        animation_threshold_summary_label(app, a)
                                    )
                                })
                                .unwrap_or_else(|| "none".to_string()),
                            Style::default().fg(muted),
                        ),
                    ]),
                    Line::from(""),
                ]
            }
        }
    };

    if !visible_rows.is_empty() {
        match &visible_rows[app.selected_visible.min(visible_rows.len() - 1)] {
            _ => {
                if let (Some(animation), Some(frame_source_key)) =
                    (active_animation, active_animation_frame.as_ref())
                {
                    let threshold = app
                        .glyphs
                        .iter()
                        .find(|g| glyph_matches_animation_row_frame(g, animation, frame_source_key))
                        .map(|g| g.working_threshold)
                        .unwrap_or(app.config.base_threshold);
                    let invert = app
                        .glyphs
                        .iter()
                        .find(|g| glyph_matches_animation_row_frame(g, animation, frame_source_key))
                        .map(|g| g.working_invert)
                        .unwrap_or(false);
                    let mut ascii = if animation.animation_type == AnimationType::Grid {
                        let rows = animation.rows.unwrap_or(2);
                        let cols = emitted_composition_cols(animation.cols.unwrap_or(2));
                        let tiles = app
                            .glyphs
                            .iter()
                            .filter(|g| {
                                g.glyph.source_parent_key == *frame_source_key
                                    && g.glyph.composition_tile.is_some()
                            })
                            .collect::<Vec<_>>();
                        composition_preview_lines_stable_frame(
                            &tiles,
                            threshold,
                            invert,
                            rows,
                            cols,
                            preview_area.width.saturating_sub(4) / 2,
                            preview_area.height.saturating_sub(8),
                        )
                    } else {
                        app.glyphs
                            .iter()
                            .find(|g| {
                                glyph_matches_animation_row_frame(g, animation, frame_source_key)
                            })
                            .map(|g| {
                                preview_lines_stable_frame(
                                    &g.glyph,
                                    threshold,
                                    invert,
                                    preview_area.width.saturating_sub(4) / 2,
                                    preview_area.height.saturating_sub(8),
                                )
                            })
                            .unwrap_or_else(|| vec![Line::from("    [Animation frame missing]")])
                    };
                    preview_content.push(Line::from(vec![
                        Span::raw("  Frame: "),
                        Span::styled(
                            source_display_name(frame_source_key),
                            Style::default().fg(Color::Yellow),
                        ),
                    ]));
                    preview_content.push(Line::from(""));
                    preview_content.append(&mut ascii);
                } else {
                    match &visible_rows[app.selected_visible.min(visible_rows.len() - 1)] {
                        VisibleGlyphRow::AnimationParent { .. } => {}
                        VisibleGlyphRow::AnimationFrame { glyph_idx, .. } => {
                            if let Some(glyph_idx) = glyph_idx {
                                let active = &app.glyphs[*glyph_idx];
                                let mut ascii = preview_lines(
                                    &active.glyph,
                                    active.working_threshold,
                                    active.working_invert,
                                    preview_area.width.saturating_sub(4) / 2,
                                    preview_area.height.saturating_sub(6),
                                );
                                preview_content.append(&mut ascii);
                            }
                        }
                        VisibleGlyphRow::Single { glyph_idx }
                        | VisibleGlyphRow::CompositionChild { glyph_idx, .. } => {
                            let active = &app.glyphs[*glyph_idx];
                            let mut ascii = preview_lines(
                                &active.glyph,
                                active.working_threshold,
                                active.working_invert,
                                preview_area.width.saturating_sub(4) / 2,
                                preview_area.height.saturating_sub(6),
                            );
                            preview_content.append(&mut ascii);
                        }
                        VisibleGlyphRow::CompositionParent {
                            source_key,
                            rows,
                            cols,
                            ..
                        } => {
                            let tiles = app
                                .glyphs
                                .iter()
                                .filter(|g| g.glyph.source_parent_key == *source_key)
                                .collect::<Vec<_>>();
                            let threshold = tiles
                                .first()
                                .map(|g| g.working_threshold)
                                .unwrap_or(app.config.base_threshold);
                            let invert = tiles.first().map(|g| g.working_invert).unwrap_or(false);
                            let mut ascii = composition_preview_lines(
                                &tiles,
                                threshold,
                                invert,
                                *rows,
                                *cols,
                                preview_area.width.saturating_sub(4) / 2,
                                preview_area.height.saturating_sub(6),
                            );
                            preview_content.append(&mut ascii);
                        }
                    }
                }
            }
        }
    }

    let supports_threshold = selected_row_supports_threshold(app);
    let supports_fps = selected_row_supports_fps(app);
    let supports_invert = selected_row_supports_invert(app);
    if supports_threshold || supports_fps || supports_invert {
        let button_style = |selected: bool| {
            if selected {
                Style::default()
                    .fg(Color::Black)
                    .bg(accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
                    .fg(Color::White)
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD)
            }
        };

        let threshold_value = selected_row_threshold_value(app)
            .map(|value| value.to_string())
            .unwrap_or_else(|| "-".to_string());
        let fps_value = selected_row_fps_value(app)
            .map(|value| value.to_string())
            .unwrap_or_else(|| "-".to_string());
        let invert_value = selected_row_invert_value(app)
            .map(|value| if value { "on" } else { "off" })
            .unwrap_or("-");

        let threshold_selected = supports_threshold
            && app.glyphs_focus == GlyphsFocus::Preview
            && app.glyph_preview_control == GlyphPreviewControl::Threshold;
        let fps_selected = supports_fps
            && app.glyphs_focus == GlyphsFocus::Preview
            && app.glyph_preview_control == GlyphPreviewControl::Fps;
        let invert_selected = supports_invert
            && app.glyphs_focus == GlyphsFocus::Preview
            && app.glyph_preview_control == GlyphPreviewControl::Invert;

        let mut edit_spans = vec![
            Span::raw("  Edit "),
            Span::styled(
                format!(" Threshold: {threshold_value} "),
                button_style(threshold_selected),
            ),
        ];
        if supports_fps {
            edit_spans.push(Span::raw(" "));
            edit_spans.push(Span::styled(
                format!(" FPS: {fps_value} "),
                button_style(fps_selected),
            ));
        }
        if supports_invert {
            edit_spans.push(Span::raw(" "));
            edit_spans.push(Span::styled(
                format!(" Invert: {invert_value} "),
                button_style(invert_selected),
            ));
        }

        preview_content.insert(0, Line::from(edit_spans));
    }

    let p = Paragraph::new(preview_content)
        .block(preview_block)
        .wrap(Wrap { trim: false });
    frame.render_widget(p, chunks[1]);
}

fn glyph_codepoint_label(app: &App, glyph_idx: &usize) -> String {
    format_codepoint(app.config.codepoint_start.saturating_add(*glyph_idx as u32))
}

fn source_display_name(source_key: &str) -> String {
    Path::new(source_key)
        .file_name()
        .and_then(|name| name.to_str())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| source_key.to_string())
}

fn composition_preview_lines(
    tiles: &[&InteractiveGlyph],
    threshold: u8,
    invert: bool,
    rows: usize,
    cols: usize,
    max_w: u16,
    max_h: u16,
) -> Vec<Line<'static>> {
    if tiles.is_empty() || rows == 0 || cols == 0 || max_w == 0 || max_h == 0 {
        return vec![Line::from("    [Composition preview unavailable]")];
    }

    let Some((tile_width, tile_height)) = tiles
        .first()
        .map(|g| (g.glyph.width as usize, g.glyph.height as usize))
    else {
        return vec![Line::from("    [Composition preview unavailable]")];
    };
    if tile_width == 0 || tile_height == 0 {
        return vec![Line::from("    [Composition preview unavailable]")];
    }

    let width = cols.saturating_mul(tile_width);
    let height = rows.saturating_mul(tile_height);
    if width == 0 || height == 0 {
        return vec![Line::from("    [Composition preview unavailable]")];
    }

    let mut matrix = vec![false; width.saturating_mul(height)];
    for tile in tiles {
        let Some(info) = &tile.glyph.composition_tile else {
            continue;
        };
        if info.rows != rows || info.cols != cols {
            continue;
        }
        if tile.glyph.width as usize != tile_width || tile.glyph.height as usize != tile_height {
            continue;
        }
        for y in 0..tile_height {
            for x in 0..tile_width {
                let src_idx = y * tile_width + x;
                let dst_x = info.col * tile_width + x;
                let dst_y = info.row * tile_height + y;
                if dst_x >= width || dst_y >= height || src_idx >= tile.glyph.coverage.len() {
                    continue;
                }
                let dst_idx = dst_y * width + dst_x;
                matrix[dst_idx] = (tile.glyph.coverage[src_idx] >= threshold) ^ invert;
            }
        }
    }

    if let Some((cropped, cropped_w, cropped_h)) =
        crop_binary_matrix_to_active_bounds(&matrix, width, height)
    {
        render_binary_preview_lines(
            &cropped, cropped_w, cropped_h, max_w, max_h, false, true, false,
        )
    } else {
        vec![Line::from("    [No visible pixels at threshold]")]
    }
}

fn composition_preview_lines_stable_frame(
    tiles: &[&InteractiveGlyph],
    threshold: u8,
    invert: bool,
    rows: usize,
    cols: usize,
    max_w: u16,
    max_h: u16,
) -> Vec<Line<'static>> {
    if rows == 0 || cols == 0 {
        return vec![Line::from("    [Composition preview unavailable]")];
    }
    let Some(first) = tiles.first() else {
        return vec![Line::from("    [Composition preview unavailable]")];
    };
    let tile_width = first.glyph.width as usize;
    let tile_height = first.glyph.height as usize;
    if tile_width == 0 || tile_height == 0 {
        return vec![Line::from("    [Composition preview unavailable]")];
    }

    let width = cols.saturating_mul(tile_width);
    let height = rows.saturating_mul(tile_height);
    if width == 0 || height == 0 {
        return vec![Line::from("    [Composition preview unavailable]")];
    }

    let mut matrix = vec![false; width.saturating_mul(height)];
    for tile in tiles {
        let Some(info) = &tile.glyph.composition_tile else {
            continue;
        };
        if info.rows != rows || info.cols != cols {
            continue;
        }
        if tile.glyph.width as usize != tile_width || tile.glyph.height as usize != tile_height {
            continue;
        }
        for y in 0..tile_height {
            for x in 0..tile_width {
                let src_idx = y * tile_width + x;
                let dst_x = info.col * tile_width + x;
                let dst_y = info.row * tile_height + y;
                if dst_x >= width || dst_y >= height || src_idx >= tile.glyph.coverage.len() {
                    continue;
                }
                let dst_idx = dst_y * width + dst_x;
                matrix[dst_idx] = (tile.glyph.coverage[src_idx] >= threshold) ^ invert;
            }
        }
    }

    render_binary_preview_lines(&matrix, width, height, max_w, max_h, true, false, true)
}

fn crop_binary_matrix_to_active_bounds(
    matrix: &[bool],
    src_w: usize,
    src_h: usize,
) -> Option<(Vec<bool>, usize, usize)> {
    if matrix.is_empty() || src_w == 0 || src_h == 0 {
        return None;
    }

    let mut min_x = src_w;
    let mut min_y = src_h;
    let mut max_x = 0usize;
    let mut max_y = 0usize;
    let mut found_on = false;

    for y in 0..src_h {
        for x in 0..src_w {
            if !matrix[y * src_w + x] {
                continue;
            }
            found_on = true;
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }
    }

    if !found_on {
        return None;
    }

    let out_w = max_x - min_x + 1;
    let out_h = max_y - min_y + 1;
    let mut cropped = vec![false; out_w.saturating_mul(out_h)];
    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let src_idx = y * src_w + x;
            let dst_idx = (y - min_y) * out_w + (x - min_x);
            cropped[dst_idx] = matrix[src_idx];
        }
    }
    Some((cropped, out_w, out_h))
}

fn crop_binary_matrix_to_active_y_bounds(
    matrix: &[bool],
    src_w: usize,
    src_h: usize,
) -> Option<(Vec<bool>, usize, usize)> {
    if matrix.is_empty() || src_w == 0 || src_h == 0 {
        return None;
    }

    let mut min_y = src_h;
    let mut max_y = 0usize;
    let mut found_on = false;

    for y in 0..src_h {
        for x in 0..src_w {
            if !matrix[y * src_w + x] {
                continue;
            }
            found_on = true;
            min_y = min_y.min(y);
            max_y = max_y.max(y);
        }
    }

    if !found_on {
        return None;
    }

    let out_h = max_y - min_y + 1;
    let mut cropped = vec![false; src_w.saturating_mul(out_h)];
    for y in min_y..=max_y {
        let src_start = y * src_w;
        let dst_start = (y - min_y) * src_w;
        cropped[dst_start..dst_start + src_w]
            .copy_from_slice(&matrix[src_start..src_start + src_w]);
    }
    Some((cropped, src_w, out_h))
}

fn render_binary_preview_lines(
    matrix: &[bool],
    src_w: usize,
    src_h: usize,
    max_w: u16,
    max_h: u16,
    allow_upscale: bool,
    trim_empty_rows: bool,
    preserve_aspect: bool,
) -> Vec<Line<'static>> {
    const PREVIEW_X_COMP: f32 = 0.88;

    if matrix.is_empty() || src_w == 0 || src_h == 0 || max_w == 0 || max_h == 0 {
        return vec![Line::from("    [Preview too small]")];
    }

    let max_w = max_w as usize;
    let max_h = max_h as usize;
    let base_w = if allow_upscale {
        max_w
    } else {
        src_w.min(max_w)
    };
    let max_out_w = ((usize::max(1, base_w) as f32) * PREVIEW_X_COMP)
        .round()
        .max(1.0) as usize;
    let max_out_h = usize::max(
        1,
        if allow_upscale {
            max_h
        } else {
            src_h.min(max_h)
        },
    );
    let (out_w, out_h) = if preserve_aspect {
        let scale = (max_out_w as f64 / src_w as f64).min(max_out_h as f64 / src_h as f64);
        (
            ((src_w as f64 * scale).round() as usize).clamp(1, max_out_w),
            ((src_h as f64 * scale).round() as usize).clamp(1, max_out_h),
        )
    } else {
        (max_out_w, max_out_h)
    };
    let sample_idx = |out_idx: usize, out_len: usize, src_len: usize| -> usize {
        let numerator = (2 * out_idx + 1) * src_len;
        let denominator = 2 * out_len;
        (numerator / denominator).min(src_len.saturating_sub(1))
    };

    let mut rows = Vec::with_capacity(out_h);
    let sample_h = out_h.saturating_mul(2);
    for oy in 0..out_h {
        let sy_top = sample_idx(oy.saturating_mul(2), sample_h, src_h);
        let sy_bottom = sample_idx(oy.saturating_mul(2).saturating_add(1), sample_h, src_h);
        let mut row = String::with_capacity(out_w * 2 + 4);
        row.push_str("    ");
        for ox in 0..out_w {
            let sx = sample_idx(ox, out_w, src_w);
            let top_on = matrix[sy_top * src_w + sx];
            let bottom_on = matrix[sy_bottom * src_w + sx];
            let glyph = match (top_on, bottom_on) {
                (true, true) => '█',
                (true, false) => '▀',
                (false, true) => '▄',
                (false, false) => ' ',
            };
            if glyph == ' ' {
                row.push_str("  ");
            } else {
                row.push(glyph);
                row.push(glyph);
            }
        }
        rows.push(row);
    }
    if trim_empty_rows {
        rows.retain(|row| row.contains('█') || row.contains('▀') || row.contains('▄'));
    }
    if rows.is_empty() {
        return vec![Line::from("    [No visible pixels at threshold]")];
    }
    rows.into_iter().map(Line::from).collect()
}

fn preview_lines(
    glyph: &PreprocessedGlyph,
    threshold: u8,
    invert: bool,
    max_w: u16,
    max_h: u16,
) -> Vec<Line<'static>> {
    let src_w = glyph.width as usize;
    let src_h = glyph.height as usize;
    if src_w == 0 || src_h == 0 || max_w == 0 || max_h == 0 {
        return vec![Line::from("    [Preview too small]")];
    }

    let mut matrix = vec![false; src_w.saturating_mul(src_h)];

    for y in 0..src_h {
        for x in 0..src_w {
            let idx = y * src_w + x;
            if (glyph.coverage[idx] >= threshold) ^ invert {
                matrix[idx] = true;
            }
        }
    }

    let Some((cropped, crop_w, crop_h)) =
        crop_binary_matrix_to_active_y_bounds(&matrix, src_w, src_h)
    else {
        return vec![Line::from("    [No visible pixels at threshold]")];
    };

    render_binary_preview_lines(&cropped, crop_w, crop_h, max_w, max_h, true, true, true)
}

fn preview_lines_stable_frame(
    glyph: &PreprocessedGlyph,
    threshold: u8,
    invert: bool,
    max_w: u16,
    max_h: u16,
) -> Vec<Line<'static>> {
    let src_w = glyph.width as usize;
    let src_h = glyph.height as usize;
    if src_w == 0 || src_h == 0 || max_w == 0 || max_h == 0 {
        return vec![Line::from("    [Preview too small]")];
    }

    let mut matrix = vec![false; src_w.saturating_mul(src_h)];
    for y in 0..src_h {
        for x in 0..src_w {
            let idx = y * src_w + x;
            if (glyph.coverage[idx] >= threshold) ^ invert {
                matrix[idx] = true;
            }
        }
    }

    render_binary_preview_lines(&matrix, src_w, src_h, max_w, max_h, true, false, true)
}

fn preview_lines_from_coverage_stable_frame(
    coverage: &[u8],
    width_u32: u32,
    height_u32: u32,
    threshold: u8,
    invert: bool,
    max_w: u16,
    max_h: u16,
) -> Vec<Line<'static>> {
    let src_w = width_u32 as usize;
    let src_h = height_u32 as usize;
    if src_w == 0 || src_h == 0 || max_w == 0 || max_h == 0 {
        return vec![Line::from("    [Preview too small]")];
    }
    if coverage.len() != src_w.saturating_mul(src_h) {
        return vec![Line::from("    [Preview unavailable]")];
    }
    let mut matrix = vec![false; src_w.saturating_mul(src_h)];
    for y in 0..src_h {
        for x in 0..src_w {
            let idx = y * src_w + x;
            if (coverage[idx] >= threshold) ^ invert {
                matrix[idx] = true;
            }
        }
    }
    let Some((cropped, crop_w, crop_h)) =
        crop_binary_matrix_to_active_y_bounds(&matrix, src_w, src_h)
    else {
        return vec![Line::from("    [No visible pixels at threshold]")];
    };
    render_binary_preview_lines(&cropped, crop_w, crop_h, max_w, max_h, true, true, true)
}

