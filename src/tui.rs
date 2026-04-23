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
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;
use walkdir::WalkDir;

use crate::build::{
    BuildSummary, MappingEntry, PreprocessedGlyph, build_outputs, expected_bdf_path,
    expected_ttf_path, glyph_sample_string, is_supported_source, preprocess_sources,
};
use crate::install::{install_built_font, install_dir_for_manifest};
use crate::project::{RuntimeConfig, load_runtime_config, read_manifest, write_manifest};

const CLI_VERSION: &str = env!("CARGO_PKG_VERSION");

pub(crate) fn tui(
    manifest_path: PathBuf,
    input_override: Option<PathBuf>,
    threshold_override: Option<u8>,
    glyph_size_override: Option<u32>,
    codepoint_start_override: Option<String>,
) -> Result<()> {
    let config = load_runtime_config(
        &manifest_path,
        input_override,
        None,
        threshold_override,
        glyph_size_override,
        codepoint_start_override,
    )?;

    let mut app = App::new(manifest_path, config);
    app.reload_glyphs()?;

    let mut session = TerminalSession::start()?;
    while !app.quit {
        session.terminal.draw(|frame| draw_ui(frame, &app))?;

        if event::poll(Duration::from_millis(120))?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
            && let Err(err) = handle_key(&mut app, key.code)
        {
            app.status = Some(err.to_string());
        }
    }

    println!("tui session closed for {}", app.project_dir.display());
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AppView {
    Home,
    Glyphs,
    Font,
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

impl App {
    pub(crate) fn new(manifest_path: PathBuf, config: RuntimeConfig) -> Self {
        let project_dir = manifest_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        let (last_build, last_sample) = cached_build_state(&config);
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
            installed_font_path: None,
        }
    }

    fn reload_config(&mut self) -> Result<()> {
        self.config = load_runtime_config(&self.manifest_path, None, None, None, None, None)?;
        let (last_build, last_sample) = cached_build_state(&self.config);
        self.last_build = last_build;
        self.last_sample = last_sample;
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

    fn install_font(&mut self) -> Result<()> {
        self.build_project()?;
        let summary = self
            .last_build
            .as_ref()
            .context("build summary missing after build")?;
        let installed = install_built_font(
            &self.manifest_path,
            &self.config.font_name,
            &summary.ttf_path,
            summary.glyph_count,
        )?;
        self.installed_font_path = Some(installed.install_path.clone());
        self.status = Some(format!(
            "installed font to {}",
            installed.install_path.display()
        ));
        Ok(())
    }

    fn sample_string(&self) -> String {
        if let Some(sample) = &self.last_sample {
            sample.clone()
        } else {
            glyph_sample_string(self.config.codepoint_start, self.glyphs.len())
        }
    }
}

impl BuildSummary {
    fn out_dir(&self) -> &Path {
        self.ttf_path.parent().unwrap_or_else(|| Path::new("."))
    }
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
            app.build_project()?;
        }
        KeyCode::Char('i') => {
            app.install_font()?;
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if app.view == AppView::Glyphs {
                app.selected = (app.selected + 1).min(app.glyphs.len().saturating_sub(1));
            }
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if app.view == AppView::Glyphs {
                app.selected = app.selected.saturating_sub(1);
            }
        }
        KeyCode::Right | KeyCode::Char('l') | KeyCode::Char('+') | KeyCode::Char('=') => {
            if app.view == AppView::Glyphs
                && let Some(glyph) = selected_glyph(app)
            {
                let next = glyph.working_threshold.saturating_add(1);
                set_selected_threshold(app, next);
            }
        }
        KeyCode::Left | KeyCode::Char('h') | KeyCode::Char('-') => {
            if app.view == AppView::Glyphs
                && let Some(glyph) = selected_glyph(app)
            {
                let next = glyph.working_threshold.saturating_sub(1);
                set_selected_threshold(app, next);
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
    let titles = vec![" 1 Home ", " 2 Glyphs ", " 3 Font "];
    let tabs = Tabs::new(titles.iter().copied().map(Line::from).collect::<Vec<_>>())
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
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(muted))
        .title(Span::styled(
            " Project Overview ",
            Style::default().fg(accent),
        ));

    let text = vec![
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

    let p = Paragraph::new(text).block(block).wrap(Wrap { trim: false });
    frame.render_widget(p, area);
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
    let block = Block::default()
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
            let target = install_dir_for_manifest(&app.manifest_path)
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| "~/.local/share/fonts/petiglyph/<project>".to_string());
            Span::styled(
                format!("Not installed in this session (target: {})", target),
                missing_style,
            )
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
        Line::from(""),
        Line::from(vec![
            Span::raw("  "),
            Span::styled("Actions:", Style::default().fg(accent)),
        ]),
        Line::from(vec![
            Span::raw("  "),
            Span::styled(
                "Press 'b' to build the font files.",
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(vec![
            Span::raw("  "),
            Span::styled(
                "Press 'i' to install the built TTF to your OS.",
                Style::default().fg(Color::White),
            ),
        ]),
    ];

    let p = Paragraph::new(text).block(block).wrap(Wrap { trim: false });
    frame.render_widget(p, area);
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
