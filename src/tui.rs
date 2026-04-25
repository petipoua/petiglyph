use anyhow::{Context, Result, bail};
use crossterm::ExecutableCommand;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, List, ListItem, ListState, Paragraph, Tabs, Wrap,
};
use ratatui::{Frame, Terminal};
use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::Duration;
use walkdir::WalkDir;

use crate::build::{
    BuildSummary, MappingEntry, PreprocessedGlyph, build_outputs, expected_bdf_path,
    expected_ttf_path, glyph_sample_string, is_supported_source, preprocess_sources,
};
use crate::install::{
    FontInstallNameMode, expected_install_ttf_path, expected_install_ttf_path_for_mode,
    install_built_font, install_dir_for_manifest,
};
use crate::project::{
    RuntimeConfig, create_project_in_dir, discover_project_manifests, load_runtime_config,
    read_manifest, write_manifest,
};

const CLI_VERSION: &str = env!("CARGO_PKG_VERSION");
const INSTALL_SPINNER_FRAMES: [&str; 4] = ["-", "\\", "|", "/"];
const WELCOME_SAMPLE_LIMIT: usize = 15;
const WELCOME_INPUT_WIDTH: usize = 15;

#[derive(Debug, Clone, Default)]
pub(crate) struct TuiLaunchOverrides {
    pub(crate) input_dir: Option<PathBuf>,
    pub(crate) threshold: Option<u8>,
    pub(crate) glyph_size: Option<u32>,
    pub(crate) codepoint_start: Option<String>,
}

#[derive(Debug, Clone)]
struct WelcomeProject {
    manifest_path: PathBuf,
    font_name: String,
}

#[derive(Debug, Clone)]
struct InstalledFontSample {
    file_name: String,
    path: PathBuf,
    sample: String,
    truncated: bool,
}

struct WelcomeApp {
    cwd: PathBuf,
    projects: Vec<WelcomeProject>,
    selected_project: usize,
    installed_fonts: Vec<InstalledFontSample>,
    create_input: String,
    focus: WelcomeFocus,
    quit: bool,
    status: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WelcomeFocus {
    Projects,
    CreateInput,
    CreateButton,
}

impl WelcomeFocus {
    fn next(self, has_projects: bool) -> Self {
        if !has_projects {
            return match self {
                Self::CreateInput | Self::Projects => Self::CreateButton,
                Self::CreateButton => Self::CreateInput,
            };
        }

        match self {
            Self::Projects => Self::CreateInput,
            Self::CreateInput => Self::CreateButton,
            Self::CreateButton => Self::Projects,
        }
    }

    fn prev(self, has_projects: bool) -> Self {
        if !has_projects {
            return match self {
                Self::CreateInput | Self::Projects => Self::CreateButton,
                Self::CreateButton => Self::CreateInput,
            };
        }

        match self {
            Self::Projects => Self::CreateButton,
            Self::CreateInput => Self::Projects,
            Self::CreateButton => Self::CreateInput,
        }
    }
}

enum WelcomeNavigationAction {
    OpenProject(PathBuf),
    Stay,
}

pub(crate) fn tui(
    manifest_path: PathBuf,
    input_override: Option<PathBuf>,
    threshold_override: Option<u8>,
    glyph_size_override: Option<u32>,
    codepoint_start_override: Option<String>,
) -> Result<()> {
    tui_with_welcome_root(
        manifest_path,
        input_override,
        threshold_override,
        glyph_size_override,
        codepoint_start_override,
        None,
    )
}

fn tui_with_welcome_root(
    manifest_path: PathBuf,
    input_override: Option<PathBuf>,
    threshold_override: Option<u8>,
    glyph_size_override: Option<u32>,
    codepoint_start_override: Option<String>,
    welcome_root: Option<PathBuf>,
) -> Result<()> {
    let launch_overrides = TuiLaunchOverrides {
        input_dir: input_override.clone(),
        threshold: threshold_override,
        glyph_size: glyph_size_override,
        codepoint_start: codepoint_start_override.clone(),
    };
    let config = load_runtime_config(
        &manifest_path,
        input_override,
        None,
        threshold_override,
        glyph_size_override,
        codepoint_start_override,
    )?;

    let mut app = App::new_with_overrides(manifest_path, config, launch_overrides, welcome_root);
    app.reload_glyphs()?;

    let mut session = TerminalSession::start()?;
    while !app.quit {
        app.poll_install_task();
        session.terminal.draw(|frame| draw_ui(frame, &app))?;

        if event::poll(Duration::from_millis(120))?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
            && let Err(err) = handle_key(&mut app, key.code)
        {
            app.status = Some(err.to_string());
        }
    }

    let navigation_action = app.navigation_action.take();
    let project_dir = app.project_dir.clone();
    drop(session);

    if let Some(NavigationAction::OpenWelcome(cwd)) = navigation_action {
        return tui_welcome(cwd);
    }

    println!("tui session closed for {}", project_dir.display());
    Ok(())
}

pub(crate) fn tui_welcome(cwd: PathBuf) -> Result<()> {
    let mut app = WelcomeApp::new(cwd);
    app.rescan()?;

    let mut session = TerminalSession::start()?;
    let mut next_manifest: Option<PathBuf> = None;

    while !app.quit {
        session
            .terminal
            .draw(|frame| draw_welcome_ui(frame, &app))?;

        if event::poll(Duration::from_millis(120))?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            match handle_welcome_key(&mut app, key.code)? {
                WelcomeNavigationAction::OpenProject(path) => {
                    next_manifest = Some(path);
                    app.quit = true;
                }
                WelcomeNavigationAction::Stay => {}
            }
        }
    }

    drop(session);
    if let Some(manifest_path) = next_manifest {
        return tui_with_welcome_root(manifest_path, None, None, None, None, Some(app.cwd));
    }

    println!("tui welcome session closed for {}", app.cwd.display());
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AppView {
    Home,
    Glyphs,
    Font,
}

#[derive(Debug, Clone)]
enum NavigationAction {
    OpenWelcome(PathBuf),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FontAction {
    Build,
    Install,
}

impl FontAction {
    fn next(self) -> Self {
        match self {
            Self::Build => Self::Install,
            Self::Install => Self::Build,
        }
    }

    fn prev(self) -> Self {
        match self {
            Self::Build => Self::Install,
            Self::Install => Self::Build,
        }
    }
}

pub(crate) struct App {
    pub(crate) manifest_path: PathBuf,
    pub(crate) project_dir: PathBuf,
    pub(crate) config: RuntimeConfig,
    pub(crate) selected: usize,
    pub(crate) glyphs: Vec<InteractiveGlyph>,
    pub(crate) quit: bool,
    pub(crate) status: Option<String>,
    pub(crate) view: AppView,
    pub(crate) last_build: Option<BuildSummary>,
    pub(crate) last_sample: Option<String>,
    pub(crate) installed_font_path: Option<PathBuf>,
    pub(crate) selected_font_action: FontAction,
    launch_overrides: TuiLaunchOverrides,
    welcome_root: Option<PathBuf>,
    navigation_action: Option<NavigationAction>,
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
}

struct InstallTask {
    receiver: Receiver<Result<InstallTaskOutput, String>>,
    spinner_index: usize,
}

struct InstallTaskOutput {
    summary: BuildSummary,
    sample: Option<String>,
    installed_path: PathBuf,
}

impl TerminalSession {
    fn start() -> Result<Self> {
        enable_raw_mode().context("failed to enable raw mode")?;
        let mut stdout = io::stdout();
        stdout
            .execute(EnterAlternateScreen)
            .context("failed to enter alternate screen")?;
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend).context("failed to initialize terminal UI")?;
        Ok(Self { terminal })
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = self.terminal.backend_mut().execute(LeaveAlternateScreen);
        let _ = self.terminal.show_cursor();
    }
}

impl WelcomeApp {
    fn new(cwd: PathBuf) -> Self {
        Self {
            cwd,
            projects: Vec::new(),
            selected_project: 0,
            installed_fonts: Vec::new(),
            create_input: String::new(),
            focus: WelcomeFocus::Projects,
            quit: false,
            status: None,
        }
    }

    fn rescan(&mut self) -> Result<()> {
        self.projects = scan_projects_in_scope(&self.cwd)?;
        self.selected_project = self
            .selected_project
            .min(self.projects.len().saturating_sub(1));

        match scan_installed_petiglyph_fonts(&self.cwd) {
            Ok(fonts) => {
                self.installed_fonts = fonts;
            }
            Err(err) => {
                self.installed_fonts.clear();
                self.status = Some(format!("font scan warning: {err}"));
            }
        }

        if self.projects.is_empty() {
            self.focus = WelcomeFocus::CreateInput;
            self.status = Some(format!(
                "no petiglyph project in {} (type a name and press Enter on Create)",
                self.cwd.display()
            ));
        } else if self.projects.len() == 1 {
            self.status = Some("one project detected (press Enter to open)".to_string());
        } else {
            self.status = Some(format!(
                "{} projects detected (choose one and press Enter)",
                self.projects.len()
            ));
        }

        Ok(())
    }

    fn submit_create(&mut self) -> Result<WelcomeNavigationAction> {
        let project_name = self.create_input.trim().to_string();
        if project_name.is_empty() {
            self.status = Some("project name cannot be empty".to_string());
            self.focus = WelcomeFocus::CreateInput;
            return Ok(WelcomeNavigationAction::Stay);
        }

        let manifest_path = create_project_in_dir(&self.cwd, &project_name)?;
        self.create_input.clear();
        self.status = Some(format!("created project `{project_name}`"));
        Ok(WelcomeNavigationAction::OpenProject(manifest_path))
    }
}

fn scan_projects_in_scope(cwd: &Path) -> Result<Vec<WelcomeProject>> {
    discover_project_manifests(cwd)?
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

fn is_valid_project_name_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.')
}

pub(crate) fn format_welcome_input_field(value: &str, focused: bool, width: usize) -> String {
    let width = width.max(1);
    let mut field = vec![' '; width];
    let trimmed = value.trim();

    if trimmed.is_empty() && !focused {
        let placeholder = "<project-name>";
        for (idx, ch) in placeholder.chars().take(width).enumerate() {
            field[idx] = ch;
        }
    } else {
        let mut len = 0usize;
        for (idx, ch) in trimmed.chars().take(width).enumerate() {
            field[idx] = ch;
            len = idx + 1;
        }

        if focused {
            let cursor_index = len.min(width - 1);
            field[cursor_index] = '_';
        }
    }

    let content = field.into_iter().collect::<String>();
    format!(" {content} ")
}

fn handle_welcome_key(app: &mut WelcomeApp, code: KeyCode) -> Result<WelcomeNavigationAction> {
    match code {
        KeyCode::Esc | KeyCode::Char('q') => {
            app.quit = true;
            Ok(WelcomeNavigationAction::Stay)
        }
        KeyCode::Char('R') => {
            app.rescan()?;
            Ok(WelcomeNavigationAction::Stay)
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if app.focus == WelcomeFocus::Projects {
                app.selected_project =
                    (app.selected_project + 1).min(app.projects.len().saturating_sub(1));
            }
            Ok(WelcomeNavigationAction::Stay)
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if app.focus == WelcomeFocus::Projects {
                app.selected_project = app.selected_project.saturating_sub(1);
            }
            Ok(WelcomeNavigationAction::Stay)
        }
        KeyCode::Tab => {
            app.focus = app.focus.next(!app.projects.is_empty());
            Ok(WelcomeNavigationAction::Stay)
        }
        KeyCode::BackTab => {
            app.focus = app.focus.prev(!app.projects.is_empty());
            Ok(WelcomeNavigationAction::Stay)
        }
        KeyCode::Left => {
            app.focus = match app.focus {
                WelcomeFocus::CreateButton => WelcomeFocus::CreateInput,
                other => other,
            };
            Ok(WelcomeNavigationAction::Stay)
        }
        KeyCode::Right => {
            app.focus = match app.focus {
                WelcomeFocus::CreateInput => WelcomeFocus::CreateButton,
                other => other,
            };
            Ok(WelcomeNavigationAction::Stay)
        }
        KeyCode::Backspace => {
            if app.focus == WelcomeFocus::CreateInput {
                app.create_input.pop();
            }
            Ok(WelcomeNavigationAction::Stay)
        }
        KeyCode::Char(ch)
            if app.focus == WelcomeFocus::CreateInput && is_valid_project_name_char(ch) =>
        {
            app.create_input.push(ch);
            Ok(WelcomeNavigationAction::Stay)
        }
        KeyCode::Enter => match app.focus {
            WelcomeFocus::Projects => {
                if let Some(project) = app.projects.get(app.selected_project) {
                    Ok(WelcomeNavigationAction::OpenProject(
                        project.manifest_path.clone(),
                    ))
                } else {
                    app.focus = WelcomeFocus::CreateInput;
                    app.status = Some(
                        "no project selected; type a name and press Enter on Create".to_string(),
                    );
                    Ok(WelcomeNavigationAction::Stay)
                }
            }
            WelcomeFocus::CreateInput => {
                app.focus = WelcomeFocus::CreateButton;
                app.status = Some("press Enter to create project".to_string());
                Ok(WelcomeNavigationAction::Stay)
            }
            WelcomeFocus::CreateButton => app.submit_create(),
        },
        _ => Ok(WelcomeNavigationAction::Stay),
    }
}

fn draw_welcome_ui(frame: &mut Frame, app: &WelcomeApp) {
    let area = frame.area();
    let accent = Color::Cyan;
    let muted = Color::DarkGray;

    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header
            Constraint::Min(0),    // Body
            Constraint::Length(1), // Footer
        ])
        .split(area);

    let header = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(Line::from(vec![
            Span::styled(
                " petiglyph welcome ",
                Style::default().fg(accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!(" v{} ", CLI_VERSION), Style::default().fg(muted)),
        ]));
    frame.render_widget(header, root[0]);

    let body = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(8),  // intro
            Constraint::Length(11), // projects
            Constraint::Min(0),     // installed fonts
        ])
        .split(root[1]);

    let intro_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(muted))
        .title(Span::styled(
            " Workspace Scan ",
            Style::default().fg(accent),
        ));

    let intro_lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::raw("  "),
            Span::styled("Current directory: ", Style::default().fg(muted)),
            Span::raw(app.cwd.display().to_string()),
        ]),
        Line::from(vec![
            Span::raw("  "),
            Span::styled("Projects scope: ", Style::default().fg(muted)),
            Span::raw("current directory + one level below"),
        ]),
        Line::from(vec![
            Span::raw("  "),
            Span::styled("If no project exists: ", Style::default().fg(muted)),
            Span::raw("use the input + Create button to make one in current directory"),
        ]),
    ];
    frame.render_widget(
        Paragraph::new(intro_lines)
            .block(intro_block)
            .wrap(Wrap { trim: false }),
        body[0],
    );

    let projects_border = if app.focus == WelcomeFocus::Projects && !app.projects.is_empty() {
        Style::default().fg(accent)
    } else {
        Style::default().fg(muted)
    };
    let projects_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(projects_border)
        .title(Span::styled(
            " Petiglyph Projects (Open with Enter) ",
            Style::default().fg(accent),
        ));

    let mut projects_text = vec![Line::from("")];
    if app.projects.is_empty() {
        projects_text.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(
                "No project detected in this scope.",
                Style::default().fg(Color::Yellow),
            ),
        ]));
    } else {
        for (idx, project) in app.projects.iter().enumerate() {
            let marker = if idx == app.selected_project {
                ">"
            } else {
                " "
            };
            projects_text.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    format!("{marker} "),
                    if idx == app.selected_project {
                        if app.focus == WelcomeFocus::Projects {
                            Style::default().fg(accent).add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(Color::LightCyan)
                        }
                    } else {
                        Style::default().fg(muted)
                    },
                ),
                Span::styled(&project.font_name, Style::default().fg(Color::White)),
                Span::styled("  ", Style::default().fg(muted)),
                Span::styled(
                    project.manifest_path.display().to_string(),
                    Style::default().fg(muted),
                ),
            ]));
        }
    }

    projects_text.push(Line::from(""));
    let input_style = if app.focus == WelcomeFocus::CreateInput {
        Style::default()
            .fg(Color::Black)
            .bg(accent)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White).bg(Color::DarkGray)
    };
    let button_style = if app.focus == WelcomeFocus::CreateButton {
        Style::default()
            .fg(Color::Black)
            .bg(accent)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White).bg(Color::DarkGray)
    };
    let input_value = format_welcome_input_field(
        &app.create_input,
        app.focus == WelcomeFocus::CreateInput,
        WELCOME_INPUT_WIDTH,
    );
    projects_text.push(Line::from(vec![
        Span::raw("  "),
        Span::styled("New project: ", Style::default().fg(muted)),
        Span::styled(input_value, input_style),
        Span::raw(" "),
        Span::styled(" Create ", button_style),
    ]));

    frame.render_widget(
        Paragraph::new(projects_text)
            .block(projects_block)
            .wrap(Wrap { trim: false }),
        body[1],
    );

    let fonts_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(muted))
        .title(Span::styled(
            " Installed Petiglyph Fonts (sample first glyphs) ",
            Style::default().fg(accent),
        ));

    let mut fonts_text = vec![Line::from("")];
    if app.installed_fonts.is_empty() {
        fonts_text.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(
                "No installed petiglyph TTF fonts found.",
                Style::default().fg(muted),
            ),
        ]));
    } else {
        for font in &app.installed_fonts {
            let sample = if font.sample.is_empty() {
                "[sample unavailable]".to_string()
            } else if font.truncated {
                format!("{}...", font.sample)
            } else {
                font.sample.clone()
            };
            fonts_text.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(&font.file_name, Style::default().fg(Color::White)),
            ]));
            fonts_text.push(Line::from(vec![
                Span::raw("    "),
                Span::styled("path: ", Style::default().fg(muted)),
                Span::raw(font.path.display().to_string()),
            ]));
            fonts_text.push(Line::from(vec![
                Span::raw("    "),
                Span::styled("sample: ", Style::default().fg(muted)),
                Span::styled(sample, Style::default().fg(accent)),
            ]));
        }
    }

    frame.render_widget(
        Paragraph::new(fonts_text)
            .block(fonts_block)
            .wrap(Wrap { trim: false }),
        body[2],
    );

    let mut footer_spans = vec![
        Span::styled(" q/esc ", Style::default().fg(accent)),
        Span::raw("quit  "),
        Span::styled(" R ", Style::default().fg(accent)),
        Span::raw("rescan  "),
        Span::styled(" tab ", Style::default().fg(accent)),
        Span::raw("change focus  "),
        Span::styled(" ↑/↓ ", Style::default().fg(accent)),
        Span::raw("move in project list  "),
        Span::styled(" ←/→ ", Style::default().fg(accent)),
        Span::raw("input/button focus  "),
        Span::styled(" Enter ", Style::default().fg(accent)),
        Span::raw("activate selected control  "),
    ];
    footer_spans.push(Span::styled(" Backspace ", Style::default().fg(accent)));
    footer_spans.push(Span::raw("edit input  "));

    if let Some(status) = &app.status {
        footer_spans.push(Span::styled(" | ", Style::default().fg(muted)));
        footer_spans.push(Span::styled(
            status.clone(),
            Style::default().fg(Color::Yellow),
        ));
    }

    let footer = Paragraph::new(Line::from(footer_spans))
        .alignment(Alignment::Center)
        .style(Style::default().fg(muted));
    frame.render_widget(footer, root[2]);
}

impl App {
    #[cfg(test)]
    pub(crate) fn new(manifest_path: PathBuf, config: RuntimeConfig) -> Self {
        Self::new_with_overrides(manifest_path, config, TuiLaunchOverrides::default(), None)
    }

    pub(crate) fn new_with_overrides(
        manifest_path: PathBuf,
        config: RuntimeConfig,
        launch_overrides: TuiLaunchOverrides,
        welcome_root: Option<PathBuf>,
    ) -> Self {
        let project_dir = manifest_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        let (last_build, last_sample) = cached_build_state(&config);
        let installed_font_path = cached_installed_font_path(&manifest_path, &config.font_name);
        Self {
            manifest_path,
            project_dir,
            config,
            selected: 0,
            glyphs: Vec::new(),
            quit: false,
            status: None,
            view: AppView::Home,
            last_build,
            last_sample,
            installed_font_path,
            selected_font_action: FontAction::Build,
            launch_overrides,
            welcome_root,
            navigation_action: None,
            install_task: None,
        }
    }

    fn reload_config(&mut self) -> Result<()> {
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
                "add image files to {} and press R to rescan",
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

    fn build_project(&mut self) -> Result<()> {
        self.reload_config()?;
        if self.config.glyph_size == 0 {
            bail!("glyph_size must be > 0");
        }

        let summary = build_outputs(&self.config)?;
        let sample = fs::read_to_string(&summary.sample_path)
            .with_context(|| format!("failed to read {}", summary.sample_path.display()))?;

        self.last_sample = Some(sample.trim_end().to_string());
        self.last_build = Some(summary.clone());
        self.status = Some(format!(
            "build complete: {} glyph{} into {}",
            summary.glyph_count,
            if summary.glyph_count == 1 { "" } else { "s" },
            summary.out_dir().display()
        ));
        self.view = AppView::Font;
        Ok(())
    }

    fn start_install_font(&mut self) {
        if self.install_task.is_some() {
            self.status = Some("install already in progress".to_string());
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
            receiver,
            spinner_index: 0,
        });
        self.view = AppView::Font;
        self.status = None;
    }

    fn poll_install_task(&mut self) {
        let mut task_result = None;
        let mut disconnected = false;

        if let Some(task) = self.install_task.as_mut() {
            task.spinner_index = (task.spinner_index + 1) % INSTALL_SPINNER_FRAMES.len();
            match task.receiver.try_recv() {
                Ok(result) => task_result = Some(result),
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => disconnected = true,
            }
        }

        if disconnected {
            self.install_task = None;
            self.status = Some("install task terminated unexpectedly".to_string());
            return;
        }

        let Some(result) = task_result else {
            return;
        };

        self.install_task = None;
        match result {
            Ok(output) => {
                self.last_build = Some(output.summary);
                self.last_sample = output.sample;
                self.installed_font_path = Some(output.installed_path.clone());
                self.status = Some(format!(
                    "installed font to {}",
                    output.installed_path.display()
                ));
                self.view = AppView::Font;
            }
            Err(err) => {
                self.status = Some(err);
            }
        }
    }

    fn install_spinner_frame(&self) -> Option<&'static str> {
        self.install_task
            .as_ref()
            .map(|task| INSTALL_SPINNER_FRAMES[task.spinner_index % INSTALL_SPINNER_FRAMES.len()])
    }

    fn install_in_progress(&self) -> bool {
        self.install_task.is_some()
    }

    fn sample_string(&self) -> String {
        if let Some(sample) = &self.last_sample {
            sample.clone()
        } else {
            glyph_sample_string(self.config.codepoint_start, self.glyphs.len())
        }
    }

    fn request_welcome_navigation(&mut self) -> Result<()> {
        let Some(root) = self.welcome_root.clone() else {
            self.status =
                Some("welcome chooser is available only in multi-project sessions".to_string());
            return Ok(());
        };

        let project_count = discover_project_manifests(&root)?.len();
        if project_count > 1 {
            self.navigation_action = Some(NavigationAction::OpenWelcome(root));
            self.quit = true;
        } else {
            self.status =
                Some("welcome chooser requires at least two projects in scope".to_string());
        }
        Ok(())
    }
}

impl BuildSummary {
    fn out_dir(&self) -> &Path {
        self.ttf_path.parent().unwrap_or_else(|| Path::new("."))
    }
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

    let summary = build_outputs(&config)?;
    let sample = fs::read_to_string(&summary.sample_path)
        .with_context(|| format!("failed to read {}", summary.sample_path.display()))?;
    let installed = install_built_font(
        &manifest_path,
        &config.font_name,
        &summary.ttf_path,
        summary.glyph_count,
    )?;
    let sample = sample.trim_end().to_string();
    let sample = if sample.is_empty() {
        None
    } else {
        Some(sample)
    };

    Ok(InstallTaskOutput {
        summary,
        sample,
        installed_path: installed.install_path,
    })
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
    if let Ok(path) =
        expected_install_ttf_path_for_mode(manifest_path, font_name, FontInstallNameMode::Plain)
    {
        candidates.push(path);
    }
    if let Ok(path) = expected_install_ttf_path_for_mode(
        manifest_path,
        font_name,
        FontInstallNameMode::ProjectPrefixed,
    ) && !candidates.contains(&path)
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

pub(crate) fn handle_key(app: &mut App, code: KeyCode) -> Result<()> {
    match code {
        KeyCode::Esc | KeyCode::Char('q') => app.quit = true,
        KeyCode::Char('1') => app.view = AppView::Home,
        KeyCode::Char('2') => app.view = AppView::Glyphs,
        KeyCode::Char('3') => app.view = AppView::Font,
        KeyCode::Tab => {
            app.view = match app.view {
                AppView::Home => AppView::Glyphs,
                AppView::Glyphs => AppView::Font,
                AppView::Font => AppView::Home,
            }
        }
        KeyCode::Char('R') => {
            app.reload_glyphs()?;
            app.view = if app.glyphs.is_empty() {
                AppView::Home
            } else {
                AppView::Glyphs
            };
        }
        KeyCode::Char('b') => {
            trigger_build_action(app)?;
        }
        KeyCode::Char('i') => {
            trigger_install_action(app);
        }
        KeyCode::Char('w') => {
            app.request_welcome_navigation()?;
        }
        KeyCode::Down => {
            if app.view == AppView::Font {
                app.selected_font_action = app.selected_font_action.next();
            } else if app.view == AppView::Glyphs {
                app.selected = (app.selected + 1).min(app.glyphs.len().saturating_sub(1));
            }
        }
        KeyCode::Char('j') => {
            if app.view == AppView::Glyphs {
                app.selected = (app.selected + 1).min(app.glyphs.len().saturating_sub(1));
            }
        }
        KeyCode::Up => {
            if app.view == AppView::Font {
                app.selected_font_action = app.selected_font_action.prev();
            } else if app.view == AppView::Glyphs {
                app.selected = app.selected.saturating_sub(1);
            }
        }
        KeyCode::Char('k') => {
            if app.view == AppView::Glyphs {
                app.selected = app.selected.saturating_sub(1);
            }
        }
        KeyCode::Right => {
            if app.view == AppView::Font {
                app.selected_font_action = app.selected_font_action.next();
            } else if app.view == AppView::Glyphs
                && let Some(glyph) = selected_glyph(app)
            {
                let next = glyph.working_threshold.saturating_add(1);
                set_selected_threshold(app, next);
            }
        }
        KeyCode::Char('l') | KeyCode::Char('+') | KeyCode::Char('=') => {
            if app.view == AppView::Glyphs
                && let Some(glyph) = selected_glyph(app)
            {
                let next = glyph.working_threshold.saturating_add(1);
                set_selected_threshold(app, next);
            }
        }
        KeyCode::Left => {
            if app.view == AppView::Font {
                app.selected_font_action = app.selected_font_action.prev();
            } else if app.view == AppView::Glyphs
                && let Some(glyph) = selected_glyph(app)
            {
                let next = glyph.working_threshold.saturating_sub(1);
                set_selected_threshold(app, next);
            }
        }
        KeyCode::Char('h') | KeyCode::Char('-') => {
            if app.view == AppView::Glyphs
                && let Some(glyph) = selected_glyph(app)
            {
                let next = glyph.working_threshold.saturating_sub(1);
                set_selected_threshold(app, next);
            }
        }
        KeyCode::Enter => {
            if app.view == AppView::Font {
                match app.selected_font_action {
                    FontAction::Build => trigger_build_action(app)?,
                    FontAction::Install => trigger_install_action(app),
                }
            }
        }
        KeyCode::PageUp => {
            if app.view == AppView::Glyphs
                && let Some(glyph) = selected_glyph(app)
            {
                let next = glyph.working_threshold.saturating_add(10);
                set_selected_threshold(app, next);
            }
        }
        KeyCode::PageDown => {
            if app.view == AppView::Glyphs
                && let Some(glyph) = selected_glyph(app)
            {
                let next = glyph.working_threshold.saturating_sub(10);
                set_selected_threshold(app, next);
            }
        }
        KeyCode::Char('r') => {
            if app.view == AppView::Glyphs {
                remove_selected_threshold_override(app);
            }
        }
        _ => {}
    }
    Ok(())
}

fn trigger_build_action(app: &mut App) -> Result<()> {
    if app.install_in_progress() {
        app.status = Some("install is in progress; wait for it to finish".to_string());
        return Ok(());
    }
    app.build_project()
}

fn trigger_install_action(app: &mut App) {
    app.start_install_font();
}

fn draw_ui(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let accent = Color::Cyan;
    let muted = Color::DarkGray;

    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header
            Constraint::Min(0),    // Body
            Constraint::Length(1), // Footer keys
        ])
        .split(area);

    // Header
    let titles = [" 1 Home ", " 2 Glyphs ", " 3 Font "];
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
            AppView::Home => 0,
            AppView::Glyphs => 1,
            AppView::Font => 2,
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
        AppView::Home => draw_home_view(frame, app, body_area, accent, muted),
        AppView::Glyphs => draw_glyphs_view(frame, app, body_area, accent, muted),
        AppView::Font => draw_font_view(frame, app, body_area, accent, muted),
    }

    // Footer
    let mut footer_spans = vec![
        Span::styled(" q/esc ", Style::default().fg(accent)),
        Span::raw("quit  "),
        Span::styled(" tab ", Style::default().fg(accent)),
        Span::raw("next panel  "),
        Span::styled(" 1-3 ", Style::default().fg(accent)),
        Span::raw("jump panel  "),
        Span::styled(" R ", Style::default().fg(accent)),
        Span::raw("rescan  "),
        Span::styled(" b ", Style::default().fg(accent)),
        Span::raw("build  "),
        Span::styled(" i ", Style::default().fg(accent)),
        Span::raw("install  "),
    ];

    if app.welcome_root.is_some() {
        footer_spans.push(Span::styled(" w ", Style::default().fg(accent)));
        footer_spans.push(Span::raw("back to welcome chooser  "));
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
    if app.view == AppView::Font {
        footer_spans.extend(vec![
            Span::styled(" \u{2190}/\u{2192} ", Style::default().fg(accent)),
            Span::raw("select action  "),
            Span::styled(" Enter ", Style::default().fg(accent)),
            Span::raw("run selected  "),
        ]);
    }

    if let Some(status) = &app.status {
        footer_spans.push(Span::styled(" | ", Style::default().fg(muted)));
        footer_spans.push(Span::styled(
            status.clone(),
            Style::default().fg(Color::LightRed),
        ));
    }

    if let Some(spinner) = app.install_spinner_frame() {
        footer_spans.push(Span::styled(" | ", Style::default().fg(muted)));
        footer_spans.push(Span::styled(
            format!("{spinner} installing font..."),
            Style::default().fg(Color::Yellow),
        ));
    }

    let footer = Paragraph::new(Line::from(footer_spans))
        .alignment(Alignment::Center)
        .style(Style::default().fg(muted));
    frame.render_widget(footer, root[2]);
}

fn draw_home_view(
    frame: &mut Frame,
    app: &App,
    area: ratatui::layout::Rect,
    accent: Color,
    muted: Color,
) {
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
        .split(area);

    let overview_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(muted))
        .title(Span::styled(
            " Project Overview ",
            Style::default().fg(accent),
        ));

    let overview = vec![
        Line::from(""),
        Line::from(vec![
            Span::raw("  "),
            Span::styled("Font Name: ", Style::default().fg(muted)),
            Span::styled(
                &app.config.font_name,
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::raw("  "),
            Span::styled("Manifest:  ", Style::default().fg(muted)),
            Span::raw(app.manifest_path.display().to_string()),
        ]),
        Line::from(vec![
            Span::raw("  "),
            Span::styled("Icons Dir: ", Style::default().fg(muted)),
            Span::raw(app.config.input_dir.display().to_string()),
        ]),
        Line::from(vec![
            Span::raw("  "),
            Span::styled("Build Dir: ", Style::default().fg(muted)),
            Span::raw(app.config.out_dir.display().to_string()),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::raw("  "),
            Span::styled("Glyphs:    ", Style::default().fg(muted)),
            Span::styled(app.glyphs.len().to_string(), Style::default().fg(accent)),
        ]),
        Line::from(vec![
            Span::raw("  "),
            Span::styled("Size:      ", Style::default().fg(muted)),
            Span::raw(format!("{}px", app.config.glyph_size)),
        ]),
        Line::from(vec![
            Span::raw("  "),
            Span::styled("Threshold: ", Style::default().fg(muted)),
            Span::raw(app.config.base_threshold.to_string()),
        ]),
        Line::from(vec![
            Span::raw("  "),
            Span::styled("Codepoint: ", Style::default().fg(muted)),
            Span::raw(format!("U+{:04X}", app.config.codepoint_start)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::raw("  "),
            Span::styled(
                "Workflow:",
                Style::default().fg(accent).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::raw("  "),
            Span::styled("1. ", Style::default().fg(muted)),
            Span::raw("Add/update images in icons/"),
        ]),
        Line::from(vec![
            Span::raw("  "),
            Span::styled("2. ", Style::default().fg(muted)),
            Span::raw("Press R to rescan, tune thresholds in Glyphs view"),
        ]),
        Line::from(vec![
            Span::raw("  "),
            Span::styled("3. ", Style::default().fg(muted)),
            Span::raw("Press b to build TTF/BDF and mappings"),
        ]),
        Line::from(vec![
            Span::raw("  "),
            Span::styled("4. ", Style::default().fg(muted)),
            Span::raw("Press i to install the font to your system"),
        ]),
    ];

    let p = Paragraph::new(overview)
        .block(overview_block)
        .wrap(Wrap { trim: false });
    frame.render_widget(p, sections[0]);

    let sample_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(muted))
        .title(Span::styled(
            " Glyph Clipboard Sample ",
            Style::default().fg(accent),
        ));

    let sample_area = sample_block.inner(sections[1]);
    let max_chars = sample_area.width.saturating_sub(4) as usize;
    let sample = app.sample_string();
    let source_note = if app.last_sample.is_some() {
        "Detected from build/glyph-sample.txt"
    } else {
        "Generated from codepoint start + current glyph count"
    };

    let mut sample_text = vec![
        Line::from(""),
        Line::from(vec![
            Span::raw("  "),
            Span::styled("Source: ", Style::default().fg(muted)),
            Span::raw(source_note),
        ]),
        Line::from(vec![
            Span::raw("  "),
            Span::styled("Copy line: ", Style::default().fg(muted)),
            Span::raw("select this exact sequence"),
        ]),
        Line::from(""),
    ];

    if sample.is_empty() {
        sample_text.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(
                "No glyph sample available yet. Add icons and build first.",
                Style::default().fg(Color::Yellow),
            ),
        ]));
    } else {
        for line in wrap_sample_for_display(&sample, max_chars) {
            sample_text.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    line,
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
            ]));
        }

        sample_text.push(Line::from(""));
        sample_text.push(Line::from(vec![
            Span::raw("  "),
            Span::styled("Readable (spaced):", Style::default().fg(muted)),
        ]));

        let spaced = spaced_sample(&sample);
        for line in wrap_sample_for_display(&spaced, max_chars) {
            sample_text.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    line,
                    Style::default().fg(accent).add_modifier(Modifier::BOLD),
                ),
            ]));
        }
    }

    let sample_paragraph = Paragraph::new(sample_text)
        .block(sample_block)
        .wrap(Wrap { trim: false });
    frame.render_widget(sample_paragraph, sections[1]);
}

fn draw_glyphs_view(
    frame: &mut Frame,
    app: &App,
    area: ratatui::layout::Rect,
    accent: Color,
    muted: Color,
) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
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
            Line::from("  Add images and press R to rescan."),
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

fn draw_font_view(
    frame: &mut Frame,
    app: &App,
    area: ratatui::layout::Rect,
    accent: Color,
    muted: Color,
) {
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(10), Constraint::Min(0)])
        .split(area);

    let status_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(muted))
        .title(Span::styled(" Font Status ", Style::default().fg(accent)));

    let sample = app.sample_string();
    let build_summary = app.last_build.as_ref();

    let ok_style = Style::default().fg(Color::Green);
    let missing_style = Style::default().fg(Color::Red);

    let ttf_status = match build_summary {
        Some(s) => Span::styled(s.ttf_path.display().to_string(), ok_style),
        None => Span::styled(
            format!(
                "Not built yet (target: {})",
                expected_ttf_path(&app.config).display()
            ),
            missing_style,
        ),
    };
    let bdf_status = match build_summary {
        Some(s) => Span::styled(s.bdf_path.display().to_string(), ok_style),
        None => Span::styled(
            format!(
                "Not built yet (target: {})",
                expected_bdf_path(&app.config).display()
            ),
            missing_style,
        ),
    };
    let installed_status = match &app.installed_font_path {
        Some(p) => Span::styled(p.display().to_string(), ok_style),
        None => {
            let target = expected_install_ttf_path(&app.manifest_path, &app.config.font_name)
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| "~/.local/share/fonts/petiglyph/<font>.ttf".to_string());
            Span::styled(format!("Not installed (target: {})", target), missing_style)
        }
    };

    let text = vec![
        Line::from(""),
        Line::from(vec![
            Span::raw("  "),
            Span::styled("Sample string:", Style::default().fg(accent)),
        ]),
        Line::from(vec![Span::raw("  "), Span::raw(sample)]),
        Line::from(""),
        Line::from(vec![
            Span::raw("  "),
            Span::styled("TTF Output: ", Style::default().fg(muted)),
            ttf_status,
        ]),
        Line::from(vec![
            Span::raw("  "),
            Span::styled("BDF Output: ", Style::default().fg(muted)),
            bdf_status,
        ]),
        Line::from(vec![
            Span::raw("  "),
            Span::styled("System:     ", Style::default().fg(muted)),
            installed_status,
        ]),
    ];

    let actions_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(muted))
        .title(Span::styled(" Actions ", Style::default().fg(accent)));

    let selected_button_style = Style::default()
        .fg(Color::Black)
        .bg(accent)
        .add_modifier(Modifier::BOLD);
    let idle_button_style = Style::default()
        .fg(Color::White)
        .bg(Color::DarkGray)
        .add_modifier(Modifier::BOLD);

    let build_button_style = if app.selected_font_action == FontAction::Build {
        selected_button_style
    } else {
        idle_button_style
    };

    let install_label = if let Some(spinner) = app.install_spinner_frame() {
        format!(" {spinner} Installing... ")
    } else {
        " Install (i) ".to_string()
    };

    let install_button_style = if app.install_in_progress() {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else if app.selected_font_action == FontAction::Install {
        selected_button_style
    } else {
        idle_button_style
    };

    let (desc_label, desc_text) = match app.selected_font_action {
        FontAction::Build => (
            "Build:",
            " generate TTF/BDF, glyph map, and sample in build/",
        ),
        FontAction::Install => ("Install:", " copy built TTF to your user font directory"),
    };
    let desc_style = Style::default().fg(accent).add_modifier(Modifier::BOLD);

    let action_lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::raw("  "),
            Span::styled(
                "Use \u{2190}/\u{2192} to select, Enter to run. Shortcuts: b / i",
                Style::default().fg(muted),
            ),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::raw("  "),
            Span::styled(" Build (b) ", build_button_style),
            Span::raw("   "),
            Span::styled(install_label, install_button_style),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::raw("  "),
            Span::styled(desc_label, desc_style),
            Span::raw(desc_text),
        ]),
    ];

    let actions_panel = Paragraph::new(action_lines)
        .block(actions_block)
        .wrap(Wrap { trim: false });
    frame.render_widget(actions_panel, sections[0]);

    let status_panel = Paragraph::new(text)
        .block(status_block)
        .wrap(Wrap { trim: false });
    frame.render_widget(status_panel, sections[1]);
}

fn preview_lines(
    glyph: &PreprocessedGlyph,
    threshold: u8,
    max_w: u16,
    max_h: u16,
) -> Vec<Line<'static>> {
    let src = glyph.size as usize;
    if src == 0 || max_w == 0 || max_h == 0 {
        return vec![Line::from("  [Preview too small]")];
    }

    let out_w = usize::max(1, usize::min(src, max_w as usize));
    let out_h = usize::max(1, usize::min(src, max_h as usize));

    let mut lines = Vec::with_capacity(out_h);

    for oy in 0..out_h {
        let sy = oy * src / out_h;
        let mut row = String::with_capacity(out_w * 2 + 4);
        row.push_str("    ");
        for ox in 0..out_w {
            let sx = ox * src / out_w;
            let idx = sy * src + sx;
            let on = glyph.coverage[idx] >= threshold;
            row.push_str(if on { "██" } else { "  " });
        }
        lines.push(Line::from(row));
    }

    lines
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
