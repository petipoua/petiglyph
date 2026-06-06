fn handle_grid_config_key(app: &mut App, config: &mut GridConfig, key: KeyEvent) -> Result<()> {
    let code = key.code;
    match code {
        KeyCode::Esc | KeyCode::Char('q') => {
            app.grid_config = None;
            app.status = Some("grid configuration canceled".to_string());
        }
        KeyCode::Left | KeyCode::Char('h') => {
            config.focus = match config.focus {
                GridConfigFocus::Rows => GridConfigFocus::Rows,
                GridConfigFocus::Cols => GridConfigFocus::Rows,
                GridConfigFocus::HorizontalBleed => GridConfigFocus::Cols,
                GridConfigFocus::VerticalBleed => GridConfigFocus::HorizontalBleed,
                GridConfigFocus::Create => GridConfigFocus::VerticalBleed,
            };
        }
        KeyCode::Right | KeyCode::Char('l') => {
            config.focus = match config.focus {
                GridConfigFocus::Rows => GridConfigFocus::Cols,
                GridConfigFocus::Cols => GridConfigFocus::HorizontalBleed,
                GridConfigFocus::HorizontalBleed => GridConfigFocus::VerticalBleed,
                GridConfigFocus::VerticalBleed => GridConfigFocus::Create,
                GridConfigFocus::Create => GridConfigFocus::Create,
            };
        }
        KeyCode::Up | KeyCode::Char('k') => match config.focus {
            GridConfigFocus::Rows => config.rows = config.rows.saturating_add(1).max(1),
            GridConfigFocus::Cols => config.cols = config.cols.saturating_add(1).max(1),
            GridConfigFocus::HorizontalBleed => {
                config.horizontal_bleed = next_bleed_level(config.horizontal_bleed)
            }
            GridConfigFocus::VerticalBleed => {
                config.vertical_bleed = next_bleed_level(config.vertical_bleed)
            }
            GridConfigFocus::Create => {}
        },
        KeyCode::Down | KeyCode::Char('j') => match config.focus {
            GridConfigFocus::Rows => config.rows = config.rows.saturating_sub(1).max(1),
            GridConfigFocus::Cols => config.cols = config.cols.saturating_sub(1).max(1),
            GridConfigFocus::HorizontalBleed => {
                config.horizontal_bleed = previous_bleed_level(config.horizontal_bleed)
            }
            GridConfigFocus::VerticalBleed => {
                config.vertical_bleed = previous_bleed_level(config.vertical_bleed)
            }
            GridConfigFocus::Create => {}
        },
        KeyCode::Char(ch) if ch.is_ascii_digit() => {
            let digit = ch.to_digit(10).unwrap_or(0);
            match config.focus {
                GridConfigFocus::Rows => {
                    if config.rows < 10 {
                        config.rows = config.rows * 10 + digit;
                    } else {
                        config.rows = digit;
                    }
                    if config.rows == 0 {
                        config.rows = 1;
                    }
                }
                GridConfigFocus::Cols => {
                    if config.cols < 10 {
                        config.cols = config.cols * 10 + digit;
                    } else {
                        config.cols = digit;
                    }
                    if config.cols == 0 {
                        config.cols = 1;
                    }
                }
                GridConfigFocus::HorizontalBleed => {
                    config.horizontal_bleed = bleed_level_from_digit(digit)
                }
                GridConfigFocus::VerticalBleed => {
                    config.vertical_bleed = bleed_level_from_digit(digit)
                }
                GridConfigFocus::Create => {}
            }
        }
        KeyCode::Char(' ') => match config.focus {
            GridConfigFocus::HorizontalBleed => {
                config.horizontal_bleed = next_bleed_level(config.horizontal_bleed)
            }
            GridConfigFocus::VerticalBleed => {
                config.vertical_bleed = next_bleed_level(config.vertical_bleed)
            }
            GridConfigFocus::Rows | GridConfigFocus::Cols | GridConfigFocus::Create => {}
        },
        KeyCode::Backspace => {
            match config.focus {
                GridConfigFocus::Rows => config.rows /= 10,
                GridConfigFocus::Cols => config.cols /= 10,
                GridConfigFocus::HorizontalBleed => config.horizontal_bleed = BleedLevel::Weak,
                GridConfigFocus::VerticalBleed => config.vertical_bleed = BleedLevel::Weak,
                GridConfigFocus::Create => {}
            }
            if config.rows == 0 {
                config.rows = 1;
            }
            if config.cols == 0 {
                config.cols = 1;
            }
        }
        KeyCode::Enter => {
            if config.focus == GridConfigFocus::Create {
                let source_key = config.source_key.clone();
                let rows = config.rows as usize;
                let cols = config.cols as usize;

                persist_composition_definition(
                    &app.manifest_path,
                    &source_key,
                    Some(CompositionDef {
                        rows,
                        cols,
                        horizontal_bleed: config.horizontal_bleed,
                        vertical_bleed: config.vertical_bleed,
                    }),
                )?;
                app.reload_glyphs()?;
                app.grid_config = None;
                if !matches!(app.home_workflow, HomeWorkflow::Launcher) {
                    app.complete_home_workflow_to_glyphs();
                }
                app.status = Some(format!(
                    "Created {}x{} grid for {} (left/right bleed: {}, top/bottom bleed: {})",
                    rows,
                    cols,
                    source_display_name(&source_key),
                    bleed_level_label(config.horizontal_bleed),
                    bleed_level_label(config.vertical_bleed)
                ));
            }
        }
        _ => {}
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct ClipboardProvider {
    command: &'static str,
    args: &'static [&'static str],
}

const LINUX_WAYLAND_CLIPBOARD_PROVIDERS: [ClipboardProvider; 1] = [ClipboardProvider {
    command: "wl-copy",
    args: &[],
}];
const LINUX_X11_CLIPBOARD_PROVIDERS: [ClipboardProvider; 2] = [
    ClipboardProvider {
        command: "xclip",
        args: &["-selection", "clipboard"],
    },
    ClipboardProvider {
        command: "wl-copy",
        args: &[],
    },
];
const MACOS_CLIPBOARD_PROVIDERS: [ClipboardProvider; 1] = [ClipboardProvider {
    command: "pbcopy",
    args: &[],
}];
const WINDOWS_CLIPBOARD_PROVIDERS: [ClipboardProvider; 2] = [
    ClipboardProvider {
        command: "powershell",
        args: &[
            "-NoProfile",
            "-Command",
            "Set-Clipboard -Value ([Console]::In.ReadToEnd())",
        ],
    },
    ClipboardProvider {
        command: "clip.exe",
        args: &[],
    },
];

fn clipboard_providers_for_current_platform() -> &'static [ClipboardProvider] {
    clipboard_providers_for_os(env::consts::OS, env::var_os("WAYLAND_DISPLAY").is_some())
}

fn clipboard_providers_for_os(
    os: &str,
    wayland_display_present: bool,
) -> &'static [ClipboardProvider] {
    match os {
        "windows" => &WINDOWS_CLIPBOARD_PROVIDERS,
        "macos" => &MACOS_CLIPBOARD_PROVIDERS,
        "linux" => {
            if wayland_display_present {
                &LINUX_WAYLAND_CLIPBOARD_PROVIDERS
            } else {
                &LINUX_X11_CLIPBOARD_PROVIDERS
            }
        }
        _ => &LINUX_X11_CLIPBOARD_PROVIDERS,
    }
}

fn execute_clipboard_provider(provider: &ClipboardProvider, text: &str) -> Result<()> {
    let resolved = resolve_command_path(provider.command)
        .ok_or_else(|| anyhow!("{} missing from PATH", provider.command))?;
    let mut command = std::process::Command::new(&resolved);
    command
        .args(provider.args)
        .stdin(std::process::Stdio::piped());
    let mut child = command
        .spawn()
        .with_context(|| format!("failed to spawn {}", provider.command))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(text.as_bytes())
            .with_context(|| format!("failed to write to {}", provider.command))?;
    }
    let status = child
        .wait()
        .with_context(|| format!("failed waiting for {}", provider.command))?;
    if !status.success() {
        bail!("{} exited with status {status}", provider.command);
    }
    Ok(())
}

fn resolve_command_path(command: &str) -> Option<PathBuf> {
    let candidate = PathBuf::from(command);
    if candidate.is_absolute() && candidate.is_file() {
        return Some(candidate);
    }

    let path_var = env::var_os("PATH")?;
    for dir in env::split_paths(&path_var) {
        if dir.as_os_str().is_empty() {
            continue;
        }
        let full = dir.join(&candidate);
        if full.is_file() {
            return Some(full);
        }
    }
    None
}

fn copy_to_clipboard_with_runner<F>(
    text: &str,
    providers: &[ClipboardProvider],
    mut run: F,
) -> Result<()>
where
    F: FnMut(&ClipboardProvider, &str) -> Result<()>,
{
    let mut attempts = Vec::new();
    let mut errors = Vec::new();
    for provider in providers {
        attempts.push(provider.command.to_string());
        match run(provider, text) {
            Ok(()) => return Ok(()),
            Err(err) => {
                errors.push(format!("{}: {err}", provider.command));
            }
        }
    }
    if !errors.is_empty() {
        bail!(
            "failed to copy to clipboard (tried: {}; errors: {})",
            attempts.join(", "),
            errors.join(" | ")
        );
    }
    bail!(
        "failed to copy to clipboard (tried: {})",
        attempts.join(", ")
    );
}

fn copy_to_clipboard(text: &str) -> Result<()> {
    if env::var_os(MOCK_CLIPBOARD_ENV).is_some() {
        return Ok(());
    }
    let providers = clipboard_providers_for_current_platform();
    copy_to_clipboard_with_runner(text, providers, execute_clipboard_provider)
}
