fn draw_home_creation_popup(frame: &mut Frame, app: &App, area: Rect, accent: Color, muted: Color) {
    let (kind, importing, tweaking, configuring_grid, configuring_animation) =
        match app.home_workflow {
            HomeWorkflow::Import(kind) => (kind, true, false, false, false),
            HomeWorkflow::Tweaking(kind) => (kind, false, true, false, false),
            HomeWorkflow::ConfigureGrid => (HomeCreationKind::Grid, false, false, true, false),
            HomeWorkflow::ConfigureAnimation(animation_type) => (
                match animation_type {
                    AnimationType::Standard => HomeCreationKind::AnimatedGlyph,
                    AnimationType::Grid => HomeCreationKind::AnimatedGridGlyph,
                },
                false,
                false,
                false,
                true,
            ),
            _ => return,
        };
    let popup_height = if tweaking {
        area.height.saturating_sub(2)
    } else {
        27
    };
    let popup = centered_popup_rect(area, 122, popup_height);
    frame.render_widget(Clear, popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(accent))
        .title(Span::styled(
            " Creation Workflow In Progress ",
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    let workflow_name = match kind {
        HomeCreationKind::Glyph => "Create glyph",
        HomeCreationKind::Grid => "Create grid",
        HomeCreationKind::AnimatedGlyph => "Create animated glyph",
        HomeCreationKind::AnimatedGridGlyph => "Create animated grid glyph",
    };
    let steps = if matches!(
        kind,
        HomeCreationKind::AnimatedGlyph | HomeCreationKind::AnimatedGridGlyph
    ) {
        vec![
            Line::from(vec![
                Span::styled(
                    " 1 ",
                    Style::default().fg(Color::Black).bg(if importing {
                        accent
                    } else {
                        Color::DarkGray
                    }),
                ),
                Span::styled(
                    " Import frame media ",
                    if importing {
                        Style::default().fg(Color::White)
                    } else {
                        Style::default().fg(muted)
                    },
                ),
                Span::styled(
                    if importing { "< current" } else { "< done" },
                    Style::default().fg(Color::Yellow),
                ),
            ]),
            Line::from(vec![
                Span::styled(
                    " 2 ",
                    Style::default().fg(Color::Black).bg(if tweaking {
                        accent
                    } else {
                        Color::DarkGray
                    }),
                ),
                Span::styled(
                    " Tweak grayscale / threshold / preview ",
                    if tweaking {
                        Style::default().fg(Color::White)
                    } else if importing {
                        Style::default().fg(muted)
                    } else {
                        Style::default().fg(Color::White)
                    },
                ),
                Span::styled(
                    if tweaking {
                        "< current"
                    } else if importing {
                        ""
                    } else {
                        "< done"
                    },
                    Style::default().fg(Color::Yellow),
                ),
            ]),
            Line::from(vec![
                Span::styled(
                    " 3 ",
                    Style::default()
                        .fg(Color::Black)
                        .bg(if configuring_animation {
                            accent
                        } else {
                            Color::DarkGray
                        }),
                ),
                Span::styled(
                    " Configure name/FPS/grid options in popup ",
                    if configuring_animation {
                        Style::default().fg(Color::White)
                    } else {
                        Style::default().fg(muted)
                    },
                ),
                Span::styled(
                    if configuring_animation {
                        "< current"
                    } else if importing || tweaking {
                        ""
                    } else {
                        "< done"
                    },
                    Style::default().fg(Color::Yellow),
                ),
            ]),
            Line::from(vec![
                Span::styled(" 4 ", Style::default().fg(Color::Black).bg(Color::DarkGray)),
                Span::styled(
                    " Create animation and switch to Glyphs ",
                    Style::default().fg(muted),
                ),
            ]),
        ]
    } else if matches!(kind, HomeCreationKind::Grid) {
        vec![
            Line::from(vec![
                Span::styled(
                    " 1 ",
                    Style::default().fg(Color::Black).bg(if importing {
                        accent
                    } else {
                        Color::DarkGray
                    }),
                ),
                Span::styled(
                    " Import source image ",
                    if importing {
                        Style::default().fg(Color::White)
                    } else {
                        Style::default().fg(muted)
                    },
                ),
                Span::styled(
                    if importing { "< current" } else { "< done" },
                    Style::default().fg(Color::Yellow),
                ),
            ]),
            Line::from(vec![
                Span::styled(
                    " 2 ",
                    Style::default().fg(Color::Black).bg(if tweaking {
                        accent
                    } else {
                        Color::DarkGray
                    }),
                ),
                Span::styled(
                    " Tweak grayscale / threshold / preview ",
                    if tweaking {
                        Style::default().fg(Color::White)
                    } else if importing {
                        Style::default().fg(muted)
                    } else {
                        Style::default().fg(Color::White)
                    },
                ),
                Span::styled(
                    if tweaking {
                        "< current"
                    } else if importing {
                        ""
                    } else {
                        "< done"
                    },
                    Style::default().fg(Color::Yellow),
                ),
            ]),
            Line::from(vec![
                Span::styled(
                    " 3 ",
                    Style::default().fg(Color::Black).bg(if configuring_grid {
                        accent
                    } else {
                        Color::DarkGray
                    }),
                ),
                Span::styled(
                    " Configure rows, columns, bleed in popup ",
                    if configuring_grid {
                        Style::default().fg(Color::White)
                    } else {
                        Style::default().fg(muted)
                    },
                ),
                Span::styled(
                    if configuring_grid {
                        "< current"
                    } else if importing || tweaking {
                        ""
                    } else {
                        "< done"
                    },
                    Style::default().fg(Color::Yellow),
                ),
            ]),
            Line::from(vec![
                Span::styled(" 4 ", Style::default().fg(Color::Black).bg(Color::DarkGray)),
                Span::styled(
                    " Create grid and switch to Glyphs ",
                    Style::default().fg(muted),
                ),
            ]),
        ]
    } else {
        vec![
            Line::from(vec![
                Span::styled(
                    " 1 ",
                    Style::default().fg(Color::Black).bg(if importing {
                        accent
                    } else {
                        Color::DarkGray
                    }),
                ),
                Span::styled(
                    " Import source images ",
                    if importing {
                        Style::default().fg(Color::White)
                    } else {
                        Style::default().fg(muted)
                    },
                ),
                Span::styled(
                    if importing { "< current" } else { "< done" },
                    Style::default().fg(Color::Yellow),
                ),
            ]),
            Line::from(vec![
                Span::styled(
                    " 2 ",
                    Style::default().fg(Color::Black).bg(if tweaking {
                        accent
                    } else {
                        Color::DarkGray
                    }),
                ),
                Span::styled(
                    " Tweak grayscale / threshold / preview ",
                    if tweaking {
                        Style::default().fg(Color::White)
                    } else if importing {
                        Style::default().fg(muted)
                    } else {
                        Style::default().fg(Color::White)
                    },
                ),
                Span::styled(
                    if tweaking {
                        "< current"
                    } else if importing {
                        ""
                    } else {
                        "< done"
                    },
                    Style::default().fg(Color::Yellow),
                ),
            ]),
            Line::from(vec![
                Span::styled(" 3 ", Style::default().fg(Color::Black).bg(Color::DarkGray)),
                Span::styled(" Continue to Glyphs panel ", Style::default().fg(muted)),
            ]),
        ]
    };
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(5),
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(2),
        ])
        .split(inner);
    frame.render_widget(
        Paragraph::new(vec![Line::from(vec![
            Span::raw("  "),
            Span::styled(
                if configuring_grid {
                    "Create grid: adjust rows/columns/bleed here, then activate Create Grid."
                        .to_string()
                } else if configuring_animation {
                    format!("{workflow_name}: configure in this popup, then create.")
                } else if tweaking {
                    format!("{workflow_name}: tweak grayscale/threshold with live preview.")
                } else {
                    let import_hint = if matches!(
                        kind,
                        HomeCreationKind::AnimatedGlyph | HomeCreationKind::AnimatedGridGlyph
                    ) {
                        "drop, paste, or drag images/GIFs/videos in this popup."
                    } else {
                        "drop, paste, or drag files in this popup."
                    };
                    format!("{workflow_name}: {import_hint}")
                },
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
        ])]),
        layout[0],
    );
    frame.render_widget(
        Paragraph::new(steps).block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(muted))
                .title(Span::styled(" Steps ", Style::default().fg(muted))),
        ),
        layout[1],
    );
    let error_line = app
        .home_workflow_error
        .as_ref()
        .map(|message| {
            Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    message.clone(),
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                ),
            ])
        })
        .unwrap_or_else(|| Line::from(""));
    frame.render_widget(Paragraph::new(vec![error_line]), layout[2]);

    if configuring_grid {
        if let Some(config) = &app.grid_config {
            draw_grid_config_ui(frame, app, config, layout[3], accent, muted);
        }
    } else if configuring_animation {
        if let GlyphToolMode::ConfigureAnimation(config) = &app.glyph_tool_mode {
            draw_animation_config_ui(frame, app, config, layout[3], accent, muted);
        } else {
            draw_animation_panel_ui(frame, app, layout[3], accent, muted);
        }
    } else if tweaking {
        draw_animation_import_workflow_ui(frame, app, layout[3], accent, muted, kind);
    } else {
        draw_home_import_drop_ui(frame, app, layout[3], accent, muted, kind);
    }
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw("  "),
            Span::styled(
                "Enter",
                Style::default().fg(accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                if configuring_grid {
                    " continue / create grid    "
                } else if configuring_animation {
                    " continue / create animation    "
                } else if tweaking {
                    if matches!(kind, HomeCreationKind::Glyph) {
                        if glyph_tweak_continue_finishes_workflow(app) {
                            " finish and open Glyphs    "
                        } else {
                            " next preview    "
                        }
                    } else {
                        " continue to next step    "
                    }
                } else {
                    " continue to tweaking step    "
                },
                Style::default().fg(muted),
            ),
            Span::styled(
                "Esc",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" cancel workflow", Style::default().fg(muted)),
        ])),
        layout[4],
    );
}

fn draw_grid_config_ui(
    frame: &mut Frame,
    _app: &App,
    config: &GridConfig,
    area: Rect,
    accent: Color,
    muted: Color,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(muted))
        .title(Span::styled(
            " Grid Configuration ",
            Style::default().fg(accent),
        ));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows_style = if config.focus == GridConfigFocus::Rows {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White).bg(Color::DarkGray)
    };
    let cols_style = if config.focus == GridConfigFocus::Cols {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White).bg(Color::DarkGray)
    };
    let horizontal_bleed_border_style = if config.focus == GridConfigFocus::HorizontalBleed {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White).bg(Color::DarkGray)
    };
    let vertical_bleed_border_style = if config.focus == GridConfigFocus::VerticalBleed {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White).bg(Color::DarkGray)
    };
    let create_style = if config.focus == GridConfigFocus::Create {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Blue)
            .add_modifier(Modifier::BOLD)
    };

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(3),
            Constraint::Length(2),
            Constraint::Min(0),
        ])
        .margin(2)
        .split(inner);

    let controls_width = 95u16;
    let centered_controls = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(controls_width),
            Constraint::Min(0),
        ])
        .split(layout[1]);
    let size_layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(11), // Rows
            Constraint::Length(11), // Cols
            Constraint::Length(28), // Left/right bleed
            Constraint::Length(28), // Top/bottom bleed
            Constraint::Length(2),  // spacer
            Constraint::Length(15), // Create
        ])
        .split(centered_controls[1]);
    let rows_text = format!(" Rows: {} ", config.rows);
    let cols_text = format!(" Cols: {} ", config.cols);
    let create_text = " Create Grid ";

    let header_text = format!(
        " Configuring grid for: {} ",
        source_display_name(&config.source_key)
    );
    frame.render_widget(
        Paragraph::new(header_text).style(Style::default().fg(Color::White)),
        layout[0],
    );

    frame.render_widget(
        Paragraph::new(rows_text)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(rows_style),
            )
            .style(rows_style),
        size_layout[0],
    );
    frame.render_widget(
        Paragraph::new(cols_text)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(cols_style),
            )
            .style(cols_style),
        size_layout[1],
    );
    frame.render_widget(
        Paragraph::new(bleed_toggle_line(
            " Left/right bleed ",
            config.horizontal_bleed,
        ))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(horizontal_bleed_border_style),
        )
        .style(Style::default().fg(Color::White).bg(Color::DarkGray)),
        size_layout[2],
    );
    frame.render_widget(
        Paragraph::new(bleed_toggle_line(
            " Top/bottom bleed ",
            config.vertical_bleed,
        ))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(vertical_bleed_border_style),
        )
        .style(Style::default().fg(Color::White).bg(Color::DarkGray)),
        size_layout[3],
    );
    frame.render_widget(
        Paragraph::new(create_text)
            .centered()
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(if config.focus == GridConfigFocus::Create {
                        BorderType::Thick
                    } else {
                        BorderType::Rounded
                    })
                    .border_style(create_style),
            )
            .style(create_style),
        size_layout[5],
    );

    let help_text = vec![Line::from(vec![
        Span::styled(" \u{2190}/\u{2192} ", Style::default().fg(accent)),
        Span::raw("move focus  "),
        Span::styled(" \u{2191}/\u{2193} ", Style::default().fg(accent)),
        Span::raw("adjust/cycle active value  "),
        Span::styled(" Enter ", Style::default().fg(accent)),
        Span::raw("create on button"),
    ])];
    frame.render_widget(Paragraph::new(help_text), layout[2]);

    let guidance_text = vec![
        Line::from(vec![
            Span::styled(" Rows/Cols: ", Style::default().fg(accent)),
            Span::raw("higher values create more glyph tiles and consume more terminal space."),
        ]),
        Line::from(vec![
            Span::styled(" Left/right bleed: ", Style::default().fg(accent)),
            Span::raw(
                "to hide vertical seams; usually safe across terminals (Ghostty, Alacritty, etc).",
            ),
        ]),
        Line::from(vec![
            Span::styled(" Top/bottom bleed: ", Style::default().fg(accent)),
            Span::raw(
                "different interline configs can mean inconsistent results across terminals; diagonal lines can also look wobblier because pixels are expanded straight up/down.",
            ),
        ]),
        Line::from(vec![
            Span::styled(" Recommended default: ", Style::default().fg(accent)),
            Span::raw("left/right = weak, top/bottom = off."),
        ]),
    ];
    frame.render_widget(
        Paragraph::new(guidance_text).wrap(Wrap { trim: true }),
        layout[3],
    );
}

fn draw_animation_config_ui(
    frame: &mut Frame,
    app: &App,
    config: &AnimationConfig,
    area: Rect,
    accent: Color,
    muted: Color,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(muted))
        .title(Span::styled(
            " Animation Configuration ",
            Style::default().fg(accent),
        ));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let focused_style = Style::default()
        .fg(Color::Black)
        .bg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    let idle_style = Style::default().fg(Color::White).bg(Color::DarkGray);
    let style_for = |focus: AnimationConfigFocus| {
        if config.focus == focus {
            focused_style
        } else {
            idle_style
        }
    };
    let create_style = if config.focus == AnimationConfigFocus::Create {
        focused_style
    } else {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Blue)
            .add_modifier(Modifier::BOLD)
    };

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(3),
            Constraint::Length(2),
            Constraint::Min(0),
        ])
        .margin(2)
        .split(inner);

    let type_label = match config.animation_type {
        AnimationType::Standard => "standard",
        AnimationType::Grid => "grid",
    };
    frame.render_widget(
        Paragraph::new(format!(
            " Configuring {type_label} animation (frames: {}) ",
            config.selected_frames.len()
        ))
        .style(Style::default().fg(Color::White)),
        layout[0],
    );

    let (row_constraints, controls_width) = if config.animation_type == AnimationType::Grid {
        (
            vec![
                Constraint::Length(11), // fps
                Constraint::Length(11), // rows
                Constraint::Length(11), // cols
                Constraint::Length(26), // lr bleed
                Constraint::Length(26), // tb bleed
                Constraint::Length(2),  // spacer
                Constraint::Length(22), // create
            ],
            109u16,
        )
    } else {
        (
            vec![
                Constraint::Length(11), // fps
                Constraint::Length(2),  // spacer
                Constraint::Length(22), // create
            ],
            35u16,
        )
    };
    let centered_controls = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(controls_width),
            Constraint::Min(0),
        ])
        .split(layout[1]);
    let fields = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(row_constraints)
        .split(centered_controls[1]);

    frame.render_widget(
        Paragraph::new(format!(" Name: {} ", config.animation_name))
            .style(Style::default().fg(muted)),
        layout[2],
    );
    let fps_style = style_for(AnimationConfigFocus::Fps);
    frame.render_widget(
        Paragraph::new(format!(" FPS: {} ", config.fps))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(fps_style),
            )
            .style(fps_style),
        fields[0],
    );

    let create_idx = if config.animation_type == AnimationType::Grid {
        let rows_style = style_for(AnimationConfigFocus::Rows);
        frame.render_widget(
            Paragraph::new(format!(" Rows: {} ", config.rows))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(rows_style),
                )
                .style(rows_style),
            fields[1],
        );
        let cols_style = style_for(AnimationConfigFocus::Cols);
        frame.render_widget(
            Paragraph::new(format!(" Cols: {} ", config.cols))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(cols_style),
                )
                .style(cols_style),
            fields[2],
        );
        frame.render_widget(
            Paragraph::new(bleed_toggle_line(
                " Left/right bleed ",
                config.horizontal_bleed,
            ))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(style_for(AnimationConfigFocus::HorizontalBleed)),
            )
            .style(style_for(AnimationConfigFocus::HorizontalBleed)),
            fields[3],
        );
        frame.render_widget(
            Paragraph::new(bleed_toggle_line(
                " Top/bottom bleed ",
                config.vertical_bleed,
            ))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(style_for(AnimationConfigFocus::VerticalBleed)),
            )
            .style(style_for(AnimationConfigFocus::VerticalBleed)),
            fields[4],
        );
        6
    } else {
        2
    };

    let create_label = if app.animation_create_in_progress() {
        format!(
            " Create Animation {} ",
            app.animation_create_spinner_frame()
        )
    } else {
        " Create Animation ".to_string()
    };
    frame.render_widget(
        Paragraph::new(create_label)
            .alignment(Alignment::Center)
            .style(create_style)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(create_style),
            ),
        fields[create_idx],
    );

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" FPS ", Style::default().fg(accent)),
            Span::styled("and grid options use Left/Right to focus, Up/Down to adjust, Enter on Create to finish.", Style::default().fg(muted)),
        ])),
        layout[2],
    );
}

fn next_bleed_level(level: BleedLevel) -> BleedLevel {
    match level {
        BleedLevel::Off => BleedLevel::Weak,
        BleedLevel::Weak => BleedLevel::Strong,
        BleedLevel::Strong => BleedLevel::Off,
    }
}

fn previous_bleed_level(level: BleedLevel) -> BleedLevel {
    match level {
        BleedLevel::Off => BleedLevel::Strong,
        BleedLevel::Weak => BleedLevel::Off,
        BleedLevel::Strong => BleedLevel::Weak,
    }
}

fn bleed_level_from_digit(digit: u32) -> BleedLevel {
    match digit {
        0 => BleedLevel::Off,
        2..=9 => BleedLevel::Strong,
        _ => BleedLevel::Weak,
    }
}

fn bleed_level_label(level: BleedLevel) -> &'static str {
    match level {
        BleedLevel::Off => "OFF",
        BleedLevel::Weak => "WEAK",
        BleedLevel::Strong => "STRONG",
    }
}

fn bleed_toggle_line(label: &'static str, level: BleedLevel) -> Line<'static> {
    let value = bleed_level_label(level);
    let value_style = match level {
        BleedLevel::Off => Style::default()
            .fg(Color::LightRed)
            .add_modifier(Modifier::BOLD),
        BleedLevel::Weak => Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD),
        BleedLevel::Strong => Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    };
    Line::from(vec![
        Span::raw(label),
        Span::styled(value, value_style),
        Span::raw(" "),
    ])
}

