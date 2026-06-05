fn draw_welcome_view(
    frame: &mut Frame,
    app: &App,
    area: ratatui::layout::Rect,
    accent: Color,
    muted: Color,
) {
    let body = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),  // scan scope hint
            Constraint::Length(1),  // settings row
            Constraint::Length(21), // projects + current project
            Constraint::Min(0),     // installed petiglyph fonts
        ])
        .split(area);

    let tip_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(body[0]);
    let switch_notice_line = if let Some(notice) = &app.switch_notice {
        Line::from(vec![Span::styled(
            format!(
                " Switched project: {} -> {} ",
                notice.from_label, notice.to_label
            ),
            Style::default()
                .fg(Color::Black)
                .bg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )])
    } else {
        Line::from("")
    };
    frame.render_widget(
        Paragraph::new(switch_notice_line).alignment(Alignment::Center),
        tip_layout[0],
    );
    frame.render_widget(
        Paragraph::new(vec![Line::from(vec![
            Span::raw("  "),
            Span::styled(
                "Local scan scope: petiglyph checks the current folder and one level below for local projects/builds.",
                Style::default().fg(muted),
            ),
        ])])
        .wrap(Wrap { trim: true }),
        tip_layout[1],
    );
    let verbose_button_style = if app.welcome_focus == WelcomeFocus::VerbosePathsToggle {
        Style::default()
            .fg(Color::Black)
            .bg(accent)
            .add_modifier(Modifier::BOLD)
    } else if app.verbose_paths {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Green)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(Color::White)
            .bg(Color::DarkGray)
            .add_modifier(Modifier::DIM)
    };
    let verbose_state_label = if app.verbose_paths { "ON" } else { "OFF" };
    frame.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            format!(" Verbose: {verbose_state_label} "),
            verbose_button_style,
        )]))
        .alignment(Alignment::Right),
        body[1],
    );

    let main = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(54), Constraint::Percentage(46)])
        .split(body[2]);

    let projects_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(muted))
        .title(Span::styled(
            " Petiglyph projects ",
            Style::default().fg(accent),
        ));

    let mut project_rows = Vec::new();
    if app.projects.is_empty() {
        project_rows.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(
                "No project detected in this folder.",
                Style::default().fg(Color::Yellow),
            ),
        ]));
    } else {
        let switching_target = app.project_switch_target_manifest_path();
        let switching_spinner = app.project_switch_spinner_frame();
        for (idx, project) in app.projects.iter().enumerate() {
            let is_active = app
                .active_project
                .as_ref()
                .is_some_and(|active| active == &project.manifest_path);
            let is_switching_target =
                switching_target.is_some_and(|target| target == project.manifest_path.as_path());
            let is_selected =
                app.welcome_focus == WelcomeFocus::ProjectList && app.selected_project == idx;
            let is_renaming = is_active && app.renaming_input.is_some();
            let marker = if is_active { "active" } else { "found " };
            let row_style = if is_selected {
                Style::default()
                    .fg(Color::Black)
                    .bg(accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            let mut row = vec![
                Span::styled(
                    if is_selected { "> " } else { "  " },
                    if is_selected {
                        Style::default()
                            .fg(Color::Black)
                            .bg(accent)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(muted)
                    },
                ),
                Span::styled(
                    format!("[{marker}] "),
                    if is_selected {
                        row_style
                    } else if is_active {
                        Style::default().fg(accent).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(muted)
                    },
                ),
            ];
            if is_renaming {
                let rename_width = 24usize;
                let input_scroll = app
                    .renaming_input
                    .as_ref()
                    .map_or(0, |inp| inp.visual_scroll(rename_width));
                let visible_input = app.renaming_input.as_ref().map_or(String::new(), |inp| {
                    inp.value()
                        .chars()
                        .skip(input_scroll)
                        .take(rename_width)
                        .collect()
                });
                let input_cursor = app
                    .renaming_input
                    .as_ref()
                    .map_or(0, |inp| inp.visual_cursor().saturating_sub(input_scroll));
                let input_value = format_welcome_input_field_with_cursor(
                    &visible_input,
                    true,
                    input_cursor,
                    rename_width,
                );
                row.push(Span::styled(
                    input_value,
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ));
                row.push(Span::styled(
                    " [renaming...]",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ));
            } else {
                row.push(Span::styled(
                    &project.font_name,
                    if is_selected {
                        row_style
                    } else if is_active {
                        Style::default()
                            .fg(Color::White)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::White)
                    },
                ));
                if is_switching_target {
                    if let Some(spinner) = switching_spinner {
                        row.push(Span::styled(
                            format!("  {spinner}"),
                            Style::default()
                                .fg(Color::Yellow)
                                .add_modifier(Modifier::BOLD),
                        ));
                    }
                    row.push(Span::styled(
                        " loading...",
                        Style::default().fg(Color::Yellow),
                    ));
                } else if let Some(warning) = &project.manifest_warning {
                    row.push(Span::styled(
                        format!("  {warning}"),
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    ));
                }
            }
            if app.verbose_paths && !is_renaming {
                row.push(Span::styled(
                    "  ",
                    if is_selected {
                        row_style
                    } else {
                        Style::default().fg(muted)
                    },
                ));
                row.push(Span::styled(
                    project.manifest_path.display().to_string(),
                    if is_selected {
                        row_style
                    } else {
                        Style::default().fg(muted)
                    },
                ));
            }
            project_rows.push(Line::from(row));
        }
    }
    let cursor_prefix =
        if app.welcome_focus == WelcomeFocus::CreateInput && !app.welcome_input_editing {
            "> "
        } else if app.welcome_focus == WelcomeFocus::CreateInput && app.welcome_input_editing {
            "> "
        } else {
            "  "
        };
    let cursor_style =
        if app.welcome_focus == WelcomeFocus::CreateInput && !app.welcome_input_editing {
            Style::default()
                .fg(Color::Black)
                .bg(accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(muted)
        };
    let is_create_focused =
        app.welcome_focus == WelcomeFocus::CreateInput && !app.welcome_input_editing;
    let create_button_style = if is_create_focused {
        Style::default()
            .fg(Color::Black)
            .bg(accent)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White).bg(Color::DarkGray)
    };
    let mut new_project_line = Vec::new();
    new_project_line.push(Span::styled(cursor_prefix, cursor_style));
    if app.welcome_input_editing {
        let input_scroll = app.create_input.visual_scroll(WELCOME_INPUT_WIDTH);
        let visible_input = app
            .create_input
            .value()
            .chars()
            .skip(input_scroll)
            .take(WELCOME_INPUT_WIDTH)
            .collect::<String>();
        let input_cursor = app
            .create_input
            .visual_cursor()
            .saturating_sub(input_scroll);
        let input_value = format_welcome_input_field_with_cursor(
            &visible_input,
            true,
            input_cursor,
            WELCOME_INPUT_WIDTH,
        );
        new_project_line.push(Span::styled("New project: ", Style::default().fg(muted)));
        new_project_line.push(Span::styled(
            input_value,
            Style::default()
                .fg(Color::Black)
                .bg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
    } else {
        new_project_line.push(Span::styled(" Create new project ", create_button_style));
    }
    new_project_line.push(Span::styled(
        format_projects_card_hint_for_display(app.welcome_focus, app.welcome_input_editing),
        Style::default().fg(muted),
    ));
    let projects_footer_lines = vec![Line::from(""), Line::from(new_project_line), Line::from("")];

    let projects_inner = projects_block.inner(main[0]);
    frame.render_widget(projects_block, main[0]);

    let projects_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(projects_footer_lines.len() as u16),
        ])
        .split(projects_inner);

    frame.render_widget(
        Paragraph::new(vec![Line::from("")]).wrap(Wrap { trim: false }),
        projects_layout[0],
    );

    let visible_project_rows = usize::from(projects_layout[1].height);
    let selected_project_row = if app.projects.is_empty() {
        0
    } else {
        app.selected_project.min(app.projects.len() - 1)
    };
    let (project_row_start, project_row_end) = visible_window_bounds(
        project_rows.len(),
        selected_project_row,
        visible_project_rows,
    );
    let rendered_project_rows = if project_row_start < project_row_end {
        project_rows[project_row_start..project_row_end].to_vec()
    } else {
        Vec::new()
    };
    let show_project_scrollbar =
        project_rows.len() > visible_project_rows && visible_project_rows > 0;
    let project_list_layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(if show_project_scrollbar { 1 } else { 0 }),
        ])
        .split(projects_layout[1]);
    frame.render_widget(
        Paragraph::new(rendered_project_rows).wrap(Wrap { trim: false }),
        project_list_layout[0],
    );
    if show_project_scrollbar {
        let (thumb_top, thumb_height) =
            scrollbar_thumb_geometry(project_rows.len(), visible_project_rows, project_row_start);
        frame.render_widget(
            Paragraph::new(vertical_scrollbar_lines(
                visible_project_rows,
                thumb_top,
                thumb_height,
                muted,
                accent,
            )),
            project_list_layout[1],
        );
    }

    frame.render_widget(
        Paragraph::new(projects_footer_lines).wrap(Wrap { trim: false }),
        projects_layout[2],
    );

    let current_project_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(muted))
        .title(Span::styled(
            " Current project ",
            Style::default().fg(accent),
        ));

    let ok_style = Style::default().fg(Color::Green);
    let missing_style = Style::default().fg(Color::Red);
    let unbuilt_style = Style::default().fg(Color::Rgb(255, 165, 0));
    let tools_active = app.active_project.is_some();
    let section_style = Style::default().fg(muted);
    let hint_style = if tools_active {
        Style::default().fg(muted)
    } else {
        Style::default().fg(Color::Yellow)
    };

    let (ttf_status, bdf_status, installed_status, ttf_built, bdf_built) =
        if app.active_project.is_none() {
            (
                Span::styled("Select a project first", missing_style),
                Span::styled("Select a project first", missing_style),
                Span::styled("Select a project first", missing_style),
                false,
                false,
            )
        } else {
            let ttf_path = expected_ttf_path(&app.config);
            let bdf_path = expected_bdf_path(&app.config);
            let ttf_built = ttf_path.is_file();
            let bdf_built = bdf_path.is_file();
            let ttf_status = if ttf_built {
                if app.verbose_paths {
                    Span::styled(format!("built: {}", ttf_path.display()), ok_style)
                } else {
                    Span::styled("built", ok_style)
                }
            } else {
                Span::styled("not built yet", unbuilt_style)
            };
            let bdf_status = if bdf_built {
                if app.verbose_paths {
                    Span::styled(format!("built: {}", bdf_path.display()), ok_style)
                } else {
                    Span::styled("built", ok_style)
                }
            } else {
                Span::styled("not built yet", unbuilt_style)
            };
            let installed_status = match &app.installed_font_path {
                Some(_) => Span::styled("✓", ok_style),
                None => Span::styled("✗", missing_style),
            };
            (
                ttf_status,
                bdf_status,
                installed_status,
                ttf_built,
                bdf_built,
            )
        };

    let current_project_summary = if tools_active {
        app.active_project_label()
    } else {
        "Select or create a project to see project-local status.".to_string()
    };

    let selected_button_style = Style::default()
        .fg(Color::Black)
        .bg(accent)
        .add_modifier(Modifier::BOLD);
    let idle_button_style = Style::default()
        .fg(Color::White)
        .bg(Color::DarkGray)
        .add_modifier(Modifier::BOLD);
    let disabled_button_style = Style::default()
        .fg(Color::DarkGray)
        .bg(Color::Black)
        .add_modifier(Modifier::DIM);

    let install_button_style = if app.active_project.is_none() && !app.install_in_progress() {
        disabled_button_style
    } else if let Some(FontTaskKind::Install) = app.font_task_kind() {
        app.font_task_button_style()
            .unwrap_or(disabled_button_style)
    } else if app.install_in_progress() {
        disabled_button_style
    } else if app.welcome_focus == WelcomeFocus::InstallButton {
        selected_button_style
    } else {
        idle_button_style
    };
    let install_label = match (app.font_task_kind(), app.font_task_spinner_frame()) {
        (Some(FontTaskKind::Install), Some(spinner)) => format!(" {spinner} Installing... "),
        _ => format!(
            " {} ",
            install_action_name(app.current_project_is_installed())
        ),
    };

    let delete_button_style = if !app.active_project_can_be_deleted() || app.install_in_progress() {
        disabled_button_style
    } else if app.welcome_focus == WelcomeFocus::DeleteProjectButton {
        Style::default()
            .fg(Color::White)
            .bg(Color::Red)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(Color::LightRed)
            .bg(Color::DarkGray)
            .add_modifier(Modifier::BOLD)
    };

    let current_project_inner = current_project_block.inner(main[1]);
    frame.render_widget(current_project_block, main[1]);

    let show_add_images_warning = tools_active && !ttf_built && !bdf_built && app.glyphs.is_empty();
    let glyph_count_label = if tools_active {
        app.live_glyph_source_count
            .map(|count| count.to_string())
            .unwrap_or_else(|| app.glyphs.len().to_string())
    } else {
        "n/a".to_string()
    };
    let mut current_project_lines = vec![
        Line::from(vec![
            Span::raw("  "),
            Span::styled(current_project_summary, hint_style),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::raw("  "),
            Span::styled(
                format!("Glyphs: {glyph_count_label}"),
                section_style.add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::raw("    "),
            Span::styled("TTF: ", Style::default().fg(muted)),
            ttf_status,
        ]),
        Line::from(vec![
            Span::raw("    "),
            Span::styled("BDF: ", Style::default().fg(muted)),
            bdf_status,
        ]),
        Line::from(vec![
            Span::raw("    "),
            Span::styled("Installed: ", Style::default().fg(muted)),
            installed_status,
        ]),
        Line::from(""),
    ];
    if show_add_images_warning {
        current_project_lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(
                "you need to add images to build the font",
                unbuilt_style.add_modifier(Modifier::BOLD),
            ),
        ]));
        current_project_lines.push(Line::from(""));
    }
    let top_lines_height = current_project_lines.len() as u16;
    let current_project_sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(top_lines_height.min(current_project_inner.height)),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(current_project_inner);

    frame.render_widget(
        Paragraph::new(current_project_lines).wrap(Wrap { trim: true }),
        current_project_sections[0],
    );
    let actions_layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(0), Constraint::Length(16)])
        .split(current_project_sections[1]);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw("  "),
            Span::styled("Actions: ", Style::default().fg(muted)),
            Span::styled(install_label, install_button_style),
        ])),
        actions_layout[0],
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            " Delete project ",
            delete_button_style,
        )]))
        .alignment(Alignment::Right),
        actions_layout[1],
    );
    if tools_active {
        draw_home_creation_area(frame, app, current_project_sections[3], accent, muted);
    }

    let fonts_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(muted))
        .title(Span::styled(
            " Installed petiglyph fonts ",
            Style::default().fg(accent),
        ));

    let installed_font_count = app.installed_fonts.len();
    let fonts_header = vec![
        Line::from(""),
        Line::from(vec![
            Span::raw("  "),
            Span::styled(
                format!(
                    "Found {installed_font_count} installed petiglyph font{}.",
                    if installed_font_count == 1 { "" } else { "s" }
                ),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::raw("  "),
            Span::styled(
                supplementary_pua_usage_line(app.pua_usage_summary.as_ref()),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::raw("  "),
            Span::styled(
                installed_fonts_restart_warning(),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(""),
    ];
    let mut font_rows = Vec::new();
    let mut selected_font_row_idx = 0usize;

    if app.installed_fonts.is_empty() {
        font_rows.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(
                "No installed petiglyph TTF fonts found.",
                Style::default().fg(muted),
            ),
        ]));
    } else {
        let sample_wrap_width = usize::from(body[3].width.saturating_sub(12).max(16));
        let now = Instant::now();

        for (f_idx, font) in app.installed_fonts.iter().enumerate() {
            let is_selected_font = f_idx == app.selected_installed_font;

            // Row 0: Name / Path / Uninstall
            {
                let is_focused = is_selected_font
                    && app.selected_installed_font_sub_index == 0
                    && app.welcome_focus == WelcomeFocus::InstalledFontList;

                if is_focused {
                    selected_font_row_idx = font_rows.len();
                }

                let base_style = if is_focused && !app.installed_font_horizontal_focus_uninstall {
                    Style::default()
                        .bg(accent)
                        .fg(Color::Black)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };

                let bullet = if is_selected_font && app.selected_installed_font_sub_index == 0 {
                    " ● "
                } else {
                    " ○ "
                };
                let mut name_spans = vec![
                    Span::styled(
                        bullet,
                        Style::default().fg(
                            if is_focused && !app.installed_font_horizontal_focus_uninstall {
                                Color::White
                            } else {
                                Color::Reset
                            },
                        ),
                    ),
                    Span::styled(&font.file_name, base_style),
                ];

                if app.verbose_paths {
                    name_spans.push(Span::styled(
                        format!("  ({})", font.path.display()),
                        if is_focused && !app.installed_font_horizontal_focus_uninstall {
                            base_style.fg(Color::Black)
                        } else {
                            Style::default().fg(muted)
                        },
                    ));
                }

                if is_focused && !app.installed_font_horizontal_focus_uninstall {
                    if let Some((at, id)) = &app.last_copy_notification {
                        if id == &format!("{}-path", f_idx)
                            && now.duration_since(*at) < Duration::from_millis(1500)
                        {
                            name_spans.push(Span::raw("  "));
                            name_spans.push(Span::styled(
                                "copied to clipboard",
                                Style::default()
                                    .fg(Color::Yellow)
                                    .add_modifier(Modifier::BOLD),
                            ));
                        }
                    }
                }

                let mut title_line = Line::from(name_spans);

                // Add Uninstall button
                title_line.spans.push(Span::raw("  "));
                let uninstall_button_style =
                    if app.is_selected_font_uninstall_in_progress(&font.path) {
                        app.font_task_button_style()
                            .unwrap_or(disabled_button_style)
                    } else if app.install_in_progress() {
                        disabled_button_style
                    } else if is_focused && app.installed_font_horizontal_focus_uninstall {
                        selected_button_style
                    } else {
                        idle_button_style
                    };
                let uninstall_label = if app.is_selected_font_uninstall_in_progress(&font.path) {
                    if let Some(spinner) = app.font_task_spinner_frame() {
                        format!(" {spinner} Removing... ")
                    } else {
                        " Uninstall Font ".to_string()
                    }
                } else {
                    " Uninstall Font ".to_string()
                };
                title_line
                    .spans
                    .push(Span::styled(uninstall_label, uninstall_button_style));

                font_rows.push(title_line);
            }

            // Row 1..N: Blocks (Selectable individually)
            for (b_idx, sample_block) in font.blocks.iter().enumerate() {
                let sub_idx = b_idx + 1;
                let is_focused = is_selected_font
                    && app.selected_installed_font_sub_index == sub_idx
                    && app.welcome_focus == WelcomeFocus::InstalledFontList;

                let wrapped_lines =
                    installed_font_block_display_lines(&sample_block.block, sample_wrap_width);

                if is_focused {
                    selected_font_row_idx = font_rows.len() + wrapped_lines.len().saturating_sub(1);
                }

                let base_style = if is_focused {
                    Style::default()
                        .bg(accent)
                        .fg(Color::Black)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(accent).add_modifier(Modifier::BOLD)
                };

                let bullet = if is_selected_font && app.selected_installed_font_sub_index == sub_idx
                {
                    " ● "
                } else {
                    " ○ "
                };

                let detail_style = if is_focused {
                    Style::default()
                        .bg(accent)
                        .fg(Color::Black)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                        .fg(Color::Magenta)
                        .add_modifier(Modifier::BOLD)
                };
                let mut detail_spans = vec![
                    Span::styled(
                        bullet,
                        Style::default().fg(if is_focused {
                            Color::White
                        } else {
                            Color::Reset
                        }),
                    ),
                    Span::styled(sample_block.label.clone(), detail_style),
                ];
                if is_focused {
                    if let Some((at, id)) = &app.last_copy_notification {
                        if id == &format!("{}-sample-{}", f_idx, b_idx)
                            && now.duration_since(*at) < Duration::from_millis(1500)
                        {
                            detail_spans.push(Span::raw("  "));
                            detail_spans.push(Span::styled(
                                "copied to clipboard",
                                Style::default()
                                    .fg(Color::Yellow)
                                    .add_modifier(Modifier::BOLD),
                            ));
                        }
                    }
                }
                font_rows.push(Line::from(detail_spans));

                for line_text in wrapped_lines {
                    let spans = vec![Span::raw("   "), Span::styled(line_text, base_style)];
                    font_rows.push(Line::from(spans));
                }
            }

            for (a_idx, animation_row) in font.animation_rows.iter().enumerate() {
                let sub_idx = 1 + font.blocks.len() + a_idx;
                let is_focused = is_selected_font
                    && app.selected_installed_font_sub_index == sub_idx
                    && app.welcome_focus == WelcomeFocus::InstalledFontList;
                let preview = font.animation_previews.get(a_idx);
                let preview_lines = preview
                    .and_then(|preview| {
                        installed_animation_preview_lines(
                            preview,
                            sample_wrap_width,
                            app.installed_animation_started_at,
                            now,
                        )
                    })
                    .unwrap_or_default();

                if is_focused {
                    selected_font_row_idx = font_rows.len() + preview_lines.len().saturating_sub(1);
                }

                let bullet = if is_selected_font && app.selected_installed_font_sub_index == sub_idx
                {
                    " ● "
                } else {
                    " ○ "
                };
                let mut spans = vec![
                    Span::styled(
                        bullet,
                        Style::default().fg(if is_focused {
                            Color::White
                        } else {
                            Color::Reset
                        }),
                    ),
                    Span::styled(
                        animation_row.clone(),
                        if is_focused {
                            Style::default()
                                .bg(accent)
                                .fg(Color::Black)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default()
                                .fg(Color::Magenta)
                                .add_modifier(Modifier::BOLD)
                        },
                    ),
                ];
                if let Some(preview) = preview {
                    if preview.frame_blocks.len() > 1 {
                        spans.push(Span::styled(
                            format!(
                                "  frame {}/{}",
                                installed_animation_frame_index(
                                    preview.fps,
                                    preview.frame_blocks.len(),
                                    app.installed_animation_started_at,
                                    now,
                                ) + 1,
                                preview.frame_blocks.len()
                            ),
                            if is_focused {
                                Style::default().bg(accent).fg(Color::Black)
                            } else {
                                Style::default().fg(muted)
                            },
                        ));
                    }
                }
                if is_focused {
                    if let Some((at, id)) = &app.last_copy_notification {
                        if id == &format!("{}-animation-{}", f_idx, a_idx)
                            && now.duration_since(*at) < Duration::from_millis(1500)
                        {
                            spans.push(Span::raw("  "));
                            spans.push(Span::styled(
                                "copied to clipboard",
                                Style::default()
                                    .fg(Color::Yellow)
                                    .add_modifier(Modifier::BOLD),
                            ));
                        }
                    }
                }
                font_rows.push(Line::from(spans));
                let preview_style = if is_focused {
                    Style::default()
                        .bg(accent)
                        .fg(Color::Black)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(accent).add_modifier(Modifier::BOLD)
                };
                for line_text in preview_lines {
                    font_rows.push(Line::from(vec![
                        Span::raw("   "),
                        Span::styled(line_text, preview_style),
                    ]));
                }
            }

            font_rows.push(Line::from(""));
        }
    }
    let fonts_inner = fonts_block.inner(body[3]);
    frame.render_widget(fonts_block, body[3]);

    let fonts_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(fonts_header.len() as u16),
            Constraint::Min(0),
        ])
        .split(fonts_inner);

    frame.render_widget(
        Paragraph::new(fonts_header).wrap(Wrap { trim: false }),
        fonts_layout[0],
    );

    let visible_font_rows = usize::from(fonts_layout[1].height);
    let (font_row_start, font_row_end) = visible_window_bounds(
        font_rows.len(),
        selected_font_row_idx.min(font_rows.len().saturating_sub(1)),
        visible_font_rows,
    );
    let rendered_font_rows = if font_row_start < font_row_end {
        font_rows[font_row_start..font_row_end].to_vec()
    } else {
        Vec::new()
    };
    let show_font_scrollbar = font_rows.len() > visible_font_rows && visible_font_rows > 0;
    let font_list_layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(if show_font_scrollbar { 1 } else { 0 }),
        ])
        .split(fonts_layout[1]);
    frame.render_widget(
        Paragraph::new(rendered_font_rows)
            .wrap(Wrap { trim: false })
            .style(Style::default()),
        font_list_layout[0],
    );
    if show_font_scrollbar {
        let (thumb_top, thumb_height) =
            scrollbar_thumb_geometry(font_rows.len(), visible_font_rows, font_row_start);
        frame.render_widget(
            Paragraph::new(vertical_scrollbar_lines(
                visible_font_rows,
                thumb_top,
                thumb_height,
                muted,
                accent,
            )),
            font_list_layout[1],
        );
    }
}

