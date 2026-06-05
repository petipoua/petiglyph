pub(crate) fn requested_keyboard_enhancement_flags() -> KeyboardEnhancementFlags {
    KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
}

fn is_valid_project_name_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.')
}

fn format_welcome_input_field_with_cursor(
    value: &str,
    editing: bool,
    cursor: usize,
    width: usize,
) -> String {
    let width = width.max(1);
    let mut field = vec![' '; width];

    if value.is_empty() && !editing {
        let placeholder = "<project-name>";
        for (idx, ch) in placeholder.chars().take(width).enumerate() {
            field[idx] = ch;
        }
    } else {
        for (idx, ch) in value.chars().take(width).enumerate() {
            field[idx] = ch;
        }

        if editing {
            let cursor_index = cursor.min(width - 1);
            field[cursor_index] = '_';
        }
    }

    let content = field.into_iter().collect::<String>();
    format!(" {content} ")
}

#[cfg(test)]
pub(crate) fn format_welcome_input_field(value: &str, focused: bool, width: usize) -> String {
    format_welcome_input_field_with_cursor(value.trim(), focused, value.chars().count(), width)
}

pub(crate) fn install_action_name(project_is_installed: bool) -> &'static str {
    if project_is_installed {
        "Reinstall"
    } else {
        "Install"
    }
}

#[cfg(test)]
pub(crate) fn format_projects_card_hint(focus: WelcomeFocus, editing: bool) -> String {
    format_projects_card_hint_for_display(focus, editing)
}

fn format_projects_card_hint_for_display(focus: WelcomeFocus, editing: bool) -> String {
    let hint = match (focus, editing) {
        (WelcomeFocus::CreateInput, true) => "typing (Enter/Esc to stop)",
        (WelcomeFocus::CreateInput, false) => "press Enter to create",
        _ => "",
    };

    format!("  {hint:<WELCOME_HINT_WIDTH$}")
}

fn reset_tui_debug_log() {
    if !tui_debug_enabled() {
        return;
    }

    let path = tui_debug_log_path();
    let now = debug_timestamp();
    let _ = fs::write(path, format!("[{now}] petiglyph TUI debug log reset\n"));
}

fn tui_debug_log(event: &str, details: impl AsRef<str>) {
    if !tui_debug_enabled() {
        return;
    }

    let path = tui_debug_log_path();
    let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) else {
        return;
    };

    let _ = writeln!(
        file,
        "[{}] {event}: {}",
        debug_timestamp(),
        details.as_ref()
    );
}

fn tui_debug_log_path() -> PathBuf {
    if let Ok(value) = env::var(TUI_DEBUG_LOG_ENV) {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }
    env::temp_dir().join(TUI_DEBUG_LOG_FILE_NAME)
}

fn tui_debug_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var("PETIGLYPH_TUI_DEBUG")
            .map(|value| {
                let value = value.trim().to_ascii_lowercase();
                !matches!(value.as_str(), "" | "0" | "false" | "off" | "no")
            })
            .unwrap_or(false)
    })
}

fn debug_timestamp() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}.{:03}", now.as_secs(), now.subsec_millis())
}

fn key_debug(key: &KeyEvent) -> String {
    format!(
        "code={:?} modifiers={:?} kind={:?} state={:?}",
        key.code, key.modifiers, key.kind, key.state
    )
}

pub(crate) fn should_dispatch_key_kind(kind: KeyEventKind) -> bool {
    matches!(kind, KeyEventKind::Press | KeyEventKind::Repeat)
}

fn app_debug_state(app: &App) -> String {
    format!(
        "view={:?} welcome_focus={:?} glyphs_focus={:?} grid_config={} selecting_for_grid={} selected_project={} editing={} verbose_paths={} input={:?} cursor={} visual_cursor={} install_task={} project_switch_task={} delete_confirm_selection={:?} renaming={} status={:?} quit={}",
        app.view,
        app.welcome_focus,
        app.glyphs_focus,
        app.grid_config.is_some(),
        app.selecting_for_grid,
        app.selected_project,
        app.welcome_input_editing,
        app.verbose_paths,
        app.create_input.value(),
        app.create_input.cursor(),
        app.create_input.visual_cursor(),
        app.install_task.is_some(),
        app.project_switch_task.is_some(),
        app.delete_project_confirm_selection,
        app.renaming_input.is_some(),
        app.status,
        app.quit
    )
}

