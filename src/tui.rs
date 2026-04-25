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
use std::time::{Duration, Instant};
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
const SWITCH_NOTICE_MS: u64 = 2500;

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
    file_name: String,
    path: PathBuf,
    sample: String,
    truncated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WelcomeFocus {
    CreateInput,
    CreateButton,
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

    let mut session = TerminalSession::start()?;
    while !app.quit {
        app.poll_install_task();
        app.clear_expired_switch_notice();
        session.terminal.draw(|frame| draw_ui(frame, &app))?;

        if event::poll(Duration::from_millis(120))?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
            && let Err(err) = handle_key(&mut app, key.code)
        {
            app.status = Some(err.to_string());
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
    Font,
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
    pub(crate) workspace_root: PathBuf,
    pub(crate) projects: Vec<WelcomeProject>,
    pub(crate) active_project: Option<PathBuf>,
    pub(crate) create_input: String,
    pub(crate) welcome_focus: WelcomeFocus,
    pub(crate) welcome_input_editing: bool,
    installed_fonts: Vec<InstalledFontSample>,
    pub(crate) switch_notice: Option<ProjectSwitchNotice>,
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

fn handle_welcome_key(app: &mut App, code: KeyCode) -> Result<()> {
    match code {
        KeyCode::Esc => {
            if app.welcome_input_editing {
                app.welcome_input_editing = false;
                app.status = Some("stopped typing project name".to_string());
            } else {
                app.quit = true;
            }
        }
        KeyCode::Char('q') if !app.welcome_input_editing => {
            app.quit = true;
        }
        KeyCode::Char('R') if !app.welcome_input_editing => {
            app.refresh_workspace_discovery()?;
            if app.active_project.is_some() {
                app.reload_glyphs()?;
            }
        }
        KeyCode::Left | KeyCode::Up | KeyCode::Char('h') | KeyCode::Char('k')
            if !app.welcome_input_editing =>
        {
            app.welcome_focus = match app.welcome_focus {
                WelcomeFocus::CreateButton => WelcomeFocus::CreateInput,
                other => other,
            };
        }
        KeyCode::Right | KeyCode::Down | KeyCode::Char('l') | KeyCode::Char('j')
            if !app.welcome_input_editing =>
        {
            app.welcome_focus = match app.welcome_focus {
                WelcomeFocus::CreateInput => WelcomeFocus::CreateButton,
                other => other,
            };
        }
        KeyCode::Backspace => {
            if app.welcome_focus == WelcomeFocus::CreateInput && app.welcome_input_editing {
                app.create_input.pop();
            }
        }
        KeyCode::Char(ch)
            if app.welcome_focus == WelcomeFocus::CreateInput
                && app.welcome_input_editing
                && is_valid_project_name_char(ch) =>
        {
            app.create_input.push(ch);
        }
        KeyCode::Enter => match app.welcome_focus {
            WelcomeFocus::CreateInput => {
                if app.welcome_input_editing {
                    app.welcome_input_editing = false;
                    app.welcome_focus = WelcomeFocus::CreateButton;
                    app.status = Some("press Enter to create project".to_string());
                } else {
                    app.welcome_input_editing = true;
                    app.status = Some("typing project name (Enter/Esc to stop)".to_string());
                }
            }
            WelcomeFocus::CreateButton => {
                app.welcome_input_editing = false;
                app.submit_create()?;
            }
        },
        _ => {}
    }
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
            Constraint::Length(7),  // intro
            Constraint::Length(11), // projects
            Constraint::Min(0),     // installed fonts
        ])
        .split(area);

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
            Span::styled("Current folder: ", Style::default().fg(muted)),
            Span::raw(app.workspace_root.display().to_string()),
        ]),
        Line::from(vec![
            Span::raw("  "),
            Span::styled("Project scan: ", Style::default().fg(muted)),
            Span::raw("current folder + one level below"),
        ]),
        Line::from(vec![
            Span::raw("  "),
            Span::styled(
                "Use Home to review detected folders or create a project folder.",
                Style::default().fg(Color::White),
            ),
        ]),
    ];
    frame.render_widget(
        Paragraph::new(intro_lines)
            .block(intro_block)
            .wrap(Wrap { trim: false }),
        body[0],
    );

    let projects_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(muted))
        .title(Span::styled(
            " Petiglyph Projects ",
            Style::default().fg(accent),
        ));

    let mut projects_text = vec![Line::from("")];
    if app.projects.is_empty() {
        projects_text.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(
                "No project detected in this folder.",
                Style::default().fg(Color::Yellow),
            ),
        ]));
    } else {
        for project in &app.projects {
            let is_active = app
                .active_project
                .as_ref()
                .is_some_and(|active| active == &project.manifest_path);
            let marker = if is_active { "active" } else { "found " };
            projects_text.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    format!("[{marker}] "),
                    if is_active {
                        Style::default().fg(accent).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(muted)
                    },
                ),
                Span::styled(
                    &project.font_name,
                    if is_active {
                        Style::default()
                            .fg(Color::White)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::White)
                    },
                ),
                Span::styled("  ", Style::default().fg(muted)),
                Span::styled(
                    project.manifest_path.display().to_string(),
                    Style::default().fg(muted),
                ),
            ]));
        }
    }

    projects_text.push(Line::from(""));
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
    let input_value = format_welcome_input_field(
        &app.create_input,
        app.welcome_input_editing,
        WELCOME_INPUT_WIDTH,
    );
    let mut new_project_line = vec![
        Span::raw("  "),
        Span::styled("New project: ", Style::default().fg(muted)),
        Span::styled(input_value, input_style),
        Span::raw(" "),
        Span::styled(" Create ", button_style),
    ];
    if app.welcome_focus == WelcomeFocus::CreateInput {
        let hint = if app.welcome_input_editing {
            "  typing (Enter/Esc to stop)"
        } else {
            "  press Enter to type"
        };
        new_project_line.push(Span::styled(hint, Style::default().fg(muted)));
    }
    projects_text.push(Line::from(new_project_line));

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
                format!("{}...", spaced_sample(&font.sample))
            } else {
                spaced_sample(&font.sample)
            };
            fonts_text.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    &font.file_name,
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
            ]));
            fonts_text.push(Line::from(vec![
                Span::raw("    "),
                Span::styled("path: ", Style::default().fg(muted)),
                Span::raw(font.path.display().to_string()),
            ]));
            fonts_text.push(Line::from(vec![
                Span::raw("    "),
                Span::styled("sample:", Style::default().fg(muted)),
            ]));
            for line in wrap_sample_for_display(&sample, 48) {
                fonts_text.push(Line::from(vec![
                    Span::raw("    "),
                    Span::styled(
                        line,
                        Style::default().fg(accent).add_modifier(Modifier::BOLD),
                    ),
                ]));
            }
        }
    }

    frame.render_widget(
        Paragraph::new(fonts_text)
            .block(fonts_block)
            .wrap(Wrap { trim: false }),
        body[2],
    );
}

impl App {
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
            create_input: String::new(),
            welcome_focus: WelcomeFocus::CreateInput,
            welcome_input_editing: false,
            installed_fonts: Vec::new(),
            switch_notice: None,
            selected: 0,
            glyphs: Vec::new(),
            quit: false,
            status: None,
            view: AppView::Welcome,
            last_build: None,
            last_sample: None,
            installed_font_path: None,
            selected_font_action: FontAction::Build,
            launch_overrides,
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
            create_input: String::new(),
            welcome_focus: WelcomeFocus::CreateInput,
            welcome_input_editing: false,
            installed_fonts: Vec::new(),
            switch_notice: None,
            selected: 0,
            glyphs: Vec::new(),
            quit: false,
            status: None,
            view: AppView::Welcome,
            last_build,
            last_sample,
            installed_font_path,
            selected_font_action: FontAction::Build,
            launch_overrides,
            install_task: None,
        }
    }

    fn refresh_workspace_discovery(&mut self) -> Result<()> {
        self.projects = scan_projects_in_folder(&self.workspace_root)?;

        match scan_installed_petiglyph_fonts(&self.workspace_root) {
            Ok(fonts) => self.installed_fonts = fonts,
            Err(err) => {
                self.installed_fonts.clear();
                self.status = Some(format!("font scan warning: {err}"));
            }
        }

        if self.projects.is_empty() {
            self.welcome_focus = WelcomeFocus::CreateInput;
            self.welcome_input_editing = false;
            if self.active_project.is_none() {
                self.status = Some(format!(
                    "no petiglyph project in {}",
                    self.workspace_root.display()
                ));
            }
        }

        Ok(())
    }

    fn submit_create(&mut self) -> Result<()> {
        let project_name = self.create_input.trim().to_string();
        if project_name.is_empty() {
            self.status = Some("project name cannot be empty".to_string());
            self.welcome_focus = WelcomeFocus::CreateInput;
            self.welcome_input_editing = true;
            return Ok(());
        }

        if self.install_in_progress() {
            self.status =
                Some("install is in progress; wait before switching projects".to_string());
            return Ok(());
        }

        let manifest_path = create_project_in_dir(&self.workspace_root, &project_name)?;
        self.create_input.clear();
        self.welcome_input_editing = false;
        self.refresh_workspace_discovery()?;
        self.set_active_project(manifest_path)?;
        self.status = Some(format!("created and opened project `{project_name}`"));
        Ok(())
    }

    fn set_active_project(&mut self, manifest_path: PathBuf) -> Result<()> {
        if self.install_in_progress() {
            self.status =
                Some("install is in progress; wait before switching projects".to_string());
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
        if self.active_project.is_none() {
            self.status = Some(
                "create a project in Home or relaunch with --manifest before building".to_string(),
            );
            return Ok(());
        }

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
        if self.active_project.is_none() {
            self.status = Some(
                "create a project in Home or relaunch with --manifest before installing"
                    .to_string(),
            );
            return;
        }

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
        if self.active_project.is_none() {
            return String::new();
        }

        if let Some(sample) = &self.last_sample {
            sample.clone()
        } else {
            glyph_sample_string(self.config.codepoint_start, self.glyphs.len())
        }
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

pub(crate) fn handle_key(app: &mut App, code: KeyCode) -> Result<()> {
    let is_global_panel_jump = matches!(code, KeyCode::Tab | KeyCode::BackTab)
        || (matches!(code, KeyCode::Char('2') | KeyCode::Char('3')) && !app.welcome_input_editing);

    if app.view == AppView::Welcome && !is_global_panel_jump {
        return handle_welcome_key(app, code);
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
        KeyCode::Char('3') => {
            app.welcome_input_editing = false;
            app.view = AppView::Font;
        }
        KeyCode::Tab => {
            app.welcome_input_editing = false;
            app.view = match app.view {
                AppView::Welcome => AppView::Glyphs,
                AppView::Glyphs => AppView::Font,
                AppView::Font => AppView::Welcome,
            }
        }
        KeyCode::Char('R') => {
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
            trigger_install_action(app);
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
            } else if app.view == AppView::Glyphs {
                if let Some(glyph) = selected_glyph(app) {
                    let next = glyph.working_threshold.saturating_add(1);
                    set_selected_threshold(app, next);
                } else if app.active_project.is_none() {
                    set_selected_threshold(app, 0);
                }
            }
        }
        KeyCode::Char('l') | KeyCode::Char('+') | KeyCode::Char('=') => {
            if app.view == AppView::Glyphs {
                if let Some(glyph) = selected_glyph(app) {
                    let next = glyph.working_threshold.saturating_add(1);
                    set_selected_threshold(app, next);
                } else if app.active_project.is_none() {
                    set_selected_threshold(app, 0);
                }
            }
        }
        KeyCode::Left => {
            if app.view == AppView::Font {
                app.selected_font_action = app.selected_font_action.prev();
            } else if app.view == AppView::Glyphs {
                if let Some(glyph) = selected_glyph(app) {
                    let next = glyph.working_threshold.saturating_sub(1);
                    set_selected_threshold(app, next);
                } else if app.active_project.is_none() {
                    set_selected_threshold(app, 0);
                }
            }
        }
        KeyCode::Char('h') | KeyCode::Char('-') => {
            if app.view == AppView::Glyphs {
                if let Some(glyph) = selected_glyph(app) {
                    let next = glyph.working_threshold.saturating_sub(1);
                    set_selected_threshold(app, next);
                } else if app.active_project.is_none() {
                    set_selected_threshold(app, 0);
                }
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
            Constraint::Length(1), // Active project / switch notice
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
            AppView::Welcome => 0,
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

    let mut active_spans = vec![
        Span::styled(" Active: ", Style::default().fg(muted)),
        Span::styled(
            app.active_project_label(),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
    ];
    if let Some(notice) = &app.switch_notice {
        active_spans.push(Span::raw("  "));
        active_spans.push(Span::styled(
            format!(
                " Switched project: {} -> {} ",
                notice.from_label, notice.to_label
            ),
            Style::default()
                .fg(Color::Black)
                .bg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
    }
    frame.render_widget(
        Paragraph::new(Line::from(active_spans)).alignment(Alignment::Center),
        root[1],
    );

    // Body
    let body_area = root[2];

    match app.view {
        AppView::Welcome => draw_welcome_view(frame, app, body_area, accent, muted),
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

    if app.view == AppView::Welcome {
        let enter_help = if app.welcome_input_editing {
            "stop typing  "
        } else {
            "type/create  "
        };
        footer_spans.extend(vec![
            Span::styled(" \u{2190}/\u{2192} ", Style::default().fg(accent)),
            Span::raw("focus  "),
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
    frame.render_widget(footer, root[3]);
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
    if app.active_project.is_none() {
        draw_blocked_project_view(frame, area, " Font ", accent, muted);
        return;
    }

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
        Some(s) => Span::styled(format!("✓ installed: {}", s.ttf_path.display()), ok_style),
        None => Span::styled(
            format!(
                "Not built yet (target: {})",
                expected_ttf_path(&app.config).display()
            ),
            missing_style,
        ),
    };
    let bdf_status = match build_summary {
        Some(s) => Span::styled(format!("✓ installed: {}", s.bdf_path.display()), ok_style),
        None => Span::styled(
            format!(
                "Not built yet (target: {})",
                expected_bdf_path(&app.config).display()
            ),
            missing_style,
        ),
    };
    let installed_status = match &app.installed_font_path {
        Some(p) => Span::styled(format!("✓ installed: {}", p.display()), ok_style),
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
