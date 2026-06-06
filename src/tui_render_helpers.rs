fn detected_terminal_name() -> Option<&'static str> {
    let term_program = env::var("TERM_PROGRAM")
        .unwrap_or_default()
        .to_ascii_lowercase();
    let term = env::var("TERM").unwrap_or_default().to_ascii_lowercase();

    if term_program.contains("ghostty")
        || term.contains("ghostty")
        || env::var_os("GHOSTTY_RESOURCES_DIR").is_some()
    {
        return Some("Ghostty");
    }

    if term_program.contains("alacritty") || env::var_os("ALACRITTY_SOCKET").is_some() {
        return Some("Alacritty");
    }

    if term_program.contains("wezterm") || env::var_os("WEZTERM_PANE").is_some() {
        return Some("WezTerm");
    }

    if term_program.contains("kitty")
        || term.contains("kitty")
        || env::var_os("KITTY_PID").is_some()
    {
        return Some("Kitty");
    }

    None
}

fn installed_fonts_restart_warning() -> String {
    if let Some(name) = detected_terminal_name() {
        return format!(
            "restart all {name} terminals to show new glyphs; if they still appear as errors or [?], reboot the computer"
        );
    }
    "restart all terminals to show new glyphs; if they still appear as errors or [?], reboot the computer"
        .to_string()
}

fn format_count_k(value: usize) -> String {
    if value >= 1_000 {
        let whole = value / 1_000;
        let tenth = (value % 1_000) / 100;
        format!("{whole}.{tenth}k")
    } else {
        value.to_string()
    }
}

fn supplementary_pua_usage_line(summary: Option<&crate::install::PuaUsageSummary>) -> String {
    let Some(summary) = summary else {
        return "supplementary PUA usage unavailable on this machine.".to_string();
    };

    let line = format!(
        "PUA usage: petiglyph {} / {} used; external {}; available {}",
        format_count_k(summary.petiglyph_occupied),
        format_count_k(summary.supplementary_pua_total),
        format_count_k(summary.external_occupied),
        format_count_k(summary.available)
    );
    line
}

fn visible_window_bounds(
    total_rows: usize,
    selected_row: usize,
    viewport_rows: usize,
) -> (usize, usize) {
    if total_rows == 0 || viewport_rows == 0 {
        return (0, 0);
    }

    if total_rows <= viewport_rows {
        return (0, total_rows);
    }

    let selected = selected_row.min(total_rows - 1);
    let half = viewport_rows / 2;
    let mut start = selected.saturating_sub(half);
    let max_start = total_rows - viewport_rows;
    if start > max_start {
        start = max_start;
    }
    let end = (start + viewport_rows).min(total_rows);
    (start, end)
}

fn scrollbar_thumb_geometry(
    total_rows: usize,
    viewport_rows: usize,
    viewport_start: usize,
) -> (usize, usize) {
    if total_rows == 0 || viewport_rows == 0 || total_rows <= viewport_rows {
        return (0, 0);
    }

    let thumb_height =
        ((viewport_rows.saturating_mul(viewport_rows)) + total_rows - 1) / total_rows;
    let thumb_height = thumb_height.max(1).min(viewport_rows);
    let track = viewport_rows.saturating_sub(thumb_height);
    let scrollable = total_rows.saturating_sub(viewport_rows);
    if track == 0 || scrollable == 0 {
        return (0, thumb_height);
    }

    let thumb_top = viewport_start.saturating_mul(track) / scrollable;
    (thumb_top.min(track), thumb_height)
}

fn vertical_scrollbar_lines(
    height: usize,
    thumb_top: usize,
    thumb_height: usize,
    track_color: Color,
    thumb_color: Color,
) -> Vec<Line<'static>> {
    let mut lines = Vec::with_capacity(height);
    let thumb_bottom = thumb_top.saturating_add(thumb_height);
    for row in 0..height {
        let in_thumb = row >= thumb_top && row < thumb_bottom;
        let (glyph, style) = if in_thumb {
            ("█", Style::default().fg(thumb_color))
        } else {
            ("│", Style::default().fg(track_color))
        };
        lines.push(Line::from(vec![Span::styled(glyph, style)]));
    }
    lines
}

fn drag_images_here_lines(
    available_width: u16,
    available_height: u16,
    accent: Color,
    imported_count: usize,
    animation_media_mode: bool,
    windows_picker_mode: bool,
    processing_spinner: Option<&str>,
    inline_notice: Option<&str>,
) -> Vec<Line<'static>> {
    let horizontal_padding = 4usize;
    let horizontal_pad = " ".repeat(horizontal_padding);
    if available_height < 3 {
        return Vec::new();
    }

    let max_line_width =
        usize::from(available_width.saturating_sub((horizontal_padding as u16).saturating_mul(2)));
    if max_line_width < 8 {
        return Vec::new();
    }

    let inner_width = max_line_width.saturating_sub(2);
    let top_border = format!("╭{}╮", dashed_pattern(inner_width));
    let bottom_border = format!("╰{}╯", dashed_pattern(inner_width));
    let side_for_row = |row: usize| if row % 2 == 0 { " " } else { "│" };
    let centered_label = center_label(
        creation_workflow_import_area_label(animation_media_mode, windows_picker_mode),
        inner_width,
    );
    let counter_text = if let Some(spinner) = processing_spinner {
        format!("Processing {spinner}")
    } else if imported_count > 0 {
        if animation_media_mode {
            format!("Media added: {imported_count} ✓")
        } else {
            format!("Images added: {imported_count} ✓")
        }
    } else {
        if animation_media_mode {
            format!("Media added: {imported_count}")
        } else {
            format!("Images added: {imported_count}")
        }
    };
    let counter_label = center_label(&counter_text, inner_width);
    let notice_label = inline_notice.map(|notice| center_label(notice, inner_width));
    let border_style = Style::default().fg(accent);
    let label_style = Style::default().fg(accent).add_modifier(Modifier::BOLD);
    let notice_style = Style::default().fg(Color::DarkGray);

    let inner_rows = available_height.saturating_sub(2);
    let label_row = usize::from(inner_rows / 2);
    let counter_row = (label_row + 1).min(usize::from(inner_rows.saturating_sub(1)));
    let notice_row = notice_label
        .as_ref()
        .map(|_| (counter_row + 1).min(usize::from(inner_rows.saturating_sub(1))));

    let mut lines = Vec::with_capacity(usize::from(available_height));
    lines.push(Line::from(vec![
        Span::raw(horizontal_pad.clone()),
        Span::styled(top_border, border_style),
    ]));

    for row in 0..usize::from(inner_rows) {
        let left_side = side_for_row(row);
        let right_side = side_for_row(row);
        if row == label_row {
            lines.push(Line::from(vec![
                Span::raw(horizontal_pad.clone()),
                Span::styled(left_side, border_style),
                Span::styled(centered_label.clone(), label_style),
                Span::styled(right_side, border_style),
            ]));
        } else if row == counter_row {
            lines.push(Line::from(vec![
                Span::raw(horizontal_pad.clone()),
                Span::styled(left_side, border_style),
                Span::styled(counter_label.clone(), Style::default().fg(Color::Gray)),
                Span::styled(right_side, border_style),
            ]));
        } else if Some(row) == notice_row && notice_row != Some(counter_row) {
            lines.push(Line::from(vec![
                Span::raw(horizontal_pad.clone()),
                Span::styled(left_side, border_style),
                Span::styled(notice_label.clone().unwrap_or_default(), notice_style),
                Span::styled(right_side, border_style),
            ]));
        } else {
            lines.push(Line::from(vec![
                Span::raw(horizontal_pad.clone()),
                Span::styled(
                    format!("{left_side}{}{right_side}", " ".repeat(inner_width)),
                    border_style,
                ),
            ]));
        }
    }

    lines.push(Line::from(vec![
        Span::raw(horizontal_pad),
        Span::styled(bottom_border, border_style),
    ]));
    lines
}

fn dashed_pattern(width: usize) -> String {
    let mut out = String::with_capacity(width);
    for idx in 0..width {
        out.push(if idx % 4 < 2 { '─' } else { ' ' });
    }
    out
}

fn center_label(label: &str, width: usize) -> String {
    let label_chars = label.chars().count();
    let label = if label_chars > width {
        label.chars().take(width).collect::<String>()
    } else {
        label.to_string()
    };
    let label_len = label.chars().count();
    let padding = width.saturating_sub(label_len);
    let left = padding / 2;
    let right = padding - left;
    format!("{}{}{}", " ".repeat(left), label, " ".repeat(right))
}

fn home_panel_button_style(selected: bool, accent: Color) -> Style {
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
}

fn render_home_panel_button(frame: &mut Frame, area: Rect, line: Line<'static>) {
    frame.render_widget(Paragraph::new(line).alignment(Alignment::Center), area);
}

fn padded_button_label(label: impl AsRef<str>) -> String {
    format!(" {} ", label.as_ref())
}

pub(crate) fn wrap_sample_for_display(sample: &str, max_chars: usize) -> Vec<String> {
    if sample.is_empty() {
        return Vec::new();
    }

    let target = max_chars.max(1);
    let mut lines = Vec::new();
    for logical_line in sample.split('\n') {
        if logical_line.is_empty() {
            lines.push(String::new());
            continue;
        }

        let mut current = String::new();
        let mut count = 0usize;
        for ch in logical_line.chars() {
            current.push(ch);
            count += 1;
            if count >= target {
                lines.push(current);
                current = String::new();
                count = 0;
            }
        }
        if !current.is_empty() {
            lines.push(current);
        }
    }

    lines
}

pub(crate) fn installed_font_block_display_lines(block: &str, max_chars: usize) -> Vec<String> {
    wrap_sample_for_display(block, max_chars)
}

fn installed_animation_frame_index(
    fps: u8,
    frame_count: usize,
    started_at: Instant,
    now: Instant,
) -> usize {
    if frame_count <= 1 {
        return 0;
    }

    let fps = u128::from(fps.max(1));
    let elapsed_ms = now.duration_since(started_at).as_millis();
    ((elapsed_ms.saturating_mul(fps) / 1000) as usize) % frame_count
}

fn animation_frame_interval(fps: u8) -> Duration {
    Duration::from_nanos(1_000_000_000u64 / u64::from(fps.max(1)))
}

fn step_animation_preview(preview: &mut AnimationPreview, animation: &AnimationDef, now: Instant) {
    let frame_count = animation.frames.len().max(1);
    if frame_count <= 1 {
        return;
    }

    let interval = animation_frame_interval(animation.fps);
    while now.duration_since(preview.last_frame_at) >= interval {
        preview.frame_index = (preview.frame_index + 1) % frame_count;
        preview.last_frame_at += interval;
    }
}

fn installed_animation_preview_lines(
    preview: &InstalledFontAnimationPreview,
    max_chars: usize,
    started_at: Instant,
    now: Instant,
) -> Option<Vec<String>> {
    if preview.frame_blocks.is_empty() {
        return None;
    }

    let frame_index =
        installed_animation_frame_index(preview.fps, preview.frame_blocks.len(), started_at, now);
    preview
        .frame_blocks
        .get(frame_index)
        .map(|block| installed_font_block_display_lines(block, max_chars))
}
