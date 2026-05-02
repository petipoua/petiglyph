use anyhow::{Context, Result, bail};
use crossterm::ExecutableCommand;
use crossterm::event::{
    self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyEventState, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
    PushKeyboardEnhancementFlags,
};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph, Tabs, Wrap,
};
use ratatui::{Frame, Terminal};
use std::collections::BTreeSet;
use std::env;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{
    OnceLock,
    mpsc::{self, Receiver, TryRecvError},
};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tui_input::{Input, backend::crossterm::EventHandler};
use walkdir::WalkDir;

use crate::artifact_warning::incompatible_artifact_warning;
use crate::build::{
    BuildSummary, MappingEntry, PreprocessedGlyph, build_outputs, expected_bdf_path,
    expected_ttf_path, is_supported_source, preprocess_sources,
};
use crate::install::{
    DEFAULT_INSTALL_NAME_MODE, FontInstallNameMode, effective_font_name,
    expected_install_ttf_path_for_mode, install_built_font, install_dir_for_manifest,
    installed_ttf_candidates_for_manifest_font, uninstall_installed_font_file,
};
use crate::project::{
    RuntimeConfig, create_project_in_dir, delete_project_for_manifest, discover_project_manifests,
    load_runtime_config, read_manifest, write_manifest,
};

const CLI_VERSION: &str = env!("CARGO_PKG_VERSION");
const INSTALL_SPINNER_FRAMES: [&str; 4] = ["-", "\\", "|", "/"];
const UNINSTALL_SPINNER_FRAMES: [&str; 4] = ["/", "|", "\\", "-"];
const FONT_TASK_SPINNER_FRAME_MS: u64 = 43;
const BUILD_TASK_MIN_VISIBLE_MS: u64 = 100;
const WELCOME_SAMPLE_LIMIT: usize = 15;
const WELCOME_INPUT_WIDTH: usize = 15;
const SWITCH_NOTICE_MS: u64 = 2500;
const EVENT_POLL_MS: u64 = 33;
const TUI_DEBUG_LOG_PATH: &str = "/tmp/petiglyph-tui-debug.log";
const TUI_MIN_WIDTH: u16 = 96;
const TUI_MIN_HEIGHT: u16 = 40;
const TUI_MAX_WIDTH: u16 = 148;
const TUI_MAX_HEIGHT: u16 = 46;
const DECPNM_NUMERIC_KEYPAD_MODE: &str = "\x1B>";
const WELCOME_HINT_WIDTH: usize = 27;
const DELETE_CONFIRM_CANCEL_INDEX: usize = 0;
const DELETE_CONFIRM_DELETE_INDEX: usize = 6;
const DELETE_CONFIRM_PATH: [(i8, i8); 7] = [(0, 0), (1, 0), (2, 0), (2, 1), (2, 2), (3, 2), (4, 2)];

#[derive(Debug, Clone, Default)]
pub(crate) struct TuiLaunchOverrides {
    pub(crate) input_dir: Option<PathBuf>,
    pub(crate) threshold: Option<u8>,
    pub(crate) glyph_size: Option<u32>,
    pub(crate) codepoint_start: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct WelcomeProject {
    manifest_path: PathBuf,
    font_name: String,
}

#[derive(Debug, Clone)]
pub(crate) struct InstalledFontSample {
    pub(crate) file_name: String,
    pub(crate) path: PathBuf,
    pub(crate) sample: String,
    pub(crate) truncated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WelcomeFocus {
    ProjectList,
    CreateInput,
    CreateButton,
    BuildButton,
    InstallButton,
    DeleteProjectButton,
    ToolList,
    InstalledFontList,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeleteProjectConfirmSlot {
    Cancel,
    Hop1,
    Hop2,
    Hop3,
    Hop4,
    Hop5,
    Delete,
}

impl DeleteProjectConfirmSlot {
    fn from_index(index: usize) -> Self {
        match index {
            0 => Self::Cancel,
            1 => Self::Hop1,
            2 => Self::Hop2,
            3 => Self::Hop3,
            4 => Self::Hop4,
            5 => Self::Hop5,
            _ => Self::Delete,
        }
    }
}

fn delete_confirm_neighbor_index(current_index: usize, dx: i8, dy: i8) -> Option<usize> {
    let &(x, y) = DELETE_CONFIRM_PATH.get(current_index)?;
    let target = (x + dx, y + dy);
    DELETE_CONFIRM_PATH
        .iter()
        .position(|&coord| coord == target)
}

#[derive(Debug, Clone)]
pub(crate) struct ProjectSwitchNotice {
    from_label: String,
    to_label: String,
    started_at: Instant,
}

pub(crate) fn tui(
    manifest_path: PathBuf,
    input_override: Option<PathBuf>,
    threshold_override: Option<u8>,
    glyph_size_override: Option<u32>,
    codepoint_start_override: Option<String>,
) -> Result<()> {
    let workspace_root = std::env::current_dir().context("failed to read current folder")?;
    tui_workspace(
        workspace_root,
        Some(manifest_path),
        input_override,
        threshold_override,
        glyph_size_override,
        codepoint_start_override,
    )
}

pub(crate) fn tui_workspace(
    workspace_root: PathBuf,
    initial_manifest: Option<PathBuf>,
    input_override: Option<PathBuf>,
    threshold_override: Option<u8>,
    glyph_size_override: Option<u32>,
    codepoint_start_override: Option<String>,
) -> Result<()> {
    let launch_overrides = TuiLaunchOverrides {
        input_dir: input_override,
        threshold: threshold_override,
        glyph_size: glyph_size_override,
        codepoint_start: codepoint_start_override,
    };

    let mut app = App::new_workspace(workspace_root, initial_manifest, launch_overrides)?;

    reset_tui_debug_log();
    tui_debug_log("tui.start", app_debug_state(&app));

    let mut session = TerminalSession::start()?;
    let mut log_next_draw_after_esc = false;
    while !app.quit {
        app.poll_build_task();
        app.poll_font_task();
        app.clear_expired_switch_notice();
        session.terminal.draw(|frame| draw_ui(frame, &app))?;
        if log_next_draw_after_esc {
            tui_debug_log("draw.after_esc", app_debug_state(&app));
            log_next_draw_after_esc = false;
        }

        if event::poll(Duration::from_millis(EVENT_POLL_MS))? {
            tui_debug_log("event.poll.ready", app_debug_state(&app));
            match event::read()? {
                Event::Key(key) => {
                    tui_debug_log("event.read.key", key_debug(&key));
                    if should_dispatch_key_kind(key.kind) {
                        tui_debug_log("event.dispatch.before", app_debug_state(&app));
                        if let Err(err) = handle_key_event(&mut app, key) {
                            app.status = Some(format_status_from_error(
                                &app.manifest_path,
                                &err.to_string(),
                            ));
                            tui_debug_log("event.dispatch.error", app_debug_state(&app));
                        } else {
                            tui_debug_log("event.dispatch.after", app_debug_state(&app));
                        }
                        if matches!(key.code, KeyCode::Esc) {
                            log_next_draw_after_esc = true;
                        }
                    } else {
                        tui_debug_log("event.key.ignored_non_press", key_debug(&key));
                    }
                }
                Event::Paste(payload) => {
                    tui_debug_log("event.read.paste", payload.replace('\n', "\\n"));
                    if let Err(err) = handle_paste_event(&mut app, &payload) {
                        app.status = Some(format_status_from_error(
                            &app.manifest_path,
                            &err.to_string(),
                        ));
                        tui_debug_log("event.dispatch.error", app_debug_state(&app));
                    } else {
                        tui_debug_log("event.dispatch.after", app_debug_state(&app));
                    }
                }
                other => {
                    tui_debug_log("event.read.non_key", format!("{other:?}"));
                }
            }
        }
    }

    let close_label = app.active_project_label();
    drop(session);

    println!("tui session closed for {close_label}");
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AppView {
    Welcome,
    Glyphs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HomeToolAction {
    ComposeGrid,
    AnimateGlyph,
}

impl HomeToolAction {
    fn description(self) -> &'static str {
        match self {
            Self::ComposeGrid => "Planned: build combined glyphs from tiled source layouts.",
            Self::AnimateGlyph => "Planned: build frame-based animated glyph assets.",
        }
    }
}

pub(crate) struct App {
    pub(crate) manifest_path: PathBuf,
    pub(crate) project_dir: PathBuf,
    pub(crate) config: RuntimeConfig,
    pub(crate) workspace_root: PathBuf,
    pub(crate) projects: Vec<WelcomeProject>,
    pub(crate) active_project: Option<PathBuf>,
    pub(crate) selected_project: usize,
    pub(crate) create_input: Input,
    pub(crate) welcome_focus: WelcomeFocus,
    pub(crate) welcome_input_editing: bool,
    pub(crate) installed_fonts: Vec<InstalledFontSample>,
    pub(crate) selected_installed_font: usize,
    pub(crate) switch_notice: Option<ProjectSwitchNotice>,
    pub(crate) selected_home_tool: HomeToolAction,
    pub(crate) selected: usize,
    pub(crate) glyphs: Vec<InteractiveGlyph>,
    pub(crate) quit: bool,
    pub(crate) status: Option<String>,
    pub(crate) view: AppView,
    pub(crate) last_build: Option<BuildSummary>,
    pub(crate) last_sample: Option<String>,
    pub(crate) installed_font_path: Option<PathBuf>,
    delete_project_confirm_selection: Option<usize>,
    launch_overrides: TuiLaunchOverrides,
    build_task: Option<BuildTask>,
    install_task: Option<InstallTask>,
}

#[derive(Debug, Clone)]
pub(crate) struct InteractiveGlyph {
    pub(crate) glyph: PreprocessedGlyph,
    pub(crate) saved_threshold: Option<u8>,
    pub(crate) working_threshold: u8,
}

struct TerminalSession {
    terminal: Terminal<CrosstermBackend<io::Stdout>>,
    keyboard_enhancements_enabled: bool,
    bracketed_paste_enabled: bool,
}

struct BuildTask {
    kind: BuildTaskKind,
    receiver: Receiver<Result<BuildTaskOutput, String>>,
    spinner_index: usize,
    spinner_last_frame_at: Instant,
    started_at: Instant,
    pending_result: Option<Result<BuildTaskOutput, String>>,
}

struct InstallTask {
    kind: FontTaskKind,
    receiver: Receiver<Result<InstallTaskOutput, String>>,
    spinner_index: usize,
    spinner_last_frame_at: Instant,
}

#[derive(Debug, Clone)]
struct BuildTaskOutput {
    summary: BuildSummary,
    sample: String,
}

#[derive(Debug, Clone)]
enum InstallTaskOutput {
    Install {
        summary: BuildSummary,
        sample: Option<String>,
        installed_path: PathBuf,
    },
    Uninstall {
        status_message: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BuildTaskKind {
    Build,
    Rebuild,
}

impl BuildTaskKind {
    fn button_label(&self) -> &'static str {
        match self {
            Self::Build => "Building...",
            Self::Rebuild => "Rebuilding...",
        }
    }

    fn footer_label(&self) -> &'static str {
        match self {
            Self::Build => "building project...",
            Self::Rebuild => "rebuilding project...",
        }
    }

    fn completion_verb(&self) -> &'static str {
        match self {
            Self::Build => "build",
            Self::Rebuild => "rebuild",
        }
    }
}

#[derive(Debug, Clone)]
enum FontTaskKind {
    Install,
    UninstallInstalled { path: PathBuf },
}

impl FontTaskKind {
    fn is_uninstall(&self) -> bool {
        !matches!(self, Self::Install)
    }

    fn spinner_frames(&self) -> &'static [&'static str] {
        if self.is_uninstall() {
            &UNINSTALL_SPINNER_FRAMES
        } else {
            &INSTALL_SPINNER_FRAMES
        }
    }

    fn spinner_frame_duration(&self) -> Duration {
        Duration::from_millis(FONT_TASK_SPINNER_FRAME_MS)
    }

    fn footer_label(&self) -> &'static str {
        if self.is_uninstall() {
            "removing font..."
        } else {
            "installing font..."
        }
    }

    fn progress_style(&self) -> Style {
        if self.is_uninstall() {
            Style::default()
                .fg(Color::White)
                .bg(Color::Red)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        }
    }
}

impl TerminalSession {
    fn start() -> Result<Self> {
        tui_debug_log("terminal.start.enable_raw_mode.before", "");
        enable_raw_mode().context("failed to enable raw mode")?;
        tui_debug_log("terminal.start.enable_raw_mode.after", "");
        let mut stdout = io::stdout();
        tui_debug_log("terminal.start.alternate_screen.before", "");
        stdout
            .execute(EnterAlternateScreen)
            .context("failed to enter alternate screen")?;
        tui_debug_log("terminal.start.alternate_screen.after", "");
        let keypad_mode_set = stdout
            .write_all(DECPNM_NUMERIC_KEYPAD_MODE.as_bytes())
            .is_ok();
        let _ = stdout.flush();
        tui_debug_log(
            "terminal.start.keypad_numeric_mode",
            format!("enabled={keypad_mode_set}"),
        );
        let keyboard_enhancements_enabled = stdout
            .execute(PushKeyboardEnhancementFlags(
                requested_keyboard_enhancement_flags(),
            ))
            .is_ok();
        tui_debug_log(
            "terminal.start.keyboard_enhancements",
            format!("enabled={keyboard_enhancements_enabled}"),
        );
        let bracketed_paste_enabled = stdout.execute(EnableBracketedPaste).is_ok();
        tui_debug_log(
            "terminal.start.bracketed_paste",
            format!("enabled={bracketed_paste_enabled}"),
        );

        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend).context("failed to initialize terminal UI")?;
        Ok(Self {
            terminal,
            keyboard_enhancements_enabled,
            bracketed_paste_enabled,
        })
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        if self.keyboard_enhancements_enabled {
            let _ = self
                .terminal
                .backend_mut()
                .execute(PopKeyboardEnhancementFlags);
        }
        if self.bracketed_paste_enabled {
            let _ = self.terminal.backend_mut().execute(DisableBracketedPaste);
        }
        let _ = disable_raw_mode();
        let _ = self.terminal.backend_mut().execute(LeaveAlternateScreen);
        let _ = self.terminal.show_cursor();
    }
}

fn scan_projects_in_folder(folder: &Path) -> Result<Vec<WelcomeProject>> {
    discover_project_manifests(folder)?
        .into_iter()
        .map(|manifest_path| {
            let manifest = read_manifest(&manifest_path).with_context(|| {
                format!(
                    "failed to read project manifest {}",
                    manifest_path.display()
                )
            })?;
            Ok(WelcomeProject {
                manifest_path,
                font_name: manifest.font_name,
            })
        })
        .collect()
}

fn scan_installed_petiglyph_fonts(cwd: &Path) -> Result<Vec<InstalledFontSample>> {
    let manifest_probe = cwd.join("petiglyph.toml");
    let install_dir = install_dir_for_manifest(&manifest_probe)?;
    if !install_dir.is_dir() {
        return Ok(Vec::new());
    }

    let mut ttf_paths = Vec::new();
    for entry in fs::read_dir(&install_dir)
        .with_context(|| format!("failed to read {}", install_dir.display()))?
    {
        let entry =
            entry.with_context(|| format!("failed to read entry in {}", install_dir.display()))?;
        let path = entry.path();
        let is_ttf = path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("ttf"))
            .unwrap_or(false);
        if path.is_file() && is_ttf {
            ttf_paths.push(path);
        }
    }
    ttf_paths.sort();

    let mut samples = Vec::new();
    for path in ttf_paths {
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("unknown.ttf")
            .to_string();

        let (sample, truncated) = fs::read(&path)
            .ok()
            .and_then(|bytes| sample_glyphs_from_ttf_bytes(&bytes, WELCOME_SAMPLE_LIMIT))
            .unwrap_or_default();

        samples.push(InstalledFontSample {
            file_name,
            path,
            sample,
            truncated,
        });
    }

    Ok(samples)
}

pub(crate) fn sample_glyphs_from_ttf_bytes(bytes: &[u8], limit: usize) -> Option<(String, bool)> {
    if limit == 0 {
        return None;
    }

    let face = ttf_parser::Face::parse(bytes, 0).ok()?;
    let cmap = face.tables().cmap?;
    let mut codepoints = BTreeSet::new();
    let mut truncated = false;

    for subtable in cmap.subtables {
        if !subtable.is_unicode() {
            continue;
        }

        subtable.codepoints(|codepoint| {
            if codepoint <= 0x20 || codepoint > 0x10_FFFF || (0xD800..=0xDFFF).contains(&codepoint)
            {
                return;
            }

            if codepoints.contains(&codepoint) {
                return;
            }

            if codepoints.len() < limit {
                codepoints.insert(codepoint);
            } else {
                truncated = true;
            }
        });

        if codepoints.len() >= limit && truncated {
            break;
        }
    }

    let sample = codepoints
        .into_iter()
        .filter_map(char::from_u32)
        .collect::<String>();
    if sample.is_empty() {
        None
    } else {
        Some((sample, truncated))
    }
}

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

pub(crate) fn build_action_name(project_is_built: bool) -> &'static str {
    if project_is_built { "Rebuild" } else { "Build" }
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
        (WelcomeFocus::CreateInput, false) => "press Enter to type",
        _ => "",
    };

    format!("  {hint:<WELCOME_HINT_WIDTH$}")
}

fn reset_tui_debug_log() {
    if !tui_debug_enabled() {
        return;
    }

    let now = debug_timestamp();
    let _ = fs::write(
        TUI_DEBUG_LOG_PATH,
        format!("[{now}] petiglyph TUI debug log reset\n"),
    );
}

fn tui_debug_log(event: &str, details: impl AsRef<str>) {
    if !tui_debug_enabled() {
        return;
    }

    let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(TUI_DEBUG_LOG_PATH)
    else {
        return;
    };

    let _ = writeln!(
        file,
        "[{}] {event}: {}",
        debug_timestamp(),
        details.as_ref()
    );
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
        "view={:?} focus={:?} selected_project={} editing={} input={:?} cursor={} visual_cursor={} build_task={} install_task={} delete_confirm_selection={:?} status={:?} quit={}",
        app.view,
        app.welcome_focus,
        app.selected_project,
        app.welcome_input_editing,
        app.create_input.value(),
        app.create_input.cursor(),
        app.create_input.visual_cursor(),
        app.build_task.is_some(),
        app.install_task.is_some(),
        app.delete_project_confirm_selection,
        app.status,
        app.quit
    )
}

fn handle_welcome_key(app: &mut App, key: KeyEvent) -> Result<()> {
    let code = key.code;
    tui_debug_log(
        "welcome.handle.enter",
        format!("{} {}", key_debug(&key), app_debug_state(app)),
    );
    if app.delete_project_confirm_selection.is_some() {
        return handle_delete_project_confirmation_key(app, code);
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
        KeyCode::Char('R') if !app.welcome_input_editing => {
            if app.build_in_progress() || app.install_in_progress() {
                app.status =
                    Some("a background task is in progress; wait before rescanning".to_string());
                return Ok(());
            }
            app.refresh_workspace_discovery()?;
            if app.active_project.is_some() {
                app.reload_glyphs()?;
            }
        }
        KeyCode::Char('b') if !app.welcome_input_editing => {
            trigger_build_action(app)?;
        }
        KeyCode::Char('i') if !app.welcome_input_editing => {
            trigger_install_action(app)?;
        }
        KeyCode::Up | KeyCode::Char('k') if !app.welcome_input_editing => {
            app.welcome_focus = match app.welcome_focus {
                WelcomeFocus::ProjectList => {
                    app.selected_project = app.selected_project.saturating_sub(1);
                    WelcomeFocus::ProjectList
                }
                WelcomeFocus::CreateInput if !app.projects.is_empty() => {
                    app.selected_project = app.projects.len() - 1;
                    WelcomeFocus::ProjectList
                }
                WelcomeFocus::CreateButton if !app.projects.is_empty() => {
                    app.selected_project = app.projects.len() - 1;
                    WelcomeFocus::ProjectList
                }
                WelcomeFocus::CreateInput => WelcomeFocus::CreateInput,
                WelcomeFocus::CreateButton => WelcomeFocus::CreateButton,
                WelcomeFocus::BuildButton => WelcomeFocus::CreateInput,
                WelcomeFocus::InstallButton => WelcomeFocus::CreateButton,
                WelcomeFocus::DeleteProjectButton => WelcomeFocus::InstallButton,
                WelcomeFocus::ToolList => match app.selected_home_tool {
                    HomeToolAction::ComposeGrid => WelcomeFocus::CreateInput,
                    HomeToolAction::AnimateGlyph => WelcomeFocus::CreateButton,
                },
                WelcomeFocus::InstalledFontList => {
                    if app.selected_installed_font > 0 {
                        app.selected_installed_font -= 1;
                        WelcomeFocus::InstalledFontList
                    } else if app.active_project.is_some() {
                        WelcomeFocus::BuildButton
                    } else if !app.projects.is_empty() {
                        WelcomeFocus::ProjectList
                    } else {
                        WelcomeFocus::CreateInput
                    }
                }
            };
        }
        KeyCode::Down | KeyCode::Char('j') if !app.welcome_input_editing => {
            app.welcome_focus = match app.welcome_focus {
                WelcomeFocus::ProjectList => {
                    if app.selected_project + 1 < app.projects.len() {
                        app.selected_project += 1;
                        WelcomeFocus::ProjectList
                    } else {
                        WelcomeFocus::CreateInput
                    }
                }
                WelcomeFocus::CreateInput => {
                    app.selected_home_tool = HomeToolAction::ComposeGrid;
                    WelcomeFocus::ToolList
                }
                WelcomeFocus::CreateButton => {
                    app.selected_home_tool = HomeToolAction::AnimateGlyph;
                    WelcomeFocus::ToolList
                }
                WelcomeFocus::BuildButton => {
                    if app.installed_fonts.is_empty() {
                        WelcomeFocus::BuildButton
                    } else {
                        WelcomeFocus::InstalledFontList
                    }
                }
                WelcomeFocus::InstallButton => {
                    if app.installed_fonts.is_empty() {
                        WelcomeFocus::InstallButton
                    } else {
                        WelcomeFocus::InstalledFontList
                    }
                }
                WelcomeFocus::DeleteProjectButton => {
                    if app.installed_fonts.is_empty() {
                        WelcomeFocus::DeleteProjectButton
                    } else {
                        WelcomeFocus::InstalledFontList
                    }
                }
                WelcomeFocus::ToolList => {
                    if app.installed_fonts.is_empty() {
                        WelcomeFocus::ToolList
                    } else {
                        WelcomeFocus::InstalledFontList
                    }
                }
                WelcomeFocus::InstalledFontList => {
                    if app.selected_installed_font + 1 < app.installed_fonts.len() {
                        app.selected_installed_font += 1;
                    }
                    WelcomeFocus::InstalledFontList
                }
            };
        }
        KeyCode::Left | KeyCode::Char('h') if !app.welcome_input_editing => {
            app.welcome_focus = match app.welcome_focus {
                WelcomeFocus::ToolList => match app.selected_home_tool {
                    HomeToolAction::ComposeGrid => WelcomeFocus::ToolList,
                    HomeToolAction::AnimateGlyph => {
                        app.selected_home_tool = HomeToolAction::ComposeGrid;
                        WelcomeFocus::ToolList
                    }
                },
                WelcomeFocus::CreateButton => WelcomeFocus::CreateInput,
                WelcomeFocus::BuildButton => {
                    app.selected_home_tool = HomeToolAction::AnimateGlyph;
                    WelcomeFocus::ToolList
                }
                WelcomeFocus::InstallButton => WelcomeFocus::BuildButton,
                WelcomeFocus::DeleteProjectButton => WelcomeFocus::InstallButton,
                WelcomeFocus::InstalledFontList => WelcomeFocus::InstalledFontList,
                WelcomeFocus::ProjectList => WelcomeFocus::ProjectList,
                WelcomeFocus::CreateInput => WelcomeFocus::CreateInput,
            };
        }
        KeyCode::Right | KeyCode::Char('l') if !app.welcome_input_editing => {
            app.welcome_focus = match app.welcome_focus {
                WelcomeFocus::CreateInput => WelcomeFocus::CreateButton,
                WelcomeFocus::ProjectList => WelcomeFocus::BuildButton,
                WelcomeFocus::CreateButton => WelcomeFocus::BuildButton,
                WelcomeFocus::BuildButton => WelcomeFocus::InstallButton,
                WelcomeFocus::InstallButton => {
                    if app.active_project_can_be_deleted() {
                        WelcomeFocus::DeleteProjectButton
                    } else {
                        WelcomeFocus::InstallButton
                    }
                }
                WelcomeFocus::DeleteProjectButton => WelcomeFocus::DeleteProjectButton,
                WelcomeFocus::ToolList => match app.selected_home_tool {
                    HomeToolAction::ComposeGrid => {
                        app.selected_home_tool = HomeToolAction::AnimateGlyph;
                        WelcomeFocus::ToolList
                    }
                    HomeToolAction::AnimateGlyph => WelcomeFocus::BuildButton,
                },
                WelcomeFocus::InstalledFontList => WelcomeFocus::InstalledFontList,
            };
        }
        KeyCode::Enter => match app.welcome_focus {
            WelcomeFocus::ProjectList => {
                app.welcome_input_editing = false;
                if let Some(project) = app.projects.get(app.selected_project) {
                    app.set_active_project(project.manifest_path.clone())?;
                }
            }
            WelcomeFocus::CreateInput => {
                if app.welcome_input_editing {
                    app.welcome_input_editing = false;
                    app.welcome_focus = WelcomeFocus::CreateButton;
                    app.status = None;
                } else {
                    app.welcome_input_editing = true;
                    app.status = None;
                }
            }
            WelcomeFocus::CreateButton => {
                app.welcome_input_editing = false;
                app.submit_create()?;
            }
            WelcomeFocus::BuildButton => {
                app.welcome_input_editing = false;
                trigger_build_action(app)?;
            }
            WelcomeFocus::InstallButton => {
                app.welcome_input_editing = false;
                trigger_install_action(app)?;
            }
            WelcomeFocus::DeleteProjectButton => {
                app.welcome_input_editing = false;
                app.begin_delete_project_confirmation()?;
            }
            WelcomeFocus::ToolList => {
                app.welcome_input_editing = false;
                trigger_home_tool_action(app)?;
            }
            WelcomeFocus::InstalledFontList => {
                app.welcome_input_editing = false;
                trigger_uninstall_action(app)?;
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
                if let Some(next) = delete_confirm_neighbor_index(*selection, -1, 0) {
                    *selection = next;
                }
            }
            app.status = None;
        }
        KeyCode::Right | KeyCode::Char('l') => {
            if let Some(selection) = app.delete_project_confirm_selection.as_mut() {
                if let Some(next) = delete_confirm_neighbor_index(*selection, 1, 0) {
                    *selection = next;
                }
            }
            app.status = None;
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if let Some(selection) = app.delete_project_confirm_selection.as_mut() {
                if let Some(next) = delete_confirm_neighbor_index(*selection, 0, -1) {
                    *selection = next;
                }
            }
            app.status = None;
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if let Some(selection) = app.delete_project_confirm_selection.as_mut() {
                if let Some(next) = delete_confirm_neighbor_index(*selection, 0, 1) {
                    *selection = next;
                }
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
            Some(_) => {
                app.status = Some("follow the arrow path with turns to reach Delete".to_string());
            }
            None => {}
        },
        _ => {}
    }
    tui_debug_log("welcome.delete_confirm.exit", app_debug_state(app));
    Ok(())
}

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
            Constraint::Length(21), // projects + current project
            Constraint::Min(0),     // installed petiglyph fonts
        ])
        .split(area);

    let tip_lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::raw("  "),
            Span::styled(
                "Local scan scope: petiglyph checks the current folder and one level below for local projects/builds.",
                Style::default().fg(muted),
            ),
        ]),
    ];
    frame.render_widget(Paragraph::new(tip_lines).wrap(Wrap { trim: true }), body[0]);

    let main = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(54), Constraint::Percentage(46)])
        .split(body[1]);

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
        for (idx, project) in app.projects.iter().enumerate() {
            let is_active = app
                .active_project
                .as_ref()
                .is_some_and(|active| active == &project.manifest_path);
            let is_selected =
                app.welcome_focus == WelcomeFocus::ProjectList && app.selected_project == idx;
            let marker = if is_active { "active" } else { "found " };
            let row_style = if is_selected {
                Style::default()
                    .fg(Color::Black)
                    .bg(accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            project_rows.push(Line::from(vec![
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
                Span::styled(
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
                ),
                Span::styled(
                    "  ",
                    if is_selected {
                        row_style
                    } else {
                        Style::default().fg(muted)
                    },
                ),
                Span::styled(
                    project.manifest_path.display().to_string(),
                    if is_selected {
                        row_style
                    } else {
                        Style::default().fg(muted)
                    },
                ),
            ]));
        }
    }
    let input_style = if app.welcome_focus == WelcomeFocus::CreateInput {
        Style::default()
            .fg(Color::Black)
            .bg(accent)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White).bg(Color::DarkGray)
    };
    let button_style = if app.welcome_focus == WelcomeFocus::CreateButton {
        Style::default()
            .fg(Color::Black)
            .bg(accent)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White).bg(Color::DarkGray)
    };
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
        app.welcome_input_editing,
        input_cursor,
        WELCOME_INPUT_WIDTH,
    );
    let mut new_project_line = vec![
        Span::raw("  "),
        Span::styled("New project: ", Style::default().fg(muted)),
        Span::styled(input_value, input_style),
        Span::raw(" "),
        Span::styled(" Create ", button_style),
    ];
    new_project_line.push(Span::styled(
        format_projects_card_hint_for_display(app.welcome_focus, app.welcome_input_editing),
        Style::default().fg(muted),
    ));
    let mut projects_footer_lines =
        vec![Line::from(""), Line::from(new_project_line), Line::from("")];

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
    let build_label = match (app.build_task_kind(), app.build_task_spinner_frame()) {
        (Some(kind), Some(spinner)) => format!(" {spinner} {} ", kind.button_label()),
        _ => format!(" {} ", build_action_name(app.current_project_is_built())),
    };
    let build_button_style = if app.active_project.is_none() {
        disabled_button_style
    } else if app.build_in_progress() {
        selected_button_style
    } else if app.install_in_progress() {
        disabled_button_style
    } else if app.welcome_focus == WelcomeFocus::BuildButton {
        selected_button_style
    } else {
        idle_button_style
    };
    let install_button_style =
        if app.active_project.is_none() && !app.install_in_progress() && !app.build_in_progress() {
            disabled_button_style
        } else if let Some(FontTaskKind::Install) = app.font_task_kind() {
            app.font_task_button_style()
                .unwrap_or(disabled_button_style)
        } else if app.install_in_progress() || app.build_in_progress() {
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
    let delete_button_style = if !app.active_project_can_be_deleted()
        || app.install_in_progress()
        || app.build_in_progress()
    {
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
    let compose_button_style = if app.active_project.is_none() {
        disabled_button_style
    } else if app.welcome_focus == WelcomeFocus::ToolList
        && app.selected_home_tool == HomeToolAction::ComposeGrid
    {
        selected_button_style
    } else {
        idle_button_style
    };
    let animate_button_style = if app.active_project.is_none() {
        disabled_button_style
    } else if app.welcome_focus == WelcomeFocus::ToolList
        && app.selected_home_tool == HomeToolAction::AnimateGlyph
    {
        selected_button_style
    } else {
        idle_button_style
    };
    let tools_hint = if app.active_project.is_some() {
        app.selected_home_tool.description().to_string()
    } else {
        "Select or create a project to enable generators.".to_string()
    };
    projects_footer_lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled("Advanced: ", Style::default().fg(muted)),
        Span::styled(" Compose Grid ", compose_button_style),
        Span::raw(" "),
        Span::styled(" Animate Glyph ", animate_button_style),
    ]));
    projects_footer_lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled(tools_hint, Style::default().fg(muted)),
    ]));

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
                Span::styled(format!("built: {}", ttf_path.display()), ok_style)
            } else {
                Span::styled("not built yet", unbuilt_style)
            };
            let bdf_status = if bdf_built {
                Span::styled(format!("built: {}", bdf_path.display()), ok_style)
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

    let current_project_inner = current_project_block.inner(main[1]);
    frame.render_widget(current_project_block, main[1]);

    let show_add_images_warning = tools_active && !ttf_built && !bdf_built && app.glyphs.is_empty();
    let mut current_project_lines = vec![
        Line::from(vec![
            Span::raw("  "),
            Span::styled(current_project_summary, hint_style),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::raw("  "),
            Span::styled("Outputs", section_style.add_modifier(Modifier::BOLD)),
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
            Constraint::Min(0),
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
            Span::styled(build_label, build_button_style),
            Span::raw(" "),
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
        frame.render_widget(
            Paragraph::new(drag_images_here_lines(
                current_project_sections[2].width,
                current_project_sections[2].height,
                accent,
            ))
            .wrap(Wrap { trim: false }),
            current_project_sections[2],
        );
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
                "Machine-wide inventory of petiglyph-managed fonts.",
                Style::default().fg(muted),
            ),
        ]),
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
    let mut selected_font_row = 0usize;
    if app.installed_fonts.is_empty() {
        font_rows.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(
                "No installed petiglyph TTF fonts found.",
                Style::default().fg(muted),
            ),
        ]));
    } else {
        let sample_wrap_width = usize::from(body[2].width.saturating_sub(8).max(16));
        for (idx, font) in app.installed_fonts.iter().enumerate() {
            if idx == app.selected_installed_font {
                selected_font_row = font_rows.len();
            }
            let sample = if font.sample.is_empty() {
                "[sample unavailable]".to_string()
            } else if font.truncated {
                format!("{}...", spaced_sample(&font.sample))
            } else {
                spaced_sample(&font.sample)
            };
            let uninstall_button_style = if app.is_selected_font_uninstall_in_progress(&font.path) {
                app.font_task_button_style()
                    .unwrap_or(disabled_button_style)
            } else if app.install_in_progress() || app.build_in_progress() {
                disabled_button_style
            } else if app.welcome_focus == WelcomeFocus::InstalledFontList
                && app.selected_installed_font == idx
            {
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
            font_rows.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    &font.file_name,
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled(uninstall_label, uninstall_button_style),
            ]));
            font_rows.push(Line::from(vec![
                Span::raw("    "),
                Span::styled("path: ", Style::default().fg(muted)),
                Span::raw(font.path.display().to_string()),
            ]));
            font_rows.push(Line::from(vec![
                Span::raw("    "),
                Span::styled("sample:", Style::default().fg(muted)),
            ]));
            for line in wrap_sample_for_display(&sample, sample_wrap_width) {
                font_rows.push(Line::from(vec![
                    Span::raw("    "),
                    Span::styled(
                        line,
                        Style::default().fg(accent).add_modifier(Modifier::BOLD),
                    ),
                ]));
            }
            font_rows.push(Line::from(""));
        }
    }
    let fonts_inner = fonts_block.inner(body[2]);
    frame.render_widget(fonts_block, body[2]);

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
        selected_font_row.min(font_rows.len().saturating_sub(1)),
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
        Paragraph::new(rendered_font_rows).wrap(Wrap { trim: false }),
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

impl App {
    fn current_project_is_built(&self) -> bool {
        self.active_project.is_some() && self.last_build.is_some()
    }

    fn current_project_is_installed(&self) -> bool {
        self.active_project.is_some() && self.installed_font_path.is_some()
    }

    #[cfg(test)]
    pub(crate) fn new(manifest_path: PathBuf, config: RuntimeConfig) -> Self {
        Self::new_with_overrides(manifest_path, config, TuiLaunchOverrides::default(), None)
    }

    pub(crate) fn new_workspace(
        workspace_root: PathBuf,
        initial_manifest: Option<PathBuf>,
        launch_overrides: TuiLaunchOverrides,
    ) -> Result<Self> {
        let mut app = match initial_manifest {
            Some(manifest_path) => {
                let config = load_runtime_config(
                    &manifest_path,
                    launch_overrides.input_dir.clone(),
                    None,
                    launch_overrides.threshold,
                    launch_overrides.glyph_size,
                    launch_overrides.codepoint_start.clone(),
                )?;
                Self::new_with_overrides(
                    manifest_path,
                    config,
                    launch_overrides,
                    Some(workspace_root),
                )
            }
            None => Self::new_inactive(workspace_root, launch_overrides),
        };

        app.refresh_workspace_discovery()?;
        if app.active_project.is_some() {
            app.reload_glyphs()?;
        }
        Ok(app)
    }

    fn new_inactive(workspace_root: PathBuf, launch_overrides: TuiLaunchOverrides) -> Self {
        let manifest_path = workspace_root.join("petiglyph.toml");
        Self {
            manifest_path,
            project_dir: workspace_root.clone(),
            config: inactive_runtime_config(&workspace_root),
            workspace_root,
            projects: Vec::new(),
            active_project: None,
            selected_project: 0,
            create_input: Input::default(),
            welcome_focus: WelcomeFocus::CreateInput,
            welcome_input_editing: false,
            installed_fonts: Vec::new(),
            selected_installed_font: 0,
            switch_notice: None,
            selected_home_tool: HomeToolAction::ComposeGrid,
            selected: 0,
            glyphs: Vec::new(),
            quit: false,
            status: None,
            view: AppView::Welcome,
            last_build: None,
            last_sample: None,
            installed_font_path: None,
            delete_project_confirm_selection: None,
            launch_overrides,
            build_task: None,
            install_task: None,
        }
    }

    pub(crate) fn new_with_overrides(
        manifest_path: PathBuf,
        config: RuntimeConfig,
        launch_overrides: TuiLaunchOverrides,
        workspace_root: Option<PathBuf>,
    ) -> Self {
        let project_dir = manifest_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        let workspace_root = workspace_root.unwrap_or_else(|| project_dir.clone());
        let (last_build, last_sample) = cached_build_state(&config);
        let installed_font_path = cached_installed_font_path(&manifest_path, &config.font_name);
        Self {
            manifest_path: manifest_path.clone(),
            project_dir,
            config,
            workspace_root,
            projects: Vec::new(),
            active_project: Some(manifest_path),
            selected_project: 0,
            create_input: Input::default(),
            welcome_focus: WelcomeFocus::CreateInput,
            welcome_input_editing: false,
            installed_fonts: Vec::new(),
            selected_installed_font: 0,
            switch_notice: None,
            selected_home_tool: HomeToolAction::ComposeGrid,
            selected: 0,
            glyphs: Vec::new(),
            quit: false,
            status: None,
            view: AppView::Welcome,
            last_build,
            last_sample,
            installed_font_path,
            delete_project_confirm_selection: None,
            launch_overrides,
            build_task: None,
            install_task: None,
        }
    }

    fn refresh_workspace_discovery(&mut self) -> Result<()> {
        self.projects = scan_projects_in_folder(&self.workspace_root)?;
        self.sync_selected_project();

        match scan_installed_petiglyph_fonts(&self.workspace_root) {
            Ok(fonts) => self.installed_fonts = fonts,
            Err(err) => {
                self.installed_fonts.clear();
                self.status = Some(format!("font scan warning: {err}"));
            }
        }
        self.sync_selected_installed_font();

        if self.projects.is_empty() {
            self.welcome_focus = WelcomeFocus::CreateInput;
            self.welcome_input_editing = false;
            if self.active_project.is_none() {
                self.status = Some(format!(
                    "no petiglyph project in {}",
                    self.workspace_root.display()
                ));
            }
        } else if self.active_project.is_none() && self.welcome_focus == WelcomeFocus::CreateInput {
            self.welcome_focus = WelcomeFocus::ProjectList;
        }

        if self.welcome_focus == WelcomeFocus::InstalledFontList && self.installed_fonts.is_empty()
        {
            self.welcome_focus = if self.active_project.is_some() {
                WelcomeFocus::BuildButton
            } else if !self.projects.is_empty() {
                WelcomeFocus::ProjectList
            } else {
                WelcomeFocus::CreateInput
            };
        }

        if self.welcome_focus == WelcomeFocus::DeleteProjectButton
            && !self.active_project_can_be_deleted()
        {
            self.welcome_focus = if self.active_project.is_some() {
                WelcomeFocus::InstallButton
            } else if !self.projects.is_empty() {
                WelcomeFocus::ProjectList
            } else {
                WelcomeFocus::CreateInput
            };
        }

        Ok(())
    }

    fn sync_selected_project(&mut self) {
        if self.projects.is_empty() {
            self.selected_project = 0;
            return;
        }

        if let Some(active_project) = &self.active_project
            && let Some(idx) = self
                .projects
                .iter()
                .position(|project| &project.manifest_path == active_project)
        {
            self.selected_project = idx;
            return;
        }

        self.selected_project = self.selected_project.min(self.projects.len() - 1);
    }

    fn sync_selected_installed_font(&mut self) {
        if self.installed_fonts.is_empty() {
            self.selected_installed_font = 0;
            return;
        }

        self.selected_installed_font = self
            .selected_installed_font
            .min(self.installed_fonts.len() - 1);
    }

    fn active_project_can_be_deleted(&self) -> bool {
        let Some(active_manifest) = &self.active_project else {
            return false;
        };
        let Some(project_dir) = active_manifest.parent() else {
            return false;
        };

        if project_dir == self.workspace_root {
            return false;
        }
        if !project_dir.starts_with(&self.workspace_root) {
            return false;
        }

        self.projects
            .iter()
            .any(|project| project.manifest_path == *active_manifest)
    }

    fn cancel_delete_project_confirmation(&mut self) {
        self.delete_project_confirm_selection = None;
        self.status = Some("project deletion canceled".to_string());
    }

    fn begin_delete_project_confirmation(&mut self) -> Result<()> {
        if self.install_in_progress() || self.build_in_progress() {
            self.status = Some(
                "a background task is in progress; wait before deleting a project".to_string(),
            );
            return Ok(());
        }
        if !self.active_project_can_be_deleted() {
            self.status =
                Some("only nested workspace projects can be deleted from Home".to_string());
            return Ok(());
        }
        self.welcome_input_editing = false;
        self.delete_project_confirm_selection = Some(DELETE_CONFIRM_CANCEL_INDEX);
        self.status = None;
        Ok(())
    }

    fn confirm_delete_project(&mut self) -> Result<()> {
        let Some(active_manifest) = self.active_project.clone() else {
            self.status = Some("no active project selected".to_string());
            self.delete_project_confirm_selection = None;
            return Ok(());
        };

        if !self.active_project_can_be_deleted() {
            self.status = Some("active project is not deletable from this workspace".to_string());
            self.delete_project_confirm_selection = None;
            return Ok(());
        }

        let deleted_project_name = active_manifest
            .parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .unwrap_or("project")
            .to_string();

        delete_project_for_manifest(&active_manifest)?;
        self.delete_project_confirm_selection = None;
        self.active_project = None;
        self.manifest_path = self.workspace_root.join("petiglyph.toml");
        self.project_dir = self.workspace_root.clone();
        self.reload_config()?;
        self.glyphs.clear();
        self.selected = 0;
        self.refresh_workspace_discovery()?;
        self.welcome_focus = if self.projects.is_empty() {
            WelcomeFocus::CreateInput
        } else {
            WelcomeFocus::ProjectList
        };
        self.status = Some(format!("deleted project `{deleted_project_name}`"));
        Ok(())
    }

    fn submit_create(&mut self) -> Result<()> {
        let project_name = self.create_input.value().trim().to_string();
        if project_name.is_empty() {
            self.status = Some("project name cannot be empty".to_string());
            self.welcome_focus = WelcomeFocus::CreateInput;
            self.welcome_input_editing = true;
            return Ok(());
        }

        if self.install_in_progress() || self.build_in_progress() {
            self.status = Some(
                "a background task is in progress; wait before switching projects".to_string(),
            );
            return Ok(());
        }

        let manifest_path = create_project_in_dir(&self.workspace_root, &project_name)?;
        self.create_input = Input::default();
        self.welcome_input_editing = false;
        self.refresh_workspace_discovery()?;
        self.set_active_project(manifest_path)?;
        self.status = Some(format!("created and opened project `{project_name}`"));
        Ok(())
    }

    fn set_active_project(&mut self, manifest_path: PathBuf) -> Result<()> {
        if self.install_in_progress() || self.build_in_progress() {
            self.status = Some(
                "a background task is in progress; wait before switching projects".to_string(),
            );
            return Ok(());
        }

        let old_manifest = self.active_project.clone();
        let old_label = self.active_project_label();
        let changed = old_manifest.as_ref() != Some(&manifest_path);

        self.manifest_path = manifest_path.clone();
        self.project_dir = manifest_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        self.active_project = Some(manifest_path);
        self.reload_config()?;
        self.reload_glyphs()?;
        self.refresh_workspace_discovery()?;

        if changed {
            self.switch_notice = Some(ProjectSwitchNotice {
                from_label: old_label,
                to_label: self.active_project_label(),
                started_at: Instant::now(),
            });
        }

        Ok(())
    }

    fn reload_config(&mut self) -> Result<()> {
        if self.active_project.is_none() {
            self.config = inactive_runtime_config(&self.workspace_root);
            self.last_build = None;
            self.last_sample = None;
            self.installed_font_path = None;
            return Ok(());
        }

        self.config = load_runtime_config(
            &self.manifest_path,
            self.launch_overrides.input_dir.clone(),
            None,
            self.launch_overrides.threshold,
            self.launch_overrides.glyph_size,
            self.launch_overrides.codepoint_start.clone(),
        )?;
        let (last_build, last_sample) = cached_build_state(&self.config);
        self.last_build = last_build;
        self.last_sample = last_sample;
        self.installed_font_path =
            cached_installed_font_path(&self.manifest_path, &self.config.font_name);
        Ok(())
    }

    fn reload_glyphs(&mut self) -> Result<()> {
        if self.active_project.is_none() {
            self.glyphs.clear();
            self.selected = 0;
            self.status = Some("create a project in Home or relaunch with --manifest".to_string());
            return Ok(());
        }

        self.reload_config()?;

        if !self.config.input_dir.exists() {
            self.glyphs.clear();
            self.selected = 0;
            self.status = Some(format!(
                "icons directory not found yet: {}",
                self.config.input_dir.display()
            ));
            return Ok(());
        }

        let mut sources = Vec::new();
        for entry in WalkDir::new(&self.config.input_dir).follow_links(true) {
            let entry = entry.with_context(|| {
                format!("failed while scanning {}", self.config.input_dir.display())
            })?;
            if entry.file_type().is_file() && is_supported_source(entry.path()) {
                sources.push(entry.path().to_path_buf());
            }
        }
        sources.sort();

        if sources.is_empty() {
            self.glyphs.clear();
            self.selected = 0;
            self.status = Some(format!(
                "add or drag image files into {}",
                self.config.input_dir.display()
            ));
            return Ok(());
        }

        let glyphs = preprocess_sources(&sources, &self.config.input_dir, self.config.glyph_size)?
            .into_iter()
            .map(|glyph| {
                let saved_threshold = self
                    .config
                    .threshold_overrides
                    .get(&glyph.source_key)
                    .copied();
                let working_threshold = saved_threshold.unwrap_or(self.config.base_threshold);
                InteractiveGlyph {
                    glyph,
                    saved_threshold,
                    working_threshold,
                }
            })
            .collect::<Vec<_>>();

        self.glyphs = glyphs;
        self.selected = self.selected.min(self.glyphs.len().saturating_sub(1));
        self.status = Some(format!(
            "loaded {} glyph{} from {}",
            self.glyphs.len(),
            if self.glyphs.len() == 1 { "" } else { "s" },
            self.config.input_dir.display()
        ));
        Ok(())
    }

    fn import_dropped_images(&mut self, payload: &str) -> Result<()> {
        if self.build_in_progress() || self.install_in_progress() {
            self.status =
                Some("a background task is in progress; wait before importing images".to_string());
            return Ok(());
        }

        if self.active_project.is_none() {
            self.status =
                Some("create or select a project before importing dropped images".to_string());
            return Ok(());
        }

        self.reload_config()?;
        fs::create_dir_all(&self.config.input_dir)
            .with_context(|| format!("failed to create {}", self.config.input_dir.display()))?;

        let dropped_paths = collect_dropped_paths(payload);
        if dropped_paths.is_empty() {
            self.status = Some("drop did not include readable file paths".to_string());
            return Ok(());
        }

        let mut imported = 0usize;
        let mut renamed = 0usize;
        let mut skipped_existing = 0usize;
        let mut skipped_unsupported = 0usize;
        let mut skipped_missing = 0usize;

        for source in dropped_paths {
            if !source.is_file() {
                skipped_missing += 1;
                continue;
            }

            if !is_supported_source(&source) {
                skipped_unsupported += 1;
                continue;
            }

            let Some(file_name) = source.file_name() else {
                skipped_missing += 1;
                continue;
            };

            let canonical_destination = self.config.input_dir.join(file_name);
            if paths_resolve_to_same_file(&source, &canonical_destination) {
                skipped_existing += 1;
                continue;
            }

            let (destination, was_renamed) =
                next_available_import_destination(&self.config.input_dir, file_name);
            fs::copy(&source, &destination).with_context(|| {
                format!(
                    "failed to import {} into {}",
                    source.display(),
                    destination.display()
                )
            })?;

            imported += 1;
            if was_renamed {
                renamed += 1;
            }
        }

        if imported > 0 {
            self.reload_glyphs()?;
            if self.view == AppView::Welcome {
                self.welcome_input_editing = false;
                self.view = AppView::Glyphs;
            }
        }

        self.status = Some(format_drop_import_status(
            imported,
            renamed,
            skipped_existing,
            skipped_unsupported,
            skipped_missing,
        ));
        Ok(())
    }

    fn start_build_project(&mut self) -> Result<()> {
        if self.active_project.is_none() {
            self.status = Some(
                "create a project in Home or relaunch with --manifest before building".to_string(),
            );
            return Ok(());
        }

        if self.build_in_progress() {
            self.status = Some("build already in progress".to_string());
            return Ok(());
        }

        self.reload_config()?;
        if self.config.glyph_size == 0 {
            bail!("glyph_size must be > 0");
        }

        let rebuilding = self.current_project_is_built();

        let kind = if rebuilding {
            BuildTaskKind::Rebuild
        } else {
            BuildTaskKind::Build
        };
        let manifest_path = self.manifest_path.clone();
        let launch_overrides = self.launch_overrides.clone();
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let result =
                build_project_task(manifest_path, launch_overrides).map_err(|err| err.to_string());
            let _ = sender.send(result);
        });

        let now = Instant::now();
        self.build_task = Some(BuildTask {
            kind,
            receiver,
            spinner_index: 0,
            spinner_last_frame_at: now,
            started_at: now,
            pending_result: None,
        });
        self.status = None;
        Ok(())
    }

    fn start_install_font(&mut self) {
        if self.active_project.is_none() {
            self.status = Some(
                "create a project in Home or relaunch with --manifest before installing"
                    .to_string(),
            );
            return;
        }

        if self.build_in_progress() {
            self.status = Some("build is in progress; wait before installing".to_string());
            return;
        }

        if self.install_task.is_some() {
            self.status = Some("font operation already in progress".to_string());
            return;
        }

        let manifest_path = self.manifest_path.clone();
        let launch_overrides = self.launch_overrides.clone();
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let result =
                build_and_install(manifest_path, launch_overrides).map_err(|err| err.to_string());
            let _ = sender.send(result);
        });

        self.install_task = Some(InstallTask {
            kind: FontTaskKind::Install,
            receiver,
            spinner_index: 0,
            spinner_last_frame_at: Instant::now(),
        });
        self.status = None;
    }

    fn start_uninstall_selected_installed_font(&mut self) -> Result<()> {
        if self.build_in_progress() {
            self.status = Some("build is in progress; wait before uninstalling".to_string());
            return Ok(());
        }

        if self.install_in_progress() {
            self.status =
                Some("font operation is in progress; wait before uninstalling".to_string());
            return Ok(());
        }

        let Some(font) = self
            .installed_fonts
            .get(self.selected_installed_font)
            .cloned()
        else {
            self.status = Some("no installed font selected".to_string());
            return Ok(());
        };

        let target_path = font.path.clone();
        let file_name = font.file_name.clone();
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let result = uninstall_installed_font_task(target_path.clone(), file_name)
                .map_err(|err| err.to_string());
            let _ = sender.send(result);
        });

        self.install_task = Some(InstallTask {
            kind: FontTaskKind::UninstallInstalled { path: font.path },
            receiver,
            spinner_index: 0,
            spinner_last_frame_at: Instant::now(),
        });
        self.status = None;
        Ok(())
    }

    fn poll_build_task(&mut self) {
        let mut disconnected = false;
        let mut completed_result = None;

        if let Some(task) = self.build_task.as_mut() {
            let frame_duration = Duration::from_millis(FONT_TASK_SPINNER_FRAME_MS);
            let now = Instant::now();
            while now.duration_since(task.spinner_last_frame_at) >= frame_duration {
                task.spinner_index = (task.spinner_index + 1) % INSTALL_SPINNER_FRAMES.len();
                task.spinner_last_frame_at += frame_duration;
            }

            if task.pending_result.is_none() {
                match task.receiver.try_recv() {
                    Ok(result) => task.pending_result = Some(result),
                    Err(TryRecvError::Empty) => {}
                    Err(TryRecvError::Disconnected) => disconnected = true,
                }
            }

            if now.duration_since(task.started_at)
                >= Duration::from_millis(BUILD_TASK_MIN_VISIBLE_MS)
            {
                completed_result = task.pending_result.take();
            }
        }

        if disconnected {
            self.build_task = None;
            self.status = Some("build task terminated unexpectedly".to_string());
            return;
        }

        let Some(result) = completed_result else {
            return;
        };

        let kind = self
            .build_task
            .as_ref()
            .map(|task| task.kind)
            .unwrap_or(BuildTaskKind::Build);
        self.build_task = None;

        match result {
            Ok(BuildTaskOutput { summary, sample }) => {
                self.last_sample = Some(sample);
                self.last_build = Some(summary.clone());
                self.status = Some(format!(
                    "{} complete: {} glyph{} into {}",
                    kind.completion_verb(),
                    summary.glyph_count,
                    if summary.glyph_count == 1 { "" } else { "s" },
                    summary.out_dir().display()
                ));
            }
            Err(err) => {
                self.status = Some(format_status_from_error(&self.manifest_path, &err));
                let _ = self.reload_config();
            }
        }
    }

    fn poll_font_task(&mut self) {
        let mut task_result = None;
        let mut disconnected = false;

        if let Some(task) = self.install_task.as_mut() {
            let frame_duration = task.kind.spinner_frame_duration();
            let now = Instant::now();
            while now.duration_since(task.spinner_last_frame_at) >= frame_duration {
                task.spinner_index = (task.spinner_index + 1) % task.kind.spinner_frames().len();
                task.spinner_last_frame_at += frame_duration;
            }
            match task.receiver.try_recv() {
                Ok(result) => task_result = Some(result),
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => disconnected = true,
            }
        }

        if disconnected {
            let operation = self
                .install_task
                .as_ref()
                .map(|task| {
                    if task.kind.is_uninstall() {
                        "uninstall"
                    } else {
                        "install"
                    }
                })
                .unwrap_or("font");
            self.install_task = None;
            self.status = Some(format!("{operation} task terminated unexpectedly"));
            return;
        }

        let Some(result) = task_result else {
            return;
        };

        self.install_task = None;
        match result {
            Ok(InstallTaskOutput::Install {
                summary,
                sample,
                installed_path,
            }) => {
                self.last_build = Some(summary);
                self.last_sample = sample;
                self.installed_font_path = Some(installed_path.clone());
                if let Err(err) = self.refresh_workspace_discovery() {
                    self.status = Some(format!(
                        "installed font to {}; refresh failed: {err}",
                        installed_path.display()
                    ));
                } else {
                    if let Some(idx) = self
                        .installed_fonts
                        .iter()
                        .position(|font| font.path == installed_path)
                    {
                        self.selected_installed_font = idx;
                    }
                    self.status = Some(format!("installed font to {}", installed_path.display()));
                }
            }
            Ok(InstallTaskOutput::Uninstall { status_message }) => {
                if let Err(err) = self.refresh_workspace_discovery() {
                    self.status = Some(format!("{status_message}; refresh failed: {err}"));
                } else if self.active_project.is_some() {
                    if let Err(err) = self.reload_config() {
                        self.status = Some(format!("{status_message}; reload failed: {err}"));
                    } else {
                        self.status = Some(status_message);
                    }
                } else {
                    self.status = Some(status_message);
                }
            }
            Err(err) => {
                self.status = Some(format_status_from_error(&self.manifest_path, &err));
                let _ = self.reload_config();
            }
        }
    }

    fn build_task_kind(&self) -> Option<BuildTaskKind> {
        self.build_task.as_ref().map(|task| task.kind)
    }

    fn build_task_spinner_frame(&self) -> Option<&'static str> {
        self.build_task
            .as_ref()
            .map(|task| INSTALL_SPINNER_FRAMES[task.spinner_index % INSTALL_SPINNER_FRAMES.len()])
    }

    fn font_task_kind(&self) -> Option<&FontTaskKind> {
        self.install_task.as_ref().map(|task| &task.kind)
    }

    fn font_task_spinner_frame(&self) -> Option<&'static str> {
        self.install_task.as_ref().map(|task| {
            let frames = task.kind.spinner_frames();
            frames[task.spinner_index % frames.len()]
        })
    }

    fn font_task_button_style(&self) -> Option<Style> {
        self.font_task_kind().map(FontTaskKind::progress_style)
    }

    fn is_selected_font_uninstall_in_progress(&self, font_path: &Path) -> bool {
        matches!(
            self.font_task_kind(),
            Some(FontTaskKind::UninstallInstalled { path }) if path == font_path
        )
    }

    fn install_in_progress(&self) -> bool {
        self.install_task.is_some()
    }

    fn build_in_progress(&self) -> bool {
        self.build_task.is_some()
    }

    #[cfg(test)]
    pub(crate) fn background_task_in_progress(&self) -> bool {
        self.build_in_progress() || self.install_in_progress()
    }

    #[cfg(test)]
    pub(crate) fn poll_background_tasks_for_test(&mut self) {
        self.poll_build_task();
        self.poll_font_task();
    }

    fn active_project_label(&self) -> String {
        let Some(active_project) = &self.active_project else {
            return "none".to_string();
        };

        let folder = active_project
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .display();
        format!("{} ({folder})", self.config.font_name)
    }

    fn clear_expired_switch_notice(&mut self) {
        if self
            .switch_notice
            .as_ref()
            .is_some_and(|notice| !switch_notice_visible(notice.started_at, Instant::now()))
        {
            self.switch_notice = None;
        }
    }
}

impl BuildSummary {
    fn out_dir(&self) -> &Path {
        self.ttf_path.parent().unwrap_or_else(|| Path::new("."))
    }
}

fn inactive_runtime_config(workspace_root: &Path) -> RuntimeConfig {
    RuntimeConfig {
        project_dir: workspace_root.to_path_buf(),
        project_id: "inactive-workspace".to_string(),
        input_dir: workspace_root.join("icons"),
        out_dir: workspace_root.join("build"),
        font_name: "No active project".to_string(),
        glyph_size: 64,
        base_threshold: 64,
        threshold_overrides: Default::default(),
        codepoint_start: 0x10_0000,
    }
}

pub(crate) fn switch_notice_visible(started_at: Instant, now: Instant) -> bool {
    now.duration_since(started_at) < Duration::from_millis(SWITCH_NOTICE_MS)
}

fn build_project_task(
    manifest_path: PathBuf,
    launch_overrides: TuiLaunchOverrides,
) -> Result<BuildTaskOutput> {
    let config = load_runtime_config(
        &manifest_path,
        launch_overrides.input_dir,
        None,
        launch_overrides.threshold,
        launch_overrides.glyph_size,
        launch_overrides.codepoint_start,
    )?;
    if config.glyph_size == 0 {
        bail!("glyph_size must be > 0");
    }

    let summary = build_outputs(&config)?;
    let sample = fs::read_to_string(&summary.sample_path)
        .with_context(|| format!("failed to read {}", summary.sample_path.display()))?;

    Ok(BuildTaskOutput {
        summary,
        sample: sample.trim_end().to_string(),
    })
}

fn build_and_install(
    manifest_path: PathBuf,
    launch_overrides: TuiLaunchOverrides,
) -> Result<InstallTaskOutput> {
    let config = load_runtime_config(
        &manifest_path,
        launch_overrides.input_dir,
        None,
        launch_overrides.threshold,
        launch_overrides.glyph_size,
        launch_overrides.codepoint_start,
    )?;
    if config.glyph_size == 0 {
        bail!("glyph_size must be > 0");
    }
    let install_font_name =
        effective_font_name(&manifest_path, &config.font_name, DEFAULT_INSTALL_NAME_MODE)?;

    let summary = build_outputs(&config)?;
    let sample = fs::read_to_string(&summary.sample_path)
        .with_context(|| format!("failed to read {}", summary.sample_path.display()))?;
    let installed = install_built_font(
        &manifest_path,
        &install_font_name,
        &config.project_id,
        &summary.ttf_path,
        summary.glyph_count,
    )?;
    let sample = sample.trim_end().to_string();
    let sample = if sample.is_empty() {
        None
    } else {
        Some(sample)
    };

    Ok(InstallTaskOutput::Install {
        summary,
        sample,
        installed_path: installed.install_path,
    })
}

fn uninstall_installed_font_task(
    installed_ttf: PathBuf,
    file_name: String,
) -> Result<InstallTaskOutput> {
    let result = uninstall_installed_font_file(&installed_ttf)?;
    let status_message = match result.outcome {
        crate::install::UninstallOutcome::Removed => format!("uninstalled {file_name}"),
        crate::install::UninstallOutcome::AlreadyAbsent => {
            format!("font already absent: {file_name}")
        }
    };
    Ok(InstallTaskOutput::Uninstall { status_message })
}

fn cached_build_state(config: &RuntimeConfig) -> (Option<BuildSummary>, Option<String>) {
    let ttf_path = expected_ttf_path(config);
    let bdf_path = expected_bdf_path(config);
    if !ttf_path.is_file() || !bdf_path.is_file() {
        return (None, None);
    }

    let mapping_path = config.out_dir.join("glyph-map.json");
    let sample_path = config.out_dir.join("glyph-sample.txt");
    let previews_dir = config.out_dir.join("previews");

    let glyph_count = fs::read_to_string(&mapping_path)
        .ok()
        .and_then(|raw| serde_json::from_str::<Vec<MappingEntry>>(&raw).ok())
        .map_or(0, |entries| entries.len());

    let sample = fs::read_to_string(&sample_path)
        .ok()
        .map(|raw| raw.trim_end().to_string())
        .filter(|value| !value.is_empty());

    (
        Some(BuildSummary {
            glyph_count,
            bdf_path,
            ttf_path,
            mapping_path,
            sample_path,
            previews_dir,
        }),
        sample,
    )
}

fn cached_installed_font_path(manifest_path: &Path, font_name: &str) -> Option<PathBuf> {
    resolve_installed_font_path_with(manifest_path, font_name, |path| path.is_file())
}

pub(crate) fn resolve_installed_font_path_with<F>(
    manifest_path: &Path,
    font_name: &str,
    mut is_installed: F,
) -> Option<PathBuf>
where
    F: FnMut(&Path) -> bool,
{
    let mut candidates = Vec::new();
    if let Ok(paths) = installed_ttf_candidates_for_manifest_font(manifest_path, font_name) {
        for path in paths {
            if !candidates.contains(&path) {
                candidates.push(path);
            }
        }
    }
    if let Ok(path) =
        expected_install_ttf_path_for_mode(manifest_path, font_name, DEFAULT_INSTALL_NAME_MODE)
        && !candidates.contains(&path)
    {
        candidates.push(path);
    }
    if let Ok(path) =
        expected_install_ttf_path_for_mode(manifest_path, font_name, FontInstallNameMode::Plain)
        && !candidates.contains(&path)
    {
        candidates.push(path);
    }

    candidates.into_iter().find(|path| is_installed(path))
}

pub(crate) fn persist_threshold_override(
    manifest_path: &Path,
    source_key: &str,
    threshold: Option<u8>,
) -> Result<()> {
    let mut manifest = read_manifest(manifest_path)?;
    match threshold {
        Some(value) => {
            manifest
                .threshold_overrides
                .insert(source_key.to_string(), value);
        }
        None => {
            manifest.threshold_overrides.remove(source_key);
        }
    }
    write_manifest(manifest_path, &manifest)
}

fn selected_glyph_mut(app: &mut App) -> Option<&mut InteractiveGlyph> {
    app.glyphs.get_mut(app.selected)
}

fn selected_glyph(app: &App) -> Option<&InteractiveGlyph> {
    app.glyphs.get(app.selected)
}

fn set_selected_threshold(app: &mut App, threshold: u8) {
    if app.active_project.is_none() {
        app.status = Some(
            "create a project in Home or relaunch with --manifest before tuning glyphs".to_string(),
        );
        return;
    }

    let Some(glyph) = selected_glyph(app) else {
        app.status = Some("no glyph selected".to_string());
        return;
    };

    let source_key = glyph.glyph.source_key.clone();
    let glyph_name = glyph.glyph.glyph_name.clone();
    let threshold_override = if threshold == app.config.base_threshold {
        None
    } else {
        Some(threshold)
    };

    match persist_threshold_override(&app.manifest_path, &source_key, threshold_override) {
        Ok(()) => {
            if let Some(glyph) = selected_glyph_mut(app) {
                glyph.working_threshold = threshold;
                glyph.saved_threshold = threshold_override;
            }
            app.status = Some(match threshold_override {
                Some(value) => format!("saved override for {glyph_name}: {source_key} -> {value}"),
                None => format!(
                    "cleared override for {glyph_name}: now using base threshold {}",
                    app.config.base_threshold
                ),
            });
        }
        Err(err) => {
            app.status = Some(format!("failed to save override for {glyph_name}: {err}"));
        }
    }
}

fn remove_selected_threshold_override(app: &mut App) {
    if app.active_project.is_none() {
        app.status = Some(
            "create a project in Home or relaunch with --manifest before tuning glyphs".to_string(),
        );
        return;
    }

    let Some(glyph) = selected_glyph(app) else {
        app.status = Some("no glyph selected".to_string());
        return;
    };

    let source_key = glyph.glyph.source_key.clone();
    let glyph_name = glyph.glyph.glyph_name.clone();

    match persist_threshold_override(&app.manifest_path, &source_key, None) {
        Ok(()) => {
            let base_threshold = app.config.base_threshold;
            if let Some(glyph) = selected_glyph_mut(app) {
                glyph.saved_threshold = None;
                glyph.working_threshold = base_threshold;
            }
            app.status = Some(format!(
                "removed override for {glyph_name}: now using base threshold {}",
                base_threshold
            ));
        }
        Err(err) => {
            app.status = Some(format!("failed to remove override for {glyph_name}: {err}"));
        }
    }
}

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
    let is_global_panel_jump = matches!(code, KeyCode::Tab | KeyCode::BackTab)
        || (matches!(code, KeyCode::Char('2')) && !app.welcome_input_editing);

    if app.view == AppView::Welcome && app.delete_project_confirm_selection.is_some() {
        tui_debug_log(
            "handle_key_event.route_delete_confirm",
            app_debug_state(app),
        );
        let result = handle_welcome_key(app, key);
        tui_debug_log("handle_key_event.exit_delete_confirm", app_debug_state(app));
        return result;
    }

    if app.view == AppView::Welcome && !is_global_panel_jump {
        tui_debug_log("handle_key_event.route_welcome", app_debug_state(app));
        let result = handle_welcome_key(app, key);
        tui_debug_log("handle_key_event.exit_welcome", app_debug_state(app));
        return result;
    }

    if app.view == AppView::Glyphs {
        if matches!(
            code,
            KeyCode::Right | KeyCode::Char('l') | KeyCode::Char('+') | KeyCode::Char('=')
        ) || is_keypad_plus_alias(&key)
        {
            adjust_selected_threshold_by(app, 1);
            tui_debug_log("handle_key_event.exit_global", app_debug_state(app));
            return Ok(());
        }
        if matches!(
            code,
            KeyCode::Left | KeyCode::Char('h') | KeyCode::Char('-')
        ) || is_keypad_minus_alias(&key)
        {
            adjust_selected_threshold_by(app, -1);
            tui_debug_log("handle_key_event.exit_global", app_debug_state(app));
            return Ok(());
        }
    }

    match code {
        KeyCode::Esc | KeyCode::Char('q') => app.quit = true,
        KeyCode::Char('1') => {
            app.welcome_input_editing = false;
            app.view = AppView::Welcome;
        }
        KeyCode::Char('2') => {
            app.welcome_input_editing = false;
            app.view = AppView::Glyphs;
        }
        KeyCode::Tab => {
            app.welcome_input_editing = false;
            app.view = match app.view {
                AppView::Welcome => AppView::Glyphs,
                AppView::Glyphs => AppView::Welcome,
            }
        }
        KeyCode::Char('R') => {
            if app.build_in_progress() || app.install_in_progress() {
                app.status =
                    Some("a background task is in progress; wait before rescanning".to_string());
                tui_debug_log("handle_key_event.exit_global", app_debug_state(app));
                return Ok(());
            }
            app.refresh_workspace_discovery()?;
            app.reload_glyphs()?;
            app.view = if app.glyphs.is_empty() {
                AppView::Welcome
            } else {
                AppView::Glyphs
            };
        }
        KeyCode::Char('b') => {
            trigger_build_action(app)?;
        }
        KeyCode::Char('i') => {
            trigger_install_action(app)?;
        }
        KeyCode::Down => {
            if app.view == AppView::Glyphs {
                app.selected = (app.selected + 1).min(app.glyphs.len().saturating_sub(1));
            }
        }
        KeyCode::Char('j') => {
            if app.view == AppView::Glyphs {
                app.selected = (app.selected + 1).min(app.glyphs.len().saturating_sub(1));
            }
        }
        KeyCode::Up => {
            if app.view == AppView::Glyphs {
                app.selected = app.selected.saturating_sub(1);
            }
        }
        KeyCode::Char('k') => {
            if app.view == AppView::Glyphs {
                app.selected = app.selected.saturating_sub(1);
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

fn trigger_build_action(app: &mut App) -> Result<()> {
    if app.build_in_progress() {
        app.status = Some("build already in progress".to_string());
        return Ok(());
    }
    if app.install_in_progress() {
        app.status = Some("font operation is in progress; wait for it to finish".to_string());
        return Ok(());
    }
    app.start_build_project()
}

fn trigger_home_tool_action(app: &mut App) -> Result<()> {
    match app.selected_home_tool {
        HomeToolAction::ComposeGrid => {
            if app.active_project.is_none() {
                app.status =
                    Some("select or create a project before composing combined glyphs".to_string());
            } else {
                app.status = Some(
                    "Compose Grid is planned for Home project tools but is not implemented yet"
                        .to_string(),
                );
            }
            Ok(())
        }
        HomeToolAction::AnimateGlyph => {
            if app.active_project.is_none() {
                app.status =
                    Some("select or create a project before preparing animated glyphs".to_string());
            } else {
                app.status = Some(
                    "Animate Glyph is planned for Home project tools but is not implemented yet"
                        .to_string(),
                );
            }
            Ok(())
        }
    }
}

fn trigger_install_action(app: &mut App) -> Result<()> {
    if app.build_in_progress() {
        app.status = Some("build is in progress; wait for it to finish".to_string());
        return Ok(());
    }
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

    frame.render_widget(Clear, area);

    if !terminal_size_supported(area) {
        draw_terminal_too_small(frame, area, accent, muted);
        return;
    }
    let area = centered_bounded_viewport(area);

    let status_height = if app.switch_notice.is_some() { 1 } else { 0 };
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),             // Header
            Constraint::Length(status_height), // Switch notice
            Constraint::Min(0),                // Body
            Constraint::Length(1),             // Footer keys
        ])
        .split(area);

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

    if let Some(notice) = &app.switch_notice {
        let notice_line = Line::from(vec![Span::styled(
            format!(
                " Switched project: {} -> {} ",
                notice.from_label, notice.to_label
            ),
            Style::default()
                .fg(Color::Black)
                .bg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )]);
        frame.render_widget(
            Paragraph::new(notice_line).alignment(Alignment::Center),
            root[1],
        );
    }

    // Body
    let body_area = root[2];

    match app.view {
        AppView::Welcome => draw_welcome_view(frame, app, body_area, accent, muted),
        AppView::Glyphs => draw_glyphs_view(frame, app, body_area, accent, muted),
    }
    draw_delete_project_confirmation_popup(frame, app, area, accent, muted);

    // Footer
    let mut footer_spans = vec![
        Span::styled(" q/esc ", Style::default().fg(accent)),
        Span::raw("quit  "),
        Span::styled(" tab ", Style::default().fg(accent)),
        Span::raw("next panel  "),
        Span::styled(" 1-2 ", Style::default().fg(accent)),
        Span::raw("jump panel  "),
        Span::styled(" R ", Style::default().fg(accent)),
        Span::raw("rescan  "),
        Span::styled(" b ", Style::default().fg(accent)),
        Span::raw(if app.current_project_is_built() {
            "rebuild  "
        } else {
            "build  "
        }),
        Span::styled(" i ", Style::default().fg(accent)),
        Span::raw(if app.current_project_is_installed() {
            "uninstall  "
        } else {
            "install  "
        }),
    ];

    if app.view == AppView::Welcome {
        let enter_help = if app.welcome_input_editing {
            "stop typing  "
        } else if app.delete_project_confirm_selection.is_some() {
            "confirm  "
        } else if app.welcome_focus == WelcomeFocus::ProjectList {
            "open project  "
        } else if app.welcome_focus == WelcomeFocus::BuildButton {
            if app.current_project_is_built() {
                "rebuild  "
            } else {
                "build  "
            }
        } else if app.welcome_focus == WelcomeFocus::InstallButton {
            if app.current_project_is_installed() {
                "uninstall  "
            } else {
                "install  "
            }
        } else if app.welcome_focus == WelcomeFocus::DeleteProjectButton {
            "delete project  "
        } else if app.welcome_focus == WelcomeFocus::InstalledFontList {
            "uninstall  "
        } else if app.welcome_focus == WelcomeFocus::ToolList {
            "run action  "
        } else {
            "type/create  "
        };
        footer_spans.extend(vec![
            Span::styled(" \u{2191}/\u{2193} ", Style::default().fg(accent)),
            Span::raw("select  "),
            Span::styled(" \u{2190}/\u{2192} ", Style::default().fg(accent)),
            Span::raw("switch section  "),
            Span::styled(" Enter ", Style::default().fg(accent)),
            Span::raw(enter_help),
            Span::styled(" Backspace ", Style::default().fg(accent)),
            Span::raw("delete char  "),
        ]);
        if app.welcome_input_editing {
            footer_spans.extend(vec![
                Span::styled(" Esc ", Style::default().fg(accent)),
                Span::raw("stop typing  "),
            ]);
        } else if app.delete_project_confirm_selection.is_some() {
            footer_spans.extend(vec![
                Span::styled(" Esc ", Style::default().fg(accent)),
                Span::raw("cancel delete  "),
            ]);
        }
    }
    if app.view == AppView::Glyphs {
        footer_spans.extend(vec![
            Span::styled(" \u{2191}/\u{2193} ", Style::default().fg(accent)),
            Span::raw("select  "),
            Span::styled(" \u{2190}/\u{2192} ", Style::default().fg(accent)),
            Span::raw("thresh +/-1  "),
            Span::styled(" PgUp/PgDn ", Style::default().fg(accent)),
            Span::raw("thresh +/-10  "),
            Span::styled(" r ", Style::default().fg(accent)),
            Span::raw("reset  "),
        ]);
    }

    if let Some(status) = &app.status {
        footer_spans.push(Span::styled(" | ", Style::default().fg(muted)));
        footer_spans.push(Span::styled(
            status.clone(),
            Style::default().fg(Color::LightRed),
        ));
    }

    if let (Some(spinner), Some(kind)) = (app.build_task_spinner_frame(), app.build_task_kind()) {
        footer_spans.push(Span::styled(" | ", Style::default().fg(muted)));
        footer_spans.push(Span::styled(
            format!("{spinner} {}", kind.footer_label()),
            Style::default().fg(accent),
        ));
    } else if let (Some(spinner), Some(kind)) =
        (app.font_task_spinner_frame(), app.font_task_kind())
    {
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
    frame.render_widget(footer, root[3]);
}

fn draw_delete_project_confirmation_popup(
    frame: &mut Frame,
    app: &App,
    area: Rect,
    accent: Color,
    muted: Color,
) {
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
    let popup = centered_popup_rect(area, 94, 14);
    frame.render_widget(Clear, popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::LightRed))
        .title(Span::styled(
            " Confirm Deletion ",
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        ));
    let selected_button = DeleteProjectConfirmSlot::from_index(selection);
    let selected_style = Style::default()
        .fg(Color::Black)
        .bg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    let danger_style = Style::default()
        .fg(Color::White)
        .bg(Color::Red)
        .add_modifier(Modifier::BOLD);
    let idle_style = Style::default().fg(Color::White).bg(Color::DarkGray);
    let hop_style = Style::default().fg(Color::Black).bg(Color::DarkGray);
    let slot_text = "        ";
    let h_gap = "  ";
    let left_pad = "  ";
    let branch_indent = format!(
        "{}{}{}{}{}",
        left_pad,
        " ".repeat(" CANCEL ".chars().count()),
        h_gap,
        " ".repeat(slot_text.chars().count()),
        h_gap
    );

    let top_row = Line::from(vec![
        Span::raw(left_pad),
        Span::styled(
            " CANCEL ",
            if selected_button == DeleteProjectConfirmSlot::Cancel {
                selected_style
            } else {
                idle_style
            },
        ),
        Span::raw(h_gap),
        Span::styled(
            slot_text,
            if selected_button == DeleteProjectConfirmSlot::Hop1 {
                selected_style
            } else {
                hop_style
            },
        ),
        Span::raw(h_gap),
        Span::styled(
            slot_text,
            if selected_button == DeleteProjectConfirmSlot::Hop2 {
                selected_style
            } else {
                hop_style
            },
        ),
    ]);

    let middle_row = Line::from(vec![
        Span::raw(&branch_indent),
        Span::styled(
            slot_text,
            if selected_button == DeleteProjectConfirmSlot::Hop3 {
                selected_style
            } else {
                hop_style
            },
        ),
    ]);

    let bottom_row = Line::from(vec![
        Span::raw(&branch_indent),
        Span::styled(
            slot_text,
            if selected_button == DeleteProjectConfirmSlot::Hop4 {
                selected_style
            } else {
                hop_style
            },
        ),
        Span::raw(h_gap),
        Span::styled(
            slot_text,
            if selected_button == DeleteProjectConfirmSlot::Hop5 {
                selected_style
            } else {
                hop_style
            },
        ),
        Span::raw(h_gap),
        Span::styled(
            " DELETE ",
            if selected_button == DeleteProjectConfirmSlot::Delete {
                danger_style
            } else {
                idle_style
            },
        ),
    ]);

    let lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::raw("  "),
            Span::styled(
                format!("Delete project `{project_label}`?"),
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(vec![
            Span::raw("  "),
            Span::styled(
                "Reach DELETE by following the slot path with turns.",
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(vec![
            Span::raw("  "),
            Span::styled(
                "Use arrows (or h/j/k/l). Enter confirms selected action. Esc cancels.",
                Style::default().fg(muted),
            ),
        ]),
        Line::from(""),
        top_row,
        Line::from(""),
        middle_row,
        Line::from(""),
        bottom_row,
    ];
    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false }),
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

fn draw_glyphs_view(
    frame: &mut Frame,
    app: &App,
    area: ratatui::layout::Rect,
    accent: Color,
    muted: Color,
) {
    if app.active_project.is_none() {
        draw_blocked_project_view(frame, area, " Glyphs ", accent, muted);
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
        .split(area);

    let list_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(muted))
        .title(Span::styled(" Glyphs ", Style::default().fg(accent)));

    let mut list_state = ListState::default();
    if !app.glyphs.is_empty() {
        list_state.select(Some(app.selected));
    }

    let items: Vec<ListItem> = if app.glyphs.is_empty() {
        vec![ListItem::new(" No glyphs found. ")]
    } else {
        app.glyphs
            .iter()
            .enumerate()
            .map(|(idx, g)| {
                let codepoint = app.config.codepoint_start + idx as u32;
                let marker = if g.saved_threshold.is_some() {
                    " *"
                } else {
                    "  "
                };
                let name = &g.glyph.glyph_name;
                ListItem::new(Line::from(vec![
                    Span::styled(marker, Style::default().fg(Color::Yellow)),
                    Span::styled(format!(" U+{:04X} ", codepoint), Style::default().fg(muted)),
                    Span::raw(name.clone()),
                ]))
            })
            .collect()
    };

    let list = List::new(items)
        .block(list_block)
        .highlight_style(
            Style::default()
                .fg(Color::Black)
                .bg(accent)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol(" \u{2023} ");

    frame.render_stateful_widget(list, chunks[0], &mut list_state);

    let preview_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(muted))
        .title(Span::styled(" Preview ", Style::default().fg(accent)));

    let preview_area = preview_block.inner(chunks[1]);

    let mut preview_content = if app.glyphs.is_empty() {
        vec![
            Line::from(""),
            Line::from("  Add or drag images into this project."),
        ]
    } else {
        let active = &app.glyphs[app.selected];
        vec![
            Line::from(""),
            Line::from(vec![
                Span::raw("  File: "),
                Span::styled(
                    active.glyph.source_path.to_string_lossy().to_string(),
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
            Line::from(""),
        ]
    };

    if !app.glyphs.is_empty() {
        let active = &app.glyphs[app.selected];
        let mut ascii = preview_lines(
            &active.glyph,
            active.working_threshold,
            preview_area.width.saturating_sub(4) / 2,
            preview_area.height.saturating_sub(5),
        );
        preview_content.append(&mut ascii);
    }

    let p = Paragraph::new(preview_content)
        .block(preview_block)
        .wrap(Wrap { trim: false });
    frame.render_widget(p, chunks[1]);
}

fn preview_lines(
    glyph: &PreprocessedGlyph,
    threshold: u8,
    max_w: u16,
    max_h: u16,
) -> Vec<Line<'static>> {
    const PREVIEW_X_COMP: f32 = 0.88;

    let src = glyph.size as usize;
    if src == 0 || max_w == 0 || max_h == 0 {
        return vec![Line::from("  [Preview too small]")];
    }

    let mut found_on = false;
    let mut sum_x = 0f32;
    let mut sum_y = 0f32;
    let mut on_count = 0usize;
    for y in 0..src {
        for x in 0..src {
            let idx = y * src + x;
            if glyph.coverage[idx] < threshold {
                continue;
            }
            found_on = true;
            sum_x += x as f32;
            sum_y += y as f32;
            on_count += 1;
        }
    }

    let out_w = ((usize::max(1, usize::min(src, max_w as usize)) as f32) * PREVIEW_X_COMP)
        .round()
        .max(1.0) as usize;
    let out_h = usize::max(1, usize::min(src, max_h as usize));
    let sample_idx = |out_idx: usize, out_len: usize| -> usize {
        let numerator = (2 * out_idx + 1) * src;
        let denominator = 2 * out_len;
        (numerator / denominator).min(src.saturating_sub(1))
    };
    let (shift_x, shift_y) = if found_on && on_count > 0 {
        let src_center_x = (src.saturating_sub(1)) as f32 / 2.0;
        let src_center_y = (src.saturating_sub(1)) as f32 / 2.0;
        let on_center_x = sum_x / on_count as f32;
        let on_center_y = sum_y / on_count as f32;
        (src_center_x - on_center_x, src_center_y - on_center_y)
    } else {
        (0.0, 0.0)
    };

    let mut rows = Vec::with_capacity(out_h);

    for oy in 0..out_h {
        let mut row = String::with_capacity(out_w * 2 + 4);
        row.push_str("    ");
        for ox in 0..out_w {
            let vy = sample_idx(oy, out_h) as f32;
            let vx = sample_idx(ox, out_w) as f32;
            let sy = (vy - shift_y).round() as i32;
            let sx = (vx - shift_x).round() as i32;
            let on = if sx >= 0 && sy >= 0 && (sx as usize) < src && (sy as usize) < src {
                let idx = sy as usize * src + sx as usize;
                glyph.coverage[idx] >= threshold
            } else {
                false
            };
            row.push_str(if on { "██" } else { "  " });
        }
        rows.push(row);
    }

    rows.retain(|row| row.contains('█'));
    if rows.is_empty() {
        return vec![Line::from("    [No visible pixels at threshold]")];
    }

    rows.into_iter().map(Line::from).collect()
}

fn looks_like_path_payload(payload: &str) -> bool {
    let trimmed = payload.trim();
    if trimmed.is_empty() {
        return false;
    }
    trimmed.contains('/') || trimmed.starts_with("file://") || trimmed.contains('\\')
}

fn collect_dropped_paths(payload: &str) -> Vec<PathBuf> {
    let normalized = payload.replace("\r\n", "\n").replace('\r', "\n");
    let mut fragments = Vec::new();
    let trimmed = normalized.trim();
    if !trimmed.is_empty() {
        fragments.push(trimmed.to_string());
    }
    for line in normalized.lines() {
        let line = line.trim();
        if !line.is_empty() {
            fragments.push(line.to_string());
        }
    }

    let mut seen = BTreeSet::new();
    let mut out = Vec::new();

    for fragment in fragments {
        let mut candidates = vec![fragment.clone()];
        candidates.extend(split_shell_like_tokens(&fragment));

        for candidate in candidates {
            let Some(path) = normalize_dropped_path_candidate(&candidate) else {
                continue;
            };
            let key = path.to_string_lossy().to_string();
            if seen.insert(key) {
                out.push(path);
            }
        }
    }

    out
}

fn split_shell_like_tokens(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut escaped = false;
    let mut in_single_quote = false;
    let mut in_double_quote = false;

    for ch in input.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }

        if ch == '\\' && !in_single_quote {
            escaped = true;
            continue;
        }

        if ch == '\'' && !in_double_quote {
            in_single_quote = !in_single_quote;
            continue;
        }

        if ch == '"' && !in_single_quote {
            in_double_quote = !in_double_quote;
            continue;
        }

        if ch.is_whitespace() && !in_single_quote && !in_double_quote {
            if !current.is_empty() {
                tokens.push(current.clone());
                current.clear();
            }
            continue;
        }

        current.push(ch);
    }

    if escaped {
        current.push('\\');
    }

    if !current.is_empty() {
        tokens.push(current);
    }

    tokens
}

fn normalize_dropped_path_candidate(candidate: &str) -> Option<PathBuf> {
    let trimmed = candidate.trim().trim_end_matches('\0');
    if trimmed.is_empty() {
        return None;
    }

    let stripped = strip_wrapping_quotes(trimmed);
    if let Some(uri_path) = stripped.strip_prefix("file://") {
        return Some(PathBuf::from(decode_file_uri_path(uri_path)));
    }

    Some(PathBuf::from(unescape_backslashes(stripped)))
}

fn strip_wrapping_quotes(value: &str) -> &str {
    if value.len() >= 2 {
        let bytes = value.as_bytes();
        let starts = bytes[0];
        let ends = bytes[value.len() - 1];
        if (starts == b'"' && ends == b'"') || (starts == b'\'' && ends == b'\'') {
            return &value[1..value.len() - 1];
        }
    }
    value
}

fn decode_file_uri_path(uri_path: &str) -> String {
    let mut path = uri_path;
    if let Some(rest) = path.strip_prefix("localhost") {
        path = rest;
    }
    percent_decode(path)
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0usize;

    while index < bytes.len() {
        if bytes[index] == b'%'
            && index + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (hex_value(bytes[index + 1]), hex_value(bytes[index + 2]))
        {
            out.push((hi << 4) | lo);
            index += 3;
            continue;
        }

        out.push(bytes[index]);
        index += 1;
    }

    String::from_utf8_lossy(&out).into_owned()
}

fn hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn unescape_backslashes(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut escaped = false;

    for ch in value.chars() {
        if escaped {
            out.push(ch);
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else {
            out.push(ch);
        }
    }

    if escaped {
        out.push('\\');
    }
    out
}

fn paths_resolve_to_same_file(left: &Path, right: &Path) -> bool {
    let Ok(left) = fs::canonicalize(left) else {
        return false;
    };
    let Ok(right) = fs::canonicalize(right) else {
        return false;
    };
    left == right
}

fn next_available_import_destination(
    input_dir: &Path,
    file_name: &std::ffi::OsStr,
) -> (PathBuf, bool) {
    let candidate = input_dir.join(file_name);
    if !candidate.exists() {
        return (candidate, false);
    }

    let stem = Path::new(file_name)
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("glyph");
    let ext = Path::new(file_name)
        .extension()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty());

    for index in 1.. {
        let renamed = match ext {
            Some(ext) => format!("{stem}-{index}.{ext}"),
            None => format!("{stem}-{index}"),
        };
        let next = input_dir.join(renamed);
        if !next.exists() {
            return (next, true);
        }
    }

    (candidate, false)
}

fn format_drop_import_status(
    imported: usize,
    renamed: usize,
    skipped_existing: usize,
    skipped_unsupported: usize,
    skipped_missing: usize,
) -> String {
    format!(
        "drop import: {imported} added, {renamed} renamed, {skipped_existing} already present, {skipped_unsupported} unsupported, {skipped_missing} missing"
    )
}

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
        return format!("restart all {name} terminals to render newly installed glyphs");
    }
    "restart all terminals to render newly installed glyphs".to_string()
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
) -> Vec<Line<'static>> {
    if available_height < 3 {
        return Vec::new();
    }

    let max_line_width = usize::from(available_width.saturating_sub(4));
    if max_line_width < 8 {
        return Vec::new();
    }

    let inner_width = max_line_width.saturating_sub(2);
    let top_bottom = format!("+{}+", dashed_pattern(inner_width));
    let empty = format!("|{}|", " ".repeat(inner_width));
    let centered_label = center_label("DRAG IMAGES HERE", inner_width);
    let border_style = Style::default().fg(accent);
    let label_style = Style::default().fg(accent).add_modifier(Modifier::BOLD);

    let inner_rows = available_height.saturating_sub(2);
    let label_row = usize::from(inner_rows / 2);

    let mut lines = Vec::with_capacity(usize::from(available_height));
    lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled(top_bottom.clone(), border_style),
    ]));

    for row in 0..usize::from(inner_rows) {
        if row == label_row {
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled("|", border_style),
                Span::styled(centered_label.clone(), label_style),
                Span::styled("|", border_style),
            ]));
        } else {
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(empty.clone(), border_style),
            ]));
        }
    }

    lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled(top_bottom, border_style),
    ]));
    lines
}

fn dashed_pattern(width: usize) -> String {
    let mut out = String::with_capacity(width);
    for idx in 0..width {
        out.push(if idx % 2 == 0 { '-' } else { ' ' });
    }
    out
}

fn center_label(label: &str, width: usize) -> String {
    let label = if label.len() > width {
        label.chars().take(width).collect::<String>()
    } else {
        label.to_string()
    };
    let padding = width.saturating_sub(label.len());
    let left = padding / 2;
    let right = padding - left;
    format!("{}{}{}", " ".repeat(left), label, " ".repeat(right))
}

pub(crate) fn wrap_sample_for_display(sample: &str, max_chars: usize) -> Vec<String> {
    if sample.is_empty() {
        return Vec::new();
    }

    let target = max_chars.max(1);
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut count = 0usize;

    for ch in sample.chars() {
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

    lines
}

pub(crate) fn spaced_sample(sample: &str) -> String {
    let mut out = String::new();
    for (index, ch) in sample.chars().enumerate() {
        if index > 0 {
            out.push_str("  ");
        }
        out.push(ch);
    }
    out
}
