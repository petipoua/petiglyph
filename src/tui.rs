use anyhow::{Context, Result, bail};
use crossterm::ExecutableCommand;
use crossterm::event::{
    self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyEventState, KeyModifiers, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
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
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs::{self, OpenOptions};
use std::hash::{Hash, Hasher};
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
    expected_ttf_path, is_supported_source, preprocess_sources_with_compositions,
};
use crate::glyph_debug;
use crate::install::{
    DEFAULT_INSTALL_NAME_MODE, FontInstallNameMode, effective_font_name,
    expected_install_ttf_path_for_mode, install_built_font, install_dir_for_manifest,
    installed_ttf_candidates_for_manifest_font, uninstall_installed_font_file,
};
use crate::project::{
    AnimationDef, AnimationType, BleedLevel, CompositionDef, RuntimeConfig, create_project_in_dir,
    delete_project_for_manifest, discover_project_manifests, format_codepoint, load_runtime_config,
    read_manifest, write_manifest,
};

const CLI_VERSION: &str = env!("CARGO_PKG_VERSION");
const INSTALL_SPINNER_FRAMES: [&str; 4] = ["-", "\\", "|", "/"];
const UNINSTALL_SPINNER_FRAMES: [&str; 4] = ["/", "|", "\\", "-"];
const ANIMATION_IMPORT_SPINNER_FRAMES: [&str; 4] = ["-", "\\", "|", "/"];
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
const DELETE_CONFIRM_DELETE_INDEX: usize = 1;
const HTY_FULL_REPAINT_ENV: &str = "PETIGLYPH_TUI_HTY_FULL_REPAINT";
const GLYPH_SOURCE_COUNT_REFRESH_MS: u64 = 300;
const INSTALL_METADATA_PREFIX: &str = ".petiglyph-install-";
const INSTALL_METADATA_SUFFIX: &str = ".json";
const INSTALLED_FONT_BLOCK_REFERENCE_SEPARATOR: &str = "  │  ";
const DEBUG_LOG_VISIBLE_LINES: usize = 6;
static HTY_FULL_REPAINT_ENABLED: OnceLock<bool> = OnceLock::new();

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
    pub(crate) blocks: Vec<String>,
    pub(crate) animation_rows: Vec<String>,
    pub(crate) animation_previews: Vec<InstalledFontAnimationPreview>,
    pub(crate) animation_exports: Vec<String>,
    pub(crate) truncated: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct InstalledFontAnimationPreview {
    pub(crate) fps: u8,
    pub(crate) frame_blocks: Vec<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct InstalledFontMetadataRecord {
    manifest_path: String,
    installed_ttf: String,
    #[serde(default)]
    animation_snapshots: Vec<InstalledAnimationSnapshotRecord>,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct InstalledAnimationSnapshotRecord {
    name: String,
    #[serde(rename = "type")]
    animation_type: AnimationType,
    fps: u8,
    #[serde(default)]
    frame_blocks: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WelcomeFocus {
    VerbosePathsToggle,
    ProjectList,
    CreateInput,
    BuildButton,
    InstallButton,
    DeleteProjectButton,
    InstalledFontList,
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
        app.poll_animation_import_task();
        app.update_animation_preview();
        app.clear_expired_switch_notice();
        app.refresh_live_glyph_source_count();
        app.refresh_pipeline_debug_log();
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
pub(crate) enum GlyphsFocus {
    List,
    GridButton,
    AnimateButton,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GridConfigFocus {
    Rows,
    Cols,
    HorizontalBleed,
    VerticalBleed,
    Create,
}

#[derive(Debug, Clone)]
pub(crate) struct GridConfig {
    pub(crate) source_key: String,
    pub(crate) rows: u32,
    pub(crate) cols: u32,
    pub(crate) horizontal_bleed: BleedLevel,
    pub(crate) vertical_bleed: BleedLevel,
    pub(crate) focus: GridConfigFocus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AnimationTypeChoiceFocus {
    Standard,
    Grid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AnimationConfigFocus {
    Name,
    Fps,
    Rows,
    Cols,
    HorizontalBleed,
    VerticalBleed,
    Create,
}

#[derive(Debug, Clone)]
struct AnimationConfig {
    selected_frames: Vec<String>,
    name_input: Input,
    animation_type: AnimationType,
    fps: u8,
    rows: u32,
    cols: u32,
    horizontal_bleed: BleedLevel,
    vertical_bleed: BleedLevel,
    focus: AnimationConfigFocus,
}

#[derive(Debug, Clone)]
struct AnimationPreview {
    animation_name: String,
    frame_index: usize,
    last_frame_at: Instant,
}

#[derive(Debug, Clone)]
enum GlyphToolMode {
    None,
    ChooseAnimationType { focus: AnimationTypeChoiceFocus },
    ImportAnimationFrames,
    SelectAnimationFrames(AnimationType),
    ConfigureAnimation(AnimationConfig),
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
    pub(crate) verbose_paths: bool,
    pub(crate) installed_fonts: Vec<InstalledFontSample>,
    installed_animation_started_at: Instant,
    pub(crate) selected_installed_font: usize,
    pub(crate) selected_installed_font_sub_index: usize,
    pub(crate) installed_font_horizontal_focus_uninstall: bool,
    pub(crate) last_copy_notification: Option<(Instant, String)>,
    pub(crate) switch_notice: Option<ProjectSwitchNotice>,
    pub(crate) selected: usize,
    pub(crate) selected_visible: usize,
    pub(crate) glyphs: Vec<InteractiveGlyph>,
    expanded_compositions: BTreeSet<String>,
    pub(crate) quit: bool,
    pub(crate) status: Option<String>,
    pub(crate) view: AppView,
    pub(crate) glyphs_focus: GlyphsFocus,
    pub(crate) grid_config: Option<GridConfig>,
    pub(crate) selecting_for_grid: bool,
    glyph_tool_mode: GlyphToolMode,
    animation_selection_order: Vec<String>,
    animation_selection_set: BTreeSet<String>,
    animation_imported_set: BTreeSet<String>,
    animation_preview: Option<AnimationPreview>,
    selecting_for_animation_frames: bool,
    pub(crate) last_build: Option<BuildSummary>,
    pub(crate) last_sample: Option<String>,
    pub(crate) installed_font_path: Option<PathBuf>,
    delete_project_confirm_selection: Option<usize>,
    renaming_input: Option<Input>,
    renaming_original: Option<String>,
    first_install_notice_open: bool,
    launch_overrides: TuiLaunchOverrides,
    build_task: Option<BuildTask>,
    install_task: Option<InstallTask>,
    animation_import_task: Option<AnimationImportTask>,
    live_glyph_source_count: Option<usize>,
    live_glyph_source_probe_fingerprint: Option<u64>,
    live_glyph_source_probe_at: Option<Instant>,
    debug_enabled: bool,
    debug_log_path: Option<PathBuf>,
    debug_log_lines: Vec<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct InteractiveGlyph {
    pub(crate) glyph: PreprocessedGlyph,
    pub(crate) saved_threshold: Option<u8>,
    pub(crate) working_threshold: u8,
}

#[derive(Debug, Clone)]
enum VisibleGlyphRow {
    Single {
        glyph_idx: usize,
    },
    CompositionParent {
        source_key: String,
        rows: usize,
        cols: usize,
        first_child_idx: usize,
    },
    CompositionChild {
        glyph_idx: usize,
        source_key: String,
        row: usize,
        col: usize,
    },
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

struct AnimationImportTask {
    receiver: Receiver<Result<AnimationImportTaskOutput, String>>,
    spinner_index: usize,
    spinner_last_frame_at: Instant,
}

#[derive(Debug, Clone)]
struct BuildTaskOutput {
    summary: BuildSummary,
    sample: String,
}

#[derive(Debug, Clone)]
struct DropImportResult {
    imported: usize,
    renamed: usize,
    skipped_existing: usize,
    skipped_unsupported: usize,
    skipped_missing: usize,
    imported_source_keys: Vec<String>,
}

#[derive(Debug, Clone)]
struct LoadedGlyphs {
    glyphs: Vec<InteractiveGlyph>,
    source_fingerprint: u64,
}

#[derive(Debug, Clone)]
struct AnimationImportTaskOutput {
    import: DropImportResult,
    loaded: Option<LoadedGlyphs>,
}

#[derive(Debug, Clone)]
enum InstallTaskOutput {
    Install {
        summary: BuildSummary,
        sample: Option<String>,
        installed_path: PathBuf,
        first_install_on_machine: bool,
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

        let sample_from_manifest = sample_from_installed_font_metadata(&install_dir, &path)
            .ok()
            .flatten();
        let (raw_blocks, animation_rows, animation_previews, animation_exports, truncated) =
            if let Some((blocks, animation_rows, animation_previews, animation_exports)) =
                sample_from_manifest
            {
                (
                    blocks,
                    animation_rows,
                    animation_previews,
                    animation_exports,
                    false,
                )
            } else {
                let (sample, truncated) = fs::read(&path)
                    .ok()
                    .and_then(|bytes| sample_glyphs_from_ttf_bytes(&bytes, WELCOME_SAMPLE_LIMIT))
                    .unwrap_or_default();
                (vec![sample], Vec::new(), Vec::new(), Vec::new(), truncated)
            };
        let blocks = regroup_installed_sample_blocks(raw_blocks);

        samples.push(InstalledFontSample {
            file_name,
            path,
            blocks,
            animation_rows,
            animation_previews,
            animation_exports,
            truncated,
        });
    }

    Ok(samples)
}

fn sample_from_installed_font_metadata(
    install_dir: &Path,
    installed_ttf: &Path,
) -> Result<
    Option<(
        Vec<String>,
        Vec<String>,
        Vec<InstalledFontAnimationPreview>,
        Vec<String>,
    )>,
> {
    let installed_canonical = installed_ttf.canonicalize().ok();
    let mut metadata_candidates = Vec::new();

    for entry in fs::read_dir(install_dir)
        .with_context(|| format!("failed to read {}", install_dir.display()))?
    {
        let entry =
            entry.with_context(|| format!("failed to read entry in {}", install_dir.display()))?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        let is_metadata = file_name.starts_with(INSTALL_METADATA_PREFIX)
            && file_name.ends_with(INSTALL_METADATA_SUFFIX);
        if !is_metadata {
            continue;
        }
        metadata_candidates.push(path);
    }

    for metadata_path in metadata_candidates {
        let raw = match fs::read_to_string(&metadata_path) {
            Ok(raw) => raw,
            Err(_) => continue,
        };
        let metadata = match serde_json::from_str::<InstalledFontMetadataRecord>(&raw) {
            Ok(metadata) => metadata,
            Err(_) => continue,
        };
        let metadata_ttf = PathBuf::from(metadata.installed_ttf);
        let ttf_matches = metadata_ttf == installed_ttf
            || installed_canonical
                .as_deref()
                .zip(metadata_ttf.canonicalize().ok().as_deref())
                .is_some_and(|(left, right)| left == right);
        if !ttf_matches {
            continue;
        }
        if let Some(sample) = sample_from_manifest_path(Path::new(&metadata.manifest_path)) {
            let mut animation_rows = Vec::new();
            let mut animation_previews = Vec::new();
            let mut animation_exports = Vec::new();
            let manifest_path = Path::new(&metadata.manifest_path);
            let resolved_animation_blocks = installed_animation_blocks_from_manifest(manifest_path);
            for snapshot in metadata.animation_snapshots {
                let type_label = match snapshot.animation_type {
                    AnimationType::Standard => "standard",
                    AnimationType::Grid => "grid",
                };
                let frame_blocks = resolved_animation_blocks
                    .get(&snapshot.name)
                    .filter(|blocks| !blocks.is_empty())
                    .cloned()
                    .unwrap_or(snapshot.frame_blocks);
                animation_rows.push(format!(
                    "Animation: {} ({}, {} fps, {} frames)",
                    snapshot.name,
                    type_label,
                    snapshot.fps,
                    frame_blocks.len()
                ));
                animation_previews.push(InstalledFontAnimationPreview {
                    fps: snapshot.fps,
                    frame_blocks: frame_blocks.clone(),
                });
                let mut export = format!(
                    "name: {}\ntype: {}\nfps: {}\n",
                    snapshot.name, type_label, snapshot.fps
                );
                if !frame_blocks.is_empty() {
                    export.push('\n');
                    export.push_str(&frame_blocks.join("\n\n"));
                }
                animation_exports.push(export);
            }
            return Ok(Some((
                sample,
                animation_rows,
                animation_previews,
                animation_exports,
            )));
        }
    }

    Ok(None)
}

fn sample_from_manifest_path(manifest_path: &Path) -> Option<Vec<String>> {
    let manifest = read_manifest(manifest_path).ok()?;
    let project_dir = manifest_path.parent().unwrap_or_else(|| Path::new("."));
    let sample_path = project_dir.join(manifest.out_dir).join("glyph-sample.txt");
    let sample = fs::read_to_string(sample_path).ok()?;
    let sample = sample.trim_end().to_string();
    if sample.is_empty() {
        None
    } else {
        Some(regroup_installed_sample_blocks(
            sample
                .split("\n\n")
                .map(|s| s.to_string())
                .collect::<Vec<_>>(),
        ))
    }
}

fn installed_animation_blocks_from_manifest(manifest_path: &Path) -> BTreeMap<String, Vec<String>> {
    let manifest = match read_manifest(manifest_path) {
        Ok(manifest) => manifest,
        Err(_) => return BTreeMap::new(),
    };
    let project_dir = manifest_path.parent().unwrap_or_else(|| Path::new("."));
    let mapping_path = project_dir.join(&manifest.out_dir).join("glyph-map.json");
    let mapping_raw = match fs::read_to_string(mapping_path) {
        Ok(raw) => raw,
        Err(_) => return BTreeMap::new(),
    };
    let mappings: Vec<MappingEntry> = match serde_json::from_str(&mapping_raw) {
        Ok(mappings) => mappings,
        Err(_) => return BTreeMap::new(),
    };
    let by_source = mappings
        .into_iter()
        .map(|entry| (entry.source_file, entry.codepoint))
        .collect::<BTreeMap<_, _>>();

    manifest
        .animations
        .into_iter()
        .map(|animation| {
            let blocks = installed_animation_blocks_for_definition(&animation, &by_source);
            (animation.name, blocks)
        })
        .collect()
}

fn installed_animation_blocks_for_definition(
    animation: &AnimationDef,
    by_source: &BTreeMap<String, String>,
) -> Vec<String> {
    animation
        .frames
        .iter()
        .map(|frame| match animation.animation_type {
            AnimationType::Standard => installed_animation_source_block(by_source, frame)
                .unwrap_or_else(|| format!("[missing:{frame}]")),
            AnimationType::Grid => {
                let rows = animation.rows.unwrap_or(1);
                let cols = animation.cols.unwrap_or(1);
                (0..rows)
                    .map(|row| {
                        (0..cols)
                            .map(|col| {
                                let key = format!("{frame}#compose:{rows}x{cols}:{row}:{col}");
                                installed_animation_source_block(by_source, &key)
                                    .and_then(|block| block.chars().next())
                                    .unwrap_or(' ')
                            })
                            .collect::<String>()
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            }
        })
        .collect()
}

fn installed_animation_source_block(
    by_source: &BTreeMap<String, String>,
    source_key: &str,
) -> Option<String> {
    let codepoint = by_source
        .get(source_key)
        .cloned()
        .or_else(|| remap_compose_source_key_unambiguous(by_source.keys(), source_key)
            .and_then(|resolved| by_source.get(&resolved).cloned()));
    codepoint
        .as_deref()
        .and_then(|cp| format_codepoint_char(cp))
        .map(|c| c.to_string())
}

fn format_codepoint_char(codepoint: &str) -> Option<char> {
    let raw = codepoint.strip_prefix("U+").unwrap_or(codepoint);
    u32::from_str_radix(raw, 16).ok().and_then(char::from_u32)
}

fn glyph_matches_animation_frame_source(glyph: &InteractiveGlyph, frame_source_key: &str) -> bool {
    if glyph.glyph.source_key == frame_source_key
        || glyph.glyph.source_parent_key == frame_source_key
    {
        return true;
    }
    let Some((frame_parent, frame_rows, frame_cols, frame_row, frame_col)) =
        parse_compose_tile_key(frame_source_key)
    else {
        return false;
    };
    let Some((glyph_parent, glyph_rows, glyph_cols, glyph_row, glyph_col)) =
        parse_compose_tile_key(&glyph.glyph.source_key)
    else {
        return false;
    };
    glyph_parent == frame_parent
        && glyph_rows == frame_rows
        && glyph_cols == frame_cols
        && glyph_row == frame_row
        && glyph_col == frame_col
}

fn parse_compose_tile_key(source_key: &str) -> Option<(&str, usize, usize, usize, usize)> {
    let (parent, compose) = source_key.split_once("#compose:")?;
    let (dims, pos) = compose.split_once(':')?;
    let mut dim_parts = dims.split('x');
    let rows = dim_parts.next()?.parse::<usize>().ok()?;
    let cols = dim_parts.next()?.parse::<usize>().ok()?;
    let mut pos_parts = pos.split(':');
    let row = pos_parts.next()?.parse::<usize>().ok()?;
    let col = pos_parts.next()?.parse::<usize>().ok()?;
    Some((parent, rows, cols, row, col))
}

fn remap_compose_source_key_unambiguous<'a>(
    existing_keys: impl Iterator<Item = &'a String>,
    source_key: &str,
) -> Option<String> {
    let (parent, rows, cols, row, col) = parse_compose_tile_key(source_key)?;
    let mut matched = existing_keys.filter_map(|candidate| {
        let (candidate_parent, candidate_rows, candidate_cols, candidate_row, candidate_col) =
            parse_compose_tile_key(candidate)?;
        if candidate_parent == parent
            && candidate_rows == rows
            && candidate_cols == cols
            && candidate_row == row
            && candidate_col == col
        {
            Some(candidate.clone())
        } else {
            None
        }
    });
    let first = matched.next()?;
    if matched.next().is_some() {
        return None;
    }
    Some(first)
}

pub(crate) fn regroup_installed_sample_blocks(blocks: Vec<String>) -> Vec<String> {
    let mut standard_blocks = Vec::new();
    let mut grid_blocks = Vec::new();

    for block in blocks {
        let normalized = block.trim().to_string();
        if normalized.is_empty() {
            continue;
        }
        if normalized.contains('\n') {
            grid_blocks.push(normalized);
        } else {
            standard_blocks.push(normalized);
        }
    }

    let mut grouped = Vec::new();
    if !standard_blocks.is_empty() {
        grouped.push(expand_standard_sample_cells(&standard_blocks.join(" ")));
    }
    grouped.extend(grid_blocks);
    grouped
}

fn expand_standard_sample_cells(sample: &str) -> String {
    let mut out = String::with_capacity(sample.len() * 2);
    for ch in sample.chars() {
        if ch.is_whitespace() {
            continue;
        }
        out.push(ch);
        out.push_str("   ");
    }
    out.trim_end().to_string()
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
        (WelcomeFocus::CreateInput, false) => "press Enter to create",
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
        "view={:?} welcome_focus={:?} glyphs_focus={:?} grid_config={} selecting_for_grid={} selected_project={} editing={} verbose_paths={} input={:?} cursor={} visual_cursor={} build_task={} install_task={} delete_confirm_selection={:?} renaming={} status={:?} quit={}",
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
        app.build_task.is_some(),
        app.install_task.is_some(),
        app.delete_project_confirm_selection,
        app.renaming_input.is_some(),
        app.status,
        app.quit
    )
}

fn handle_grid_config_key(app: &mut App, config: &mut GridConfig, key: KeyEvent) -> Result<()> {
    let code = key.code;
    match code {
        KeyCode::Esc => {
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

fn copy_to_clipboard(text: String) {
    let child = if std::env::var("WAYLAND_DISPLAY").is_ok() {
        std::process::Command::new("wl-copy")
            .stdin(std::process::Stdio::piped())
            .spawn()
    } else {
        std::process::Command::new("xclip")
            .arg("-selection")
            .arg("clipboard")
            .stdin(std::process::Stdio::piped())
            .spawn()
    };

    if let Ok(mut child) = child {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(text.as_bytes());
        }
        let _ = child.wait();
    }
}

fn handle_glyphs_key(app: &mut App, key: KeyEvent) -> Result<()> {
    let code = key.code;
    app.normalize_glyphs_focus();

    if is_keypad_plus_alias(&key) {
        adjust_selected_threshold_by(app, 1);
        return Ok(());
    }
    if is_keypad_minus_alias(&key) {
        adjust_selected_threshold_by(app, -1);
        return Ok(());
    }

    if let Some(mut config) = app.grid_config.clone() {
        let res = handle_grid_config_key(app, &mut config, key);
        app.grid_config = if app.grid_config.is_some() {
            Some(config)
        } else {
            None
        };
        return res;
    }

    if let GlyphToolMode::ChooseAnimationType { mut focus } = app.glyph_tool_mode.clone() {
        match code {
            KeyCode::Esc => {
                app.glyph_tool_mode = GlyphToolMode::None;
                app.clear_animation_draft();
                app.status = Some("animation creation canceled".to_string());
            }
            KeyCode::Left | KeyCode::Char('h') => {
                focus = AnimationTypeChoiceFocus::Standard;
                app.glyph_tool_mode = GlyphToolMode::ChooseAnimationType { focus };
            }
            KeyCode::Right | KeyCode::Char('l') => {
                focus = AnimationTypeChoiceFocus::Grid;
                app.glyph_tool_mode = GlyphToolMode::ChooseAnimationType { focus };
            }
            KeyCode::Enter => {
                let animation_type = match focus {
                    AnimationTypeChoiceFocus::Standard => AnimationType::Standard,
                    AnimationTypeChoiceFocus::Grid => AnimationType::Grid,
                };
                app.glyph_tool_mode = GlyphToolMode::SelectAnimationFrames(animation_type);
                app.selecting_for_animation_frames = true;
                app.status = Some(
                    "Select imported frame glyphs with Space, then Enter to configure".to_string(),
                );
            }
            _ => {}
        }
        return Ok(());
    }

    if let GlyphToolMode::ImportAnimationFrames = app.glyph_tool_mode.clone() {
        match code {
            KeyCode::Esc => {
                if app.animation_import_task.is_some() {
                    app.status = Some("animation frames are still loading".to_string());
                    return Ok(());
                }
                app.glyph_tool_mode = GlyphToolMode::None;
                app.clear_animation_draft();
                app.status = Some("animation import canceled".to_string());
            }
            KeyCode::Enter => {
                if app.animation_import_task.is_some() {
                    app.status = Some("animation frames are still loading".to_string());
                    return Ok(());
                }
                app.glyph_tool_mode = GlyphToolMode::ChooseAnimationType {
                    focus: AnimationTypeChoiceFocus::Standard,
                };
                app.status = Some("Choose animation type".to_string());
            }
            _ => {}
        }
        return Ok(());
    }

    if let GlyphToolMode::SelectAnimationFrames(animation_type) = app.glyph_tool_mode.clone() {
        match code {
            KeyCode::Esc => {
                app.glyph_tool_mode = GlyphToolMode::None;
                app.clear_animation_draft();
                app.status = Some("animation frame selection canceled".to_string());
            }
            KeyCode::Char(' ') => {
                if let Some(selected_source_key) = selected_source_parent_key(app) {
                    if !app.animation_imported_set.contains(&selected_source_key) {
                        app.status = Some(
                            "only images imported in this animation flow can be used as frames"
                                .to_string(),
                        );
                        return Ok(());
                    }
                    if app.animation_selection_set.contains(&selected_source_key) {
                        app.animation_selection_set.remove(&selected_source_key);
                        app.animation_selection_order
                            .retain(|k| k != &selected_source_key);
                    } else {
                        app.animation_selection_set
                            .insert(selected_source_key.clone());
                        app.animation_selection_order.push(selected_source_key);
                    }
                }
            }
            KeyCode::Enter => {
                if app.animation_selection_order.is_empty() {
                    app.status = Some("select at least one frame".to_string());
                } else {
                    app.start_animation_config(animation_type);
                    app.selecting_for_animation_frames = false;
                }
            }
            _ => {}
        }
        return Ok(());
    }

    if let GlyphToolMode::ConfigureAnimation(mut animation_config) = app.glyph_tool_mode.clone() {
        match code {
            KeyCode::Esc => {
                app.glyph_tool_mode = GlyphToolMode::None;
                app.clear_animation_draft();
                app.status = Some("animation configuration canceled".to_string());
                return Ok(());
            }
            KeyCode::Left | KeyCode::Char('h') => {
                animation_config.focus = match animation_config.focus {
                    AnimationConfigFocus::Name => AnimationConfigFocus::Name,
                    AnimationConfigFocus::Fps => AnimationConfigFocus::Name,
                    AnimationConfigFocus::Rows => AnimationConfigFocus::Fps,
                    AnimationConfigFocus::Cols => AnimationConfigFocus::Rows,
                    AnimationConfigFocus::HorizontalBleed => AnimationConfigFocus::Cols,
                    AnimationConfigFocus::VerticalBleed => AnimationConfigFocus::HorizontalBleed,
                    AnimationConfigFocus::Create => {
                        if animation_config.animation_type == AnimationType::Grid {
                            AnimationConfigFocus::VerticalBleed
                        } else {
                            AnimationConfigFocus::Fps
                        }
                    }
                };
            }
            KeyCode::Right | KeyCode::Char('l') => {
                animation_config.focus = match animation_config.focus {
                    AnimationConfigFocus::Name => AnimationConfigFocus::Fps,
                    AnimationConfigFocus::Fps => {
                        if animation_config.animation_type == AnimationType::Grid {
                            AnimationConfigFocus::Rows
                        } else {
                            AnimationConfigFocus::Create
                        }
                    }
                    AnimationConfigFocus::Rows => AnimationConfigFocus::Cols,
                    AnimationConfigFocus::Cols => AnimationConfigFocus::HorizontalBleed,
                    AnimationConfigFocus::HorizontalBleed => AnimationConfigFocus::VerticalBleed,
                    AnimationConfigFocus::VerticalBleed => AnimationConfigFocus::Create,
                    AnimationConfigFocus::Create => AnimationConfigFocus::Create,
                };
            }
            KeyCode::Up | KeyCode::Char('k') => match animation_config.focus {
                AnimationConfigFocus::Fps => {
                    animation_config.fps = animation_config.fps.saturating_add(1).clamp(1, 30)
                }
                AnimationConfigFocus::Rows => {
                    animation_config.rows = animation_config.rows.saturating_add(1).max(1)
                }
                AnimationConfigFocus::Cols => {
                    animation_config.cols = animation_config.cols.saturating_add(1).max(1)
                }
                AnimationConfigFocus::HorizontalBleed => {
                    animation_config.horizontal_bleed =
                        next_bleed_level(animation_config.horizontal_bleed)
                }
                AnimationConfigFocus::VerticalBleed => {
                    animation_config.vertical_bleed =
                        next_bleed_level(animation_config.vertical_bleed)
                }
                _ => {}
            },
            KeyCode::Down | KeyCode::Char('j') => match animation_config.focus {
                AnimationConfigFocus::Fps => {
                    animation_config.fps = animation_config.fps.saturating_sub(1).clamp(1, 30)
                }
                AnimationConfigFocus::Rows => {
                    animation_config.rows = animation_config.rows.saturating_sub(1).max(1)
                }
                AnimationConfigFocus::Cols => {
                    animation_config.cols = animation_config.cols.saturating_sub(1).max(1)
                }
                AnimationConfigFocus::HorizontalBleed => {
                    animation_config.horizontal_bleed =
                        previous_bleed_level(animation_config.horizontal_bleed)
                }
                AnimationConfigFocus::VerticalBleed => {
                    animation_config.vertical_bleed =
                        previous_bleed_level(animation_config.vertical_bleed)
                }
                _ => {}
            },
            KeyCode::Enter => {
                app.create_animation_from_config(&animation_config)?;
                return Ok(());
            }
            _ => {
                if animation_config.focus == AnimationConfigFocus::Name {
                    animation_config.name_input.handle_event(&Event::Key(key));
                }
            }
        }
        app.glyph_tool_mode = GlyphToolMode::ConfigureAnimation(animation_config);
        return Ok(());
    }

    match code {
        KeyCode::Esc => {
            if app.selecting_for_grid {
                app.selecting_for_grid = false;
                app.status = Some("grid selection canceled".to_string());
            } else if app.selecting_for_animation_frames {
                app.selecting_for_animation_frames = false;
                app.glyph_tool_mode = GlyphToolMode::None;
                app.clear_animation_draft();
                app.status = Some("animation selection canceled".to_string());
            } else {
                app.view = AppView::Welcome;
                app.welcome_focus = WelcomeFocus::BuildButton;
            }
        }
        KeyCode::Char('q') => {
            app.quit = true;
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if app.glyphs_focus == GlyphsFocus::List {
                let row_count = app.visible_glyph_rows().len();
                if row_count > 0 {
                    app.selected_visible = (app.selected_visible + 1).min(row_count - 1);
                    app.clamp_glyph_selection();
                }
            } else if app.glyphs_focus == GlyphsFocus::GridButton
                || app.glyphs_focus == GlyphsFocus::AnimateButton
            {
                if !app.visible_glyph_rows().is_empty() {
                    app.glyphs_focus = GlyphsFocus::List;
                    app.selected_visible = 0;
                    app.clamp_glyph_selection();
                }
            }
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if app.glyphs_focus == GlyphsFocus::List {
                if app.selected_visible == 0 {
                    app.glyphs_focus = GlyphsFocus::GridButton;
                } else {
                    app.selected_visible = app.selected_visible.saturating_sub(1);
                    app.clamp_glyph_selection();
                }
            }
        }
        KeyCode::Left | KeyCode::Char('h') | KeyCode::Char('-') => {
            if app.glyphs_focus == GlyphsFocus::List {
                adjust_selected_threshold_by(app, -1);
            } else if app.glyphs_focus == GlyphsFocus::AnimateButton {
                app.glyphs_focus = GlyphsFocus::GridButton;
            }
        }
        KeyCode::Right | KeyCode::Char('l') | KeyCode::Char('+') | KeyCode::Char('=') => {
            if app.glyphs_focus == GlyphsFocus::List {
                adjust_selected_threshold_by(app, 1);
            } else if app.glyphs_focus == GlyphsFocus::GridButton {
                app.glyphs_focus = GlyphsFocus::AnimateButton;
            }
        }
        KeyCode::Enter | KeyCode::Char(' ') => match app.glyphs_focus {
            GlyphsFocus::List => {
                if app.selecting_for_grid {
                    if matches!(
                        app.selected_visible_row(),
                        Some(VisibleGlyphRow::CompositionChild { .. })
                    ) {
                        app.status = Some(
                            "Select a standalone glyph or composition parent (children cannot be selected)"
                                .to_string(),
                        );
                        return Ok(());
                    }
                    if let Some(selected_source_key) = selected_source_parent_key(app) {
                        let source_key = if app
                            .config
                            .compositions
                            .contains_key(&selected_source_key)
                        {
                            duplicate_selected_parent_source_for_grid(app, &selected_source_key)?
                        } else {
                            selected_source_key
                        };
                        app.grid_config = Some(GridConfig {
                            source_key,
                            rows: 2,
                            cols: 2,
                            horizontal_bleed: BleedLevel::Weak,
                            vertical_bleed: BleedLevel::Off,
                            focus: GridConfigFocus::Rows,
                        });
                        app.selecting_for_grid = false;
                        app.status = Some(
                            "Configure grid: use arrows to change rows/cols, Right to Create"
                                .to_string(),
                        );
                    }
                } else {
                    app.toggle_selected_composition_expansion();
                }
            }
            GlyphsFocus::GridButton => {
                app.selecting_for_grid = true;
                app.glyphs_focus = GlyphsFocus::List;
                app.status =
                    Some("Select a glyph to use as grid source and press Enter".to_string());
            }
            GlyphsFocus::AnimateButton => {
                app.clear_animation_draft();
                app.glyph_tool_mode = GlyphToolMode::ImportAnimationFrames;
                app.selecting_for_animation_frames = true;
                app.status = Some(
                    "Import animation frame images now, then press Enter to continue".to_string(),
                );
            }
        },
        KeyCode::Char('c') => {
            apply_default_composition_to_selected(app)?;
        }
        KeyCode::Char('C') => {
            clear_selected_composition(app)?;
        }
        KeyCode::Char('D') => {
            let Some(source_key) = selected_source_parent_key(app) else {
                app.status = Some("no glyph selected".to_string());
                return Ok(());
            };
            let matches = app
                .config
                .animations
                .iter()
                .filter(|a| a.frames.iter().any(|f| f == &source_key))
                .map(|a| a.name.clone())
                .collect::<Vec<_>>();
            if matches.is_empty() {
                app.status = Some("no animation linked to selected glyph".to_string());
            } else {
                let target = matches[0].clone();
                if remove_animation_definition(&app.manifest_path, &target)? {
                    app.reload_glyphs()?;
                    app.refresh_workspace_discovery()?;
                    app.status = Some(format!("deleted animation `{target}`"));
                }
            }
        }
        KeyCode::PageUp => {
            if let Some(glyph) = selected_glyph(app) {
                let next = glyph.working_threshold.saturating_add(10);
                set_selected_threshold(app, next);
            }
        }
        KeyCode::PageDown => {
            if let Some(glyph) = selected_glyph(app) {
                let next = glyph.working_threshold.saturating_sub(10);
                set_selected_threshold(app, next);
            }
        }
        KeyCode::Char('r') => {
            remove_selected_threshold_override(app);
        }
        _ => {}
    }
    Ok(())
}

fn handle_rename_mode_key(app: &mut App, code: KeyCode) -> Result<()> {
    match code {
        KeyCode::Esc => {
            app.renaming_input = None;
            app.renaming_original = None;
            app.status = Some("rename canceled".to_string());
        }
        KeyCode::Enter => {
            app.confirm_rename()?;
        }
        KeyCode::Char(ch) if is_valid_project_name_char(ch) => {
            if let Some(input) = app.renaming_input.as_mut() {
                input.handle_event(&Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
            }
        }
        KeyCode::Backspace
        | KeyCode::Delete
        | KeyCode::Left
        | KeyCode::Right
        | KeyCode::Home
        | KeyCode::End => {
            if let Some(input) = app.renaming_input.as_mut() {
                input.handle_event(&Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
            }
        }
        _ => {}
    }
    Ok(())
}

fn handle_welcome_key(app: &mut App, key: KeyEvent) -> Result<()> {
    let code = key.code;
    let home_project_actions_enabled = app.active_project.is_some();
    tui_debug_log(
        "welcome.handle.enter",
        format!("{} {}", key_debug(&key), app_debug_state(app)),
    );
    if app.delete_project_confirm_selection.is_some() {
        return handle_delete_project_confirmation_key(app, code);
    }
    if app.renaming_input.is_some() {
        return handle_rename_mode_key(app, code);
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
                WelcomeFocus::VerbosePathsToggle => WelcomeFocus::VerbosePathsToggle,
                WelcomeFocus::ProjectList => {
                    if app.selected_project > 0 {
                        app.selected_project -= 1;
                        WelcomeFocus::ProjectList
                    } else {
                        WelcomeFocus::VerbosePathsToggle
                    }
                }
                WelcomeFocus::CreateInput if !app.projects.is_empty() => {
                    app.selected_project = app.projects.len() - 1;
                    WelcomeFocus::ProjectList
                }
                WelcomeFocus::CreateInput => WelcomeFocus::VerbosePathsToggle,
                WelcomeFocus::BuildButton => WelcomeFocus::VerbosePathsToggle,
                WelcomeFocus::InstallButton => WelcomeFocus::VerbosePathsToggle,
                WelcomeFocus::DeleteProjectButton => WelcomeFocus::VerbosePathsToggle,
                WelcomeFocus::InstalledFontList => {
                    if app.selected_installed_font_sub_index > 0 {
                        app.selected_installed_font_sub_index -= 1;
                        app.installed_font_horizontal_focus_uninstall = false;
                        WelcomeFocus::InstalledFontList
                    } else if app.selected_installed_font > 0 {
                        app.selected_installed_font -= 1;
                        app.selected_installed_font_sub_index = app
                            .installed_font_sub_row_count(app.selected_installed_font)
                            .saturating_sub(1);
                        app.installed_font_horizontal_focus_uninstall = false;
                        WelcomeFocus::InstalledFontList
                    } else if app.active_project.is_some() {
                        WelcomeFocus::BuildButton
                    } else if !app.projects.is_empty() {
                        WelcomeFocus::ProjectList
                    } else {
                        WelcomeFocus::VerbosePathsToggle
                    }
                }
            };
        }
        KeyCode::Down | KeyCode::Char('j') if !app.welcome_input_editing => {
            app.welcome_focus = match app.welcome_focus {
                WelcomeFocus::VerbosePathsToggle => {
                    if app.active_project.is_some() {
                        WelcomeFocus::InstallButton
                    } else if !app.projects.is_empty() {
                        WelcomeFocus::ProjectList
                    } else {
                        WelcomeFocus::CreateInput
                    }
                }
                WelcomeFocus::ProjectList => {
                    if app.selected_project + 1 < app.projects.len() {
                        app.selected_project += 1;
                        WelcomeFocus::ProjectList
                    } else {
                        WelcomeFocus::CreateInput
                    }
                }
                WelcomeFocus::CreateInput => {
                    if !app.installed_fonts.is_empty() {
                        app.selected_installed_font = 0;
                        app.selected_installed_font_sub_index = 0;
                        app.installed_font_horizontal_focus_uninstall = false;
                        WelcomeFocus::InstalledFontList
                    } else {
                        WelcomeFocus::CreateInput
                    }
                }
                WelcomeFocus::BuildButton => {
                    if app.installed_fonts.is_empty() {
                        WelcomeFocus::BuildButton
                    } else {
                        app.selected_installed_font = 0;
                        app.selected_installed_font_sub_index = 0;
                        app.installed_font_horizontal_focus_uninstall = false;
                        WelcomeFocus::InstalledFontList
                    }
                }
                WelcomeFocus::InstallButton => {
                    if app.installed_fonts.is_empty() {
                        WelcomeFocus::InstallButton
                    } else {
                        app.selected_installed_font = 0;
                        app.selected_installed_font_sub_index = 0;
                        app.installed_font_horizontal_focus_uninstall = false;
                        WelcomeFocus::InstalledFontList
                    }
                }
                WelcomeFocus::DeleteProjectButton => {
                    if app.installed_fonts.is_empty() {
                        WelcomeFocus::DeleteProjectButton
                    } else {
                        app.selected_installed_font = 0;
                        app.selected_installed_font_sub_index = 0;
                        app.installed_font_horizontal_focus_uninstall = false;
                        WelcomeFocus::InstalledFontList
                    }
                }
                WelcomeFocus::InstalledFontList => {
                    if app.installed_font_horizontal_focus_uninstall {
                        // Pressing down on Uninstall button goes to sample line (sub-index 1)
                        app.installed_font_horizontal_focus_uninstall = false;
                        app.selected_installed_font_sub_index = 1.min(
                            app.installed_font_sub_row_count(app.selected_installed_font)
                                .saturating_sub(1),
                        );
                        WelcomeFocus::InstalledFontList
                    } else {
                        let sub_count =
                            app.installed_font_sub_row_count(app.selected_installed_font);
                        if app.selected_installed_font_sub_index + 1 < sub_count {
                            app.selected_installed_font_sub_index += 1;
                            WelcomeFocus::InstalledFontList
                        } else if app.selected_installed_font + 1 < app.installed_fonts.len() {
                            app.selected_installed_font += 1;
                            app.selected_installed_font_sub_index = 0;
                            WelcomeFocus::InstalledFontList
                        } else {
                            WelcomeFocus::InstalledFontList
                        }
                    }
                }
            };
        }
        KeyCode::Left | KeyCode::Char('h') if !app.welcome_input_editing => {
            app.welcome_focus = match app.welcome_focus {
                WelcomeFocus::VerbosePathsToggle => WelcomeFocus::VerbosePathsToggle,
                WelcomeFocus::BuildButton => WelcomeFocus::CreateInput,
                WelcomeFocus::InstallButton => WelcomeFocus::BuildButton,
                WelcomeFocus::DeleteProjectButton => WelcomeFocus::InstallButton,
                WelcomeFocus::InstalledFontList => {
                    if app.selected_installed_font_sub_index == 0 {
                        app.installed_font_horizontal_focus_uninstall = false;
                    }
                    WelcomeFocus::InstalledFontList
                }
                WelcomeFocus::ProjectList => WelcomeFocus::ProjectList,
                WelcomeFocus::CreateInput => WelcomeFocus::CreateInput,
            };
        }
        KeyCode::Right | KeyCode::Char('l') if !app.welcome_input_editing => {
            app.welcome_focus = match app.welcome_focus {
                WelcomeFocus::VerbosePathsToggle => WelcomeFocus::VerbosePathsToggle,
                WelcomeFocus::CreateInput => {
                    if home_project_actions_enabled {
                        WelcomeFocus::BuildButton
                    } else {
                        WelcomeFocus::VerbosePathsToggle
                    }
                }
                WelcomeFocus::ProjectList => {
                    if home_project_actions_enabled {
                        WelcomeFocus::BuildButton
                    } else {
                        WelcomeFocus::ProjectList
                    }
                }
                WelcomeFocus::BuildButton => {
                    if home_project_actions_enabled {
                        WelcomeFocus::InstallButton
                    } else {
                        WelcomeFocus::CreateInput
                    }
                }
                WelcomeFocus::InstallButton => {
                    if !home_project_actions_enabled {
                        WelcomeFocus::CreateInput
                    } else if app.active_project_can_be_deleted() {
                        WelcomeFocus::DeleteProjectButton
                    } else {
                        WelcomeFocus::InstallButton
                    }
                }
                WelcomeFocus::DeleteProjectButton => WelcomeFocus::DeleteProjectButton,
                WelcomeFocus::InstalledFontList => {
                    if app.selected_installed_font_sub_index == 0 {
                        app.installed_font_horizontal_focus_uninstall = true;
                    }
                    WelcomeFocus::InstalledFontList
                }
            };
        }
        KeyCode::Enter => match app.welcome_focus {
            WelcomeFocus::VerbosePathsToggle => {
                app.welcome_input_editing = false;
                app.verbose_paths = !app.verbose_paths;
                app.status = Some(format!(
                    "verbose paths {}",
                    if app.verbose_paths {
                        "enabled"
                    } else {
                        "disabled"
                    }
                ));
            }
            WelcomeFocus::ProjectList => {
                app.welcome_input_editing = false;
                if let Some(project) = app.projects.get(app.selected_project) {
                    let is_active = app
                        .active_project
                        .as_ref()
                        .is_some_and(|active| active == &project.manifest_path);
                    if is_active {
                        app.renaming_input = Some(Input::new(app.config.font_name.clone()));
                        app.renaming_original = Some(app.config.font_name.clone());
                        app.status = Some("renaming project...".to_string());
                    } else {
                        app.set_active_project(project.manifest_path.clone())?;
                    }
                }
            }
            WelcomeFocus::CreateInput => {
                if app.welcome_input_editing {
                    app.welcome_input_editing = false;
                    app.status = None;
                    if !app.create_input.value().trim().is_empty() {
                        app.submit_create()?;
                    }
                } else {
                    app.welcome_input_editing = true;
                    app.status = None;
                }
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
            WelcomeFocus::InstalledFontList => {
                app.welcome_input_editing = false;
                if app.installed_font_horizontal_focus_uninstall {
                    trigger_uninstall_action(app)?;
                } else {
                    // Copy to clipboard
                    if let Some(font) = app.installed_fonts.get(app.selected_installed_font) {
                        let content = if app.selected_installed_font_sub_index == 0 {
                            font.path.display().to_string()
                        } else {
                            let sample_count = font.blocks.len();
                            let sub = app.selected_installed_font_sub_index - 1;
                            if sub < sample_count {
                                font.blocks.get(sub).cloned().unwrap_or_default()
                            } else {
                                let anim_idx = sub - sample_count;
                                font.animation_exports
                                    .get(anim_idx)
                                    .cloned()
                                    .unwrap_or_default()
                            }
                        };

                        if !content.is_empty() {
                            copy_to_clipboard(content);
                            let row_id = if app.selected_installed_font_sub_index == 0 {
                                "path".to_string()
                            } else {
                                let sample_count = font.blocks.len();
                                let sub = app.selected_installed_font_sub_index - 1;
                                if sub < sample_count {
                                    format!("sample-{sub}")
                                } else {
                                    format!("animation-{}", sub - sample_count)
                                }
                            };
                            app.last_copy_notification = Some((
                                Instant::now(),
                                format!("{}-{}", app.selected_installed_font, row_id),
                            ));
                        }
                    }
                }
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
                *selection = DELETE_CONFIRM_CANCEL_INDEX;
            }
            app.status = None;
        }
        KeyCode::Right | KeyCode::Char('l') => {
            if let Some(selection) = app.delete_project_confirm_selection.as_mut() {
                *selection = DELETE_CONFIRM_DELETE_INDEX;
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
            Some(_) => {}
            None => {}
        },
        _ => {}
    }
    tui_debug_log("welcome.delete_confirm.exit", app_debug_state(app));
    Ok(())
}

fn handle_first_install_notice_key(app: &mut App, code: KeyCode) -> Result<()> {
    if matches!(code, KeyCode::Enter | KeyCode::Esc | KeyCode::Char(' ')) {
        app.first_install_notice_open = false;
    }
    tui_debug_log("first_install_notice.exit", app_debug_state(app));
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
        for (idx, project) in app.projects.iter().enumerate() {
            let is_active = app
                .active_project
                .as_ref()
                .is_some_and(|active| active == &project.manifest_path);
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
                current_project_sections[3].width,
                current_project_sections[3].height,
                accent,
            ))
            .wrap(Wrap { trim: false }),
            current_project_sections[3],
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
                    } else if app.install_in_progress() || app.build_in_progress() {
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
            for (b_idx, block_str) in font.blocks.iter().enumerate() {
                let sub_idx = b_idx + 1;
                let is_focused = is_selected_font
                    && app.selected_installed_font_sub_index == sub_idx
                    && app.welcome_focus == WelcomeFocus::InstalledFontList;

                let wrapped_lines =
                    installed_font_block_display_lines_with_reference(block_str, sample_wrap_width);

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

                for (l_idx, line_text) in wrapped_lines.into_iter().enumerate() {
                    let mut spans = vec![
                        Span::styled(
                            if l_idx == 0 { bullet } else { "   " },
                            Style::default().fg(if is_focused {
                                Color::White
                            } else {
                                Color::Reset
                            }),
                        ),
                        Span::styled(line_text, base_style),
                    ];

                    if l_idx == 0 && is_focused {
                        if let Some((at, id)) = &app.last_copy_notification {
                            if id == &format!("{}-sample-{}", f_idx, b_idx)
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
    fn clear_animation_draft(&mut self) {
        self.animation_selection_order.clear();
        self.animation_selection_set.clear();
        self.animation_imported_set.clear();
        self.selecting_for_animation_frames = false;
    }

    fn start_animation_config(&mut self, animation_type: AnimationType) {
        let mut frames = self.animation_selection_order.clone();
        if frames.is_empty() {
            let mut fallback = self
                .animation_selection_set
                .iter()
                .cloned()
                .collect::<Vec<_>>();
            sort_source_keys_alphabetically(&mut fallback);
            frames = fallback;
        }
        sort_source_keys_alphabetically(&mut frames);
        let name = default_animation_name(&self.config);
        self.glyph_tool_mode = GlyphToolMode::ConfigureAnimation(AnimationConfig {
            selected_frames: frames,
            name_input: Input::new(name),
            animation_type,
            fps: 8,
            rows: 2,
            cols: 2,
            horizontal_bleed: BleedLevel::Weak,
            vertical_bleed: BleedLevel::Off,
            focus: AnimationConfigFocus::Name,
        });
    }

    fn create_animation_from_config(&mut self, config: &AnimationConfig) -> Result<()> {
        let name = config.name_input.value().trim().to_string();
        if name.is_empty() {
            self.status = Some("animation name cannot be empty".to_string());
            return Ok(());
        }
        if config.selected_frames.is_empty() {
            self.status = Some("animation requires at least one frame".to_string());
            return Ok(());
        }
        if self.config.animations.iter().any(|a| a.name == name) {
            self.status = Some(format!("animation `{name}` already exists"));
            return Ok(());
        }
        let mut selected_frames = config.selected_frames.clone();
        sort_source_keys_alphabetically(&mut selected_frames);

        if config.animation_type == AnimationType::Grid {
            for frame in &selected_frames {
                let desired = CompositionDef {
                    rows: config.rows as usize,
                    cols: config.cols as usize,
                    horizontal_bleed: config.horizontal_bleed,
                    vertical_bleed: config.vertical_bleed,
                };
                if let Some(existing) = self.config.compositions.get(frame) {
                    if existing != &desired {
                        self.status = Some(format!(
                            "grid mismatch for {} (expected {}x{} with selected bleed)",
                            source_display_name(frame),
                            config.rows,
                            config.cols
                        ));
                        return Ok(());
                    }
                } else {
                    persist_composition_definition(&self.manifest_path, frame, Some(desired))?;
                }
            }
            self.reload_config()?;
        }

        let frames = selected_frames;

        let def = AnimationDef {
            name: name.clone(),
            animation_type: config.animation_type,
            fps: config.fps,
            frames,
            rows: (config.animation_type == AnimationType::Grid).then_some(config.rows as usize),
            cols: (config.animation_type == AnimationType::Grid).then_some(config.cols as usize),
            horizontal_bleed: (config.animation_type == AnimationType::Grid)
                .then_some(config.horizontal_bleed),
            vertical_bleed: (config.animation_type == AnimationType::Grid)
                .then_some(config.vertical_bleed),
        };
        persist_animation_definition(&self.manifest_path, def)?;
        self.reload_glyphs()?;
        self.refresh_workspace_discovery()?;
        self.glyph_tool_mode = GlyphToolMode::None;
        self.clear_animation_draft();
        self.status = Some(format!("created animation `{name}`"));
        Ok(())
    }

    fn update_animation_preview(&mut self) {
        if self.config.animations.is_empty() {
            self.animation_preview = None;
            return;
        }
        let Some(source_key) = selected_source_parent_key(self) else {
            self.animation_preview = None;
            return;
        };
        let Some(animation) = self.config.animations.iter().find(|a| {
            a.frames.iter().any(|frame| frame == &source_key)
                || a.frames
                    .iter()
                    .any(|frame| frame.starts_with(&format!("{source_key}#compose:")))
        }) else {
            self.animation_preview = None;
            return;
        };
        let now = Instant::now();
        let mut preview = self.animation_preview.clone().unwrap_or(AnimationPreview {
            animation_name: animation.name.clone(),
            frame_index: 0,
            last_frame_at: now,
        });
        if preview.animation_name != animation.name {
            preview = AnimationPreview {
                animation_name: animation.name.clone(),
                frame_index: 0,
                last_frame_at: now,
            };
        }
        let interval_ms = (1000u64 / u64::from(animation.fps.max(1))).max(1);
        if now.duration_since(preview.last_frame_at) >= Duration::from_millis(interval_ms) {
            preview.frame_index = (preview.frame_index + 1) % animation.frames.len().max(1);
            preview.last_frame_at = now;
        }
        self.animation_preview = Some(preview);
    }

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
        let debug_enabled = glyph_debug::debug_enabled();
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
            verbose_paths: false,
            installed_fonts: Vec::new(),
            installed_animation_started_at: Instant::now(),
            selected_installed_font: 0,
            selected_installed_font_sub_index: 0,
            installed_font_horizontal_focus_uninstall: false,
            last_copy_notification: None,
            switch_notice: None,
            selected: 0,
            selected_visible: 0,
            glyphs: Vec::new(),
            expanded_compositions: BTreeSet::new(),
            quit: false,
            status: None,
            view: AppView::Welcome,
            glyphs_focus: GlyphsFocus::List,
            grid_config: None,
            selecting_for_grid: false,
            glyph_tool_mode: GlyphToolMode::None,
            animation_selection_order: Vec::new(),
            animation_selection_set: BTreeSet::new(),
            animation_imported_set: BTreeSet::new(),
            animation_preview: None,
            selecting_for_animation_frames: false,
            last_build: None,
            last_sample: None,
            installed_font_path: None,
            delete_project_confirm_selection: None,
            renaming_input: None,
            renaming_original: None,
            first_install_notice_open: false,
            launch_overrides,
            build_task: None,
            install_task: None,
            animation_import_task: None,
            live_glyph_source_count: None,
            live_glyph_source_probe_fingerprint: None,
            live_glyph_source_probe_at: None,
            debug_enabled,
            debug_log_path: None,
            debug_log_lines: Vec::new(),
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
        let debug_enabled = glyph_debug::debug_enabled();
        let debug_log_path = Some(glyph_debug::session_log_path(&project_dir));
        let (last_build, last_sample) = cached_build_state(&config);
        let installed_font_path =
            cached_installed_font_path(&manifest_path, &config.font_name, &config.project_id);
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
            verbose_paths: false,
            installed_fonts: Vec::new(),
            installed_animation_started_at: Instant::now(),
            selected_installed_font: 0,
            selected_installed_font_sub_index: 0,
            installed_font_horizontal_focus_uninstall: false,
            last_copy_notification: None,
            switch_notice: None,
            selected: 0,
            selected_visible: 0,
            glyphs: Vec::new(),
            expanded_compositions: BTreeSet::new(),
            quit: false,
            status: None,
            view: AppView::Welcome,
            glyphs_focus: GlyphsFocus::List,
            grid_config: None,
            selecting_for_grid: false,
            glyph_tool_mode: GlyphToolMode::None,
            animation_selection_order: Vec::new(),
            animation_selection_set: BTreeSet::new(),
            animation_imported_set: BTreeSet::new(),
            animation_preview: None,
            selecting_for_animation_frames: false,
            last_build,
            last_sample,
            installed_font_path,
            delete_project_confirm_selection: None,
            renaming_input: None,
            renaming_original: None,
            first_install_notice_open: false,
            launch_overrides,
            build_task: None,
            install_task: None,
            animation_import_task: None,
            live_glyph_source_count: None,
            live_glyph_source_probe_fingerprint: None,
            live_glyph_source_probe_at: None,
            debug_enabled,
            debug_log_path,
            debug_log_lines: Vec::new(),
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

        if self.active_project.is_none()
            && matches!(
                self.welcome_focus,
                WelcomeFocus::BuildButton
                    | WelcomeFocus::InstallButton
                    | WelcomeFocus::DeleteProjectButton
            )
        {
            self.welcome_focus = if self.projects.is_empty() {
                WelcomeFocus::CreateInput
            } else {
                WelcomeFocus::ProjectList
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
            self.selected_installed_font_sub_index = 0;
            return;
        }

        self.selected_installed_font = self
            .selected_installed_font
            .min(self.installed_fonts.len() - 1);

        let sub_count = self.installed_font_sub_row_count(self.selected_installed_font);
        self.selected_installed_font_sub_index = self
            .selected_installed_font_sub_index
            .min(sub_count.saturating_sub(1));
    }

    fn installed_font_sub_row_count(&self, idx: usize) -> usize {
        let font = match self.installed_fonts.get(idx) {
            Some(f) => f,
            None => return 0,
        };
        // 1 (Title) + number of sample blocks + animation rows
        1 + font.blocks.len() + font.animation_rows.len()
    }

    fn visible_glyph_rows(&self) -> Vec<VisibleGlyphRow> {
        let mut rows = Vec::new();
        let mut idx = 0usize;
        while idx < self.glyphs.len() {
            let glyph = &self.glyphs[idx];
            if let Some(tile) = &glyph.glyph.composition_tile {
                if tile.row == 0 && tile.col == 0 {
                    let source_key = glyph.glyph.source_parent_key.clone();
                    rows.push(VisibleGlyphRow::CompositionParent {
                        source_key: source_key.clone(),
                        rows: tile.rows,
                        cols: tile.cols,
                        first_child_idx: idx,
                    });
                    let span = tile.rows.saturating_mul(tile.cols);
                    if self.expanded_compositions.contains(&source_key) {
                        for offset in 0..span {
                            if let Some(child) = self.glyphs.get(idx + offset)
                                && let Some(child_tile) = &child.glyph.composition_tile
                            {
                                rows.push(VisibleGlyphRow::CompositionChild {
                                    glyph_idx: idx + offset,
                                    source_key: source_key.clone(),
                                    row: child_tile.row,
                                    col: child_tile.col,
                                });
                            }
                        }
                    }
                    idx = idx.saturating_add(span.max(1));
                    continue;
                }
                idx += 1;
                continue;
            }

            rows.push(VisibleGlyphRow::Single { glyph_idx: idx });
            idx += 1;
        }
        rows
    }

    fn clamp_glyph_selection(&mut self) {
        let rows = self.visible_glyph_rows();
        if rows.is_empty() {
            self.selected_visible = 0;
            self.selected = 0;
            return;
        }

        self.selected_visible = self.selected_visible.min(rows.len() - 1);
        self.selected = match &rows[self.selected_visible] {
            VisibleGlyphRow::Single { glyph_idx }
            | VisibleGlyphRow::CompositionChild { glyph_idx, .. } => *glyph_idx,
            VisibleGlyphRow::CompositionParent {
                first_child_idx, ..
            } => *first_child_idx,
        };
    }

    fn normalize_glyphs_focus(&mut self) {
        if self.active_project.is_some()
            && self.visible_glyph_rows().is_empty()
            && self.glyphs_focus == GlyphsFocus::List
        {
            self.glyphs_focus = GlyphsFocus::GridButton;
        }
    }

    fn selected_visible_row(&self) -> Option<VisibleGlyphRow> {
        let rows = self.visible_glyph_rows();
        if rows.is_empty() {
            return None;
        }
        rows.get(self.selected_visible.min(rows.len() - 1)).cloned()
    }

    fn toggle_selected_composition_expansion(&mut self) {
        let Some(row) = self.selected_visible_row() else {
            return;
        };
        let source_key = match row {
            VisibleGlyphRow::CompositionParent { source_key, .. }
            | VisibleGlyphRow::CompositionChild { source_key, .. } => source_key,
            VisibleGlyphRow::Single { .. } => return,
        };

        if !self.expanded_compositions.insert(source_key.clone()) {
            self.expanded_compositions.remove(&source_key);
        }
        self.clamp_glyph_selection();
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
        self.selected_visible = 0;
        self.expanded_compositions.clear();
        self.refresh_workspace_discovery()?;
        self.welcome_focus = if self.projects.is_empty() {
            WelcomeFocus::CreateInput
        } else {
            WelcomeFocus::ProjectList
        };
        self.status = Some(format!("deleted project `{deleted_project_name}`"));
        Ok(())
    }

    fn confirm_rename(&mut self) -> Result<()> {
        let Some(input) = self.renaming_input.take() else {
            return Ok(());
        };
        let new_name = input.value().trim().to_string();
        self.renaming_original = None;

        if new_name.is_empty() {
            self.status = Some("project name cannot be empty; rename canceled".to_string());
            return Ok(());
        }

        let old_dir = self.project_dir.clone();
        if old_dir == self.workspace_root {
            self.status = Some("refusing to rename the workspace root directory".to_string());
            return Ok(());
        }

        let new_dir = self.workspace_root.join(&new_name);
        if new_dir.exists() {
            self.status = Some(format!("directory already exists: {}", new_dir.display()));
            return Ok(());
        }

        let old_name = self.config.font_name.clone();
        fs::rename(&old_dir, &new_dir).with_context(|| {
            format!(
                "failed to rename {} to {}",
                old_dir.display(),
                new_dir.display()
            )
        })?;

        let new_manifest_path = new_dir.join("petiglyph.toml");
        let mut manifest = read_manifest(&new_manifest_path)?;
        manifest.font_name = new_name.clone();
        write_manifest(&new_manifest_path, &manifest)?;

        let out_dir = new_dir.join(&manifest.out_dir);
        let old_ttf = out_dir.join(format!("{old_name}.ttf"));
        let new_ttf = out_dir.join(format!("{new_name}.ttf"));
        if old_ttf.is_file() && !new_ttf.exists() {
            fs::rename(&old_ttf, &new_ttf).with_context(|| {
                format!(
                    "failed to rename {} to {}",
                    old_ttf.display(),
                    new_ttf.display()
                )
            })?;
        }
        let old_bdf = out_dir.join(format!("{old_name}.bdf"));
        let new_bdf = out_dir.join(format!("{new_name}.bdf"));
        if old_bdf.is_file() && !new_bdf.exists() {
            fs::rename(&old_bdf, &new_bdf).with_context(|| {
                format!(
                    "failed to rename {} to {}",
                    old_bdf.display(),
                    new_bdf.display()
                )
            })?;
        }

        self.manifest_path = new_manifest_path;
        self.project_dir = new_dir;
        self.active_project = Some(self.manifest_path.clone());
        self.reload_config()?;
        self.refresh_workspace_discovery()?;
        self.status = Some(format!("renamed project from `{old_name}` to `{new_name}`"));
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
        let old_label = self.active_project_switch_label();
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
                to_label: self.active_project_switch_label(),
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
            self.debug_log_path = None;
            self.debug_log_lines.clear();
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
        self.installed_font_path = cached_installed_font_path(
            &self.manifest_path,
            &self.config.font_name,
            &self.config.project_id,
        );
        self.debug_log_path = Some(glyph_debug::session_log_path(&self.config.project_dir));
        Ok(())
    }

    fn reload_glyphs(&mut self) -> Result<()> {
        if self.active_project.is_none() {
            self.glyphs.clear();
            self.selected = 0;
            self.selected_visible = 0;
            self.expanded_compositions.clear();
            self.live_glyph_source_count = None;
            self.live_glyph_source_probe_fingerprint = None;
            self.live_glyph_source_probe_at = Some(Instant::now());
            self.status = Some("create a project in Home or relaunch with --manifest".to_string());
            return Ok(());
        }

        self.reload_config()?;
        if self.debug_enabled {
            glyph_debug::begin_session(&self.config.project_dir, "tui.reload_glyphs");
        }

        if !self.config.input_dir.exists() {
            self.glyphs.clear();
            self.selected = 0;
            self.selected_visible = 0;
            self.expanded_compositions.clear();
            self.live_glyph_source_count = Some(0);
            self.live_glyph_source_probe_fingerprint = Some(0);
            self.live_glyph_source_probe_at = Some(Instant::now());
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
            self.selected_visible = 0;
            self.expanded_compositions.clear();
            self.live_glyph_source_count = Some(0);
            self.live_glyph_source_probe_fingerprint = Some(0);
            self.live_glyph_source_probe_at = Some(Instant::now());
            self.status = Some(format!(
                "add or drag image files into {}",
                self.config.input_dir.display()
            ));
            return Ok(());
        }

        let glyphs = preprocess_sources_with_compositions(
            &sources,
            &self.config.input_dir,
            self.config.glyph_size,
            &self.config.compositions,
        )?
        .into_iter()
        .map(|glyph| {
            let saved_threshold = self
                .config
                .threshold_overrides
                .get(&glyph.source_parent_key)
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
        let active_compositions = self
            .glyphs
            .iter()
            .filter_map(|g| {
                g.glyph
                    .composition_tile
                    .as_ref()
                    .map(|_| g.glyph.source_parent_key.clone())
            })
            .collect::<BTreeSet<_>>();
        self.expanded_compositions
            .retain(|source| active_compositions.contains(source));
        self.clamp_glyph_selection();
        self.live_glyph_source_count = Some(self.glyphs.len());
        self.live_glyph_source_probe_fingerprint =
            Some(glyph_source_fingerprint(&self.config.input_dir)?);
        self.live_glyph_source_probe_at = Some(Instant::now());
        let mut status = format!(
            "loaded {} glyph{} from {}",
            self.glyphs.len(),
            if self.glyphs.len() == 1 { "" } else { "s" },
            self.config.input_dir.display()
        );
        if self.debug_enabled {
            status.push_str(&format!(
                " | debug: {}",
                self.config.project_dir.join("debug").display()
            ));
        }
        self.status = Some(status);
        Ok(())
    }

    fn refresh_pipeline_debug_log(&mut self) {
        if !self.debug_enabled {
            self.debug_log_lines.clear();
            return;
        }
        let Some(path) = &self.debug_log_path else {
            self.debug_log_lines.clear();
            return;
        };
        self.debug_log_lines = glyph_debug::read_recent_log_lines(path, DEBUG_LOG_VISIBLE_LINES);
    }

    fn refresh_live_glyph_source_count(&mut self) {
        if self.active_project.is_none() {
            self.live_glyph_source_count = None;
            self.live_glyph_source_probe_fingerprint = None;
            self.live_glyph_source_probe_at = Some(Instant::now());
            return;
        }

        let now = Instant::now();
        if self.live_glyph_source_probe_at.is_some_and(|at| {
            now.duration_since(at) < Duration::from_millis(GLYPH_SOURCE_COUNT_REFRESH_MS)
        }) {
            return;
        }
        self.live_glyph_source_probe_at = Some(now);

        let Ok(next_fingerprint) = glyph_source_fingerprint(&self.config.input_dir) else {
            return;
        };

        if self.live_glyph_source_probe_fingerprint == Some(next_fingerprint) {
            return;
        }

        self.live_glyph_source_probe_fingerprint = Some(next_fingerprint);
        self.live_glyph_source_count =
            Some(count_supported_sources(&self.config.input_dir).unwrap_or(self.glyphs.len()));
    }

    fn import_dropped_images(&mut self, payload: &str) -> Result<()> {
        if self.build_in_progress()
            || self.install_in_progress()
            || self.animation_import_task.is_some()
        {
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
        if matches!(
            self.glyph_tool_mode,
            GlyphToolMode::ImportAnimationFrames
        ) {
            self.start_animation_frame_import(payload.to_string())?;
            return Ok(());
        }

        let import = import_image_files_to_input(&self.config.input_dir, payload)?;

        if import.imported > 0 {
            self.reload_glyphs()?;
            if self.view == AppView::Welcome {
                self.welcome_input_editing = false;
                self.view = AppView::Glyphs;
            }
        }

        self.status = Some(format_drop_import_status(
            import.imported,
            import.renamed,
            import.skipped_existing,
            import.skipped_unsupported,
            import.skipped_missing,
        ));
        Ok(())
    }

    fn start_animation_frame_import(&mut self, payload: String) -> Result<()> {
        if self.animation_import_task.is_some() {
            self.status = Some("animation frames are already loading".to_string());
            return Ok(());
        }

        let config = self.config.clone();
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let result = (|| -> Result<AnimationImportTaskOutput> {
                let import = import_image_files_to_input(&config.input_dir, &payload)?;
                let loaded = if import.imported > 0 {
                    Some(load_interactive_glyphs_from_config(&config)?)
                } else {
                    None
                };
                Ok(AnimationImportTaskOutput { import, loaded })
            })()
            .map_err(|err| err.to_string());
            let _ = sender.send(result);
        });

        self.animation_import_task = Some(AnimationImportTask {
            receiver,
            spinner_index: 0,
            spinner_last_frame_at: Instant::now(),
        });
        self.status = Some("loading animation frames...".to_string());
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
                first_install_on_machine,
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
                if first_install_on_machine {
                    self.first_install_notice_open = true;
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

    fn poll_animation_import_task(&mut self) {
        let mut task_result = None;
        let mut disconnected = false;

        if let Some(task) = self.animation_import_task.as_mut() {
            let frame_duration = Duration::from_millis(FONT_TASK_SPINNER_FRAME_MS);
            let now = Instant::now();
            while now.duration_since(task.spinner_last_frame_at) >= frame_duration {
                task.spinner_index =
                    (task.spinner_index + 1) % ANIMATION_IMPORT_SPINNER_FRAMES.len();
                task.spinner_last_frame_at += frame_duration;
            }
            match task.receiver.try_recv() {
                Ok(result) => task_result = Some(result),
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => disconnected = true,
            }
        }

        if disconnected {
            self.animation_import_task = None;
            self.status = Some("animation frame import task terminated unexpectedly".to_string());
            return;
        }

        let Some(result) = task_result else {
            return;
        };

        self.animation_import_task = None;
        match result {
            Ok(output) => self.finish_animation_import(output),
            Err(err) => self.status = Some(format!("animation frame import failed: {err}")),
        }
    }

    fn finish_animation_import(&mut self, output: AnimationImportTaskOutput) {
        if let Some(loaded) = output.loaded {
            self.glyphs = loaded.glyphs;
            self.clamp_glyph_selection();
            self.live_glyph_source_count = Some(self.glyphs.len());
            self.live_glyph_source_probe_fingerprint = Some(loaded.source_fingerprint);
            self.live_glyph_source_probe_at = Some(Instant::now());
            if self.view == AppView::Welcome {
                self.welcome_input_editing = false;
                self.view = AppView::Glyphs;
            }
        }

        for source_key in output.import.imported_source_keys {
            self.animation_imported_set.insert(source_key.clone());
            if self.animation_selection_set.insert(source_key.clone()) {
                self.animation_selection_order.push(source_key);
            }
        }

        if output.import.imported > 0 {
            self.status = Some(format!(
                "animation draft import: {} frame{} selected",
                self.animation_selection_order.len(),
                if self.animation_selection_order.len() == 1 {
                    ""
                } else {
                    "s"
                }
            ));
        } else {
            self.status = Some(format_drop_import_status(
                output.import.imported,
                output.import.renamed,
                output.import.skipped_existing,
                output.import.skipped_unsupported,
                output.import.skipped_missing,
            ));
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

    fn animation_import_spinner_frame(&self) -> Option<&'static str> {
        self.animation_import_task.as_ref().map(|task| {
            ANIMATION_IMPORT_SPINNER_FRAMES
                [task.spinner_index % ANIMATION_IMPORT_SPINNER_FRAMES.len()]
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
        self.build_in_progress()
            || self.install_in_progress()
            || self.animation_import_task.is_some()
    }

    #[cfg(test)]
    pub(crate) fn poll_background_tasks_for_test(&mut self) {
        self.poll_build_task();
        self.poll_font_task();
        self.poll_animation_import_task();
    }

    fn active_project_label(&self) -> String {
        let Some(active_project) = &self.active_project else {
            return "none".to_string();
        };

        if self.verbose_paths {
            let folder = active_project
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .display();
            format!("{} ({folder})", self.config.font_name)
        } else {
            self.config.font_name.clone()
        }
    }

    fn active_project_switch_label(&self) -> String {
        let Some(active_project) = &self.active_project else {
            return "none".to_string();
        };

        active_project
            .parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| self.config.font_name.clone())
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

fn sort_source_keys_alphabetically(keys: &mut [String]) {
    keys.sort_by_cached_key(|key| source_display_name(key).to_ascii_lowercase());
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
        compositions: Default::default(),
        animations: Vec::new(),
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
        first_install_on_machine: installed.first_install_on_machine,
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

fn cached_installed_font_path(
    manifest_path: &Path,
    font_name: &str,
    project_id: &str,
) -> Option<PathBuf> {
    resolve_installed_font_path_with(manifest_path, font_name, Some(project_id), |path| {
        path.is_file()
    })
}

pub(crate) fn resolve_installed_font_path_with<F>(
    manifest_path: &Path,
    font_name: &str,
    project_id: Option<&str>,
    mut is_installed: F,
) -> Option<PathBuf>
where
    F: FnMut(&Path) -> bool,
{
    let mut candidates = Vec::new();
    if let Ok(paths) =
        installed_ttf_candidates_for_manifest_font(manifest_path, font_name, project_id)
    {
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

fn persist_composition_definition(
    manifest_path: &Path,
    source_key: &str,
    composition: Option<CompositionDef>,
) -> Result<()> {
    let mut manifest = read_manifest(manifest_path)?;
    match composition {
        Some(def) => {
            manifest.compositions.insert(source_key.to_string(), def);
        }
        None => {
            manifest.compositions.remove(source_key);
        }
    }
    write_manifest(manifest_path, &manifest)
}

fn persist_animation_definition(manifest_path: &Path, animation: AnimationDef) -> Result<()> {
    let mut manifest = read_manifest(manifest_path)?;
    manifest.animations.push(animation);
    write_manifest(manifest_path, &manifest)
}

fn remove_animation_definition(manifest_path: &Path, animation_name: &str) -> Result<bool> {
    let mut manifest = read_manifest(manifest_path)?;
    let original_len = manifest.animations.len();
    manifest.animations.retain(|a| a.name != animation_name);
    let removed = manifest.animations.len() != original_len;
    if removed {
        write_manifest(manifest_path, &manifest)?;
    }
    Ok(removed)
}

fn default_animation_name(config: &RuntimeConfig) -> String {
    let existing = config
        .animations
        .iter()
        .map(|a| a.name.as_str())
        .collect::<BTreeSet<_>>();
    for idx in 1..=9999 {
        let candidate = format!("animation_{idx}");
        if !existing.contains(candidate.as_str()) {
            return candidate;
        }
    }
    "animation".to_string()
}

fn selected_source_parent_key(app: &App) -> Option<String> {
    let row = app.selected_visible_row()?;
    match row {
        VisibleGlyphRow::Single { glyph_idx } => app
            .glyphs
            .get(glyph_idx)
            .map(|g| g.glyph.source_parent_key.clone()),
        VisibleGlyphRow::CompositionParent { source_key, .. }
        | VisibleGlyphRow::CompositionChild { source_key, .. } => Some(source_key),
    }
}

fn source_key_from_input_path(input_dir: &Path, source_path: &Path) -> String {
    source_path
        .strip_prefix(input_dir)
        .unwrap_or(source_path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn duplicate_selected_parent_source_for_grid(app: &mut App, source_key: &str) -> Result<String> {
    let Some(source_path) = app
        .glyphs
        .iter()
        .find(|g| g.glyph.source_parent_key == source_key)
        .map(|g| g.glyph.source_path.clone())
    else {
        anyhow::bail!("unable to locate source path for {source_key}");
    };
    let Some(file_name) = source_path.file_name() else {
        anyhow::bail!("invalid source file path for {source_key}");
    };

    let duplicate_path =
        next_incremental_duplicate_destination(&app.config.input_dir, Path::new(file_name))?;
    fs::copy(&source_path, &duplicate_path).with_context(|| {
        format!(
            "failed to duplicate source {} -> {}",
            source_path.display(),
            duplicate_path.display()
        )
    })?;
    Ok(source_key_from_input_path(
        &app.config.input_dir,
        &duplicate_path,
    ))
}

fn next_incremental_duplicate_destination(
    input_dir: &Path,
    source_file_name: &Path,
) -> Result<PathBuf> {
    let stem = source_file_name
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("glyph");
    let ext = source_file_name
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_string());
    let base_stem = stem_without_trailing_numeric_suffixes(stem);

    let mut max_suffix = 0u32;
    for entry in fs::read_dir(input_dir)
        .with_context(|| format!("failed to scan {}", input_dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let candidate_ext = path
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| value.to_string());
        if candidate_ext != ext {
            continue;
        }
        let Some(candidate_stem) = path.file_stem().and_then(|value| value.to_str()) else {
            continue;
        };
        if candidate_stem == base_stem {
            max_suffix = max_suffix.max(0);
            continue;
        }
        if let Some(rest) = candidate_stem.strip_prefix(base_stem)
            && let Some(numeric) = rest.strip_prefix('-')
            && let Ok(value) = numeric.parse::<u32>()
        {
            max_suffix = max_suffix.max(value);
        }
    }

    let next = max_suffix.saturating_add(1);
    let file_name = match ext {
        Some(ext) => format!("{base_stem}-{next}.{ext}"),
        None => format!("{base_stem}-{next}"),
    };
    Ok(input_dir.join(file_name))
}

fn stem_without_trailing_numeric_suffixes(stem: &str) -> &str {
    let mut current = stem;
    loop {
        let Some((head, tail)) = current.rsplit_once('-') else {
            break;
        };
        if tail.is_empty() || !tail.chars().all(|ch| ch.is_ascii_digit()) {
            break;
        }
        current = head;
    }
    if current.is_empty() { stem } else { current }
}

fn apply_default_composition_to_selected(app: &mut App) -> Result<()> {
    if app.active_project.is_none() {
        app.status = Some(
            "create a project in Home or relaunch with --manifest before composing".to_string(),
        );
        return Ok(());
    }

    let Some(source_key) = selected_source_parent_key(app) else {
        app.status = Some("no glyph selected".to_string());
        return Ok(());
    };
    if app.config.compositions.contains_key(&source_key) {
        app.status = Some(format!(
            "composition already exists for {source_key}; press C to remove it first"
        ));
        return Ok(());
    }

    persist_composition_definition(
        &app.manifest_path,
        &source_key,
        Some(CompositionDef {
            rows: 2,
            cols: 2,
            horizontal_bleed: BleedLevel::Weak,
            vertical_bleed: BleedLevel::Off,
        }),
    )?;
    app.reload_glyphs()?;
    app.expanded_compositions.insert(source_key.clone());
    app.clamp_glyph_selection();
    app.status = Some(format!(
        "created composition for {source_key}: 2x2 (edit [compositions] in petiglyph.toml for custom sizes)"
    ));
    Ok(())
}

fn clear_selected_composition(app: &mut App) -> Result<()> {
    if app.active_project.is_none() {
        app.status = Some(
            "create a project in Home or relaunch with --manifest before composing".to_string(),
        );
        return Ok(());
    }
    let Some(source_key) = selected_source_parent_key(app) else {
        app.status = Some("no glyph selected".to_string());
        return Ok(());
    };
    if !app.config.compositions.contains_key(&source_key) {
        app.status = Some(format!("no composition configured for {source_key}"));
        return Ok(());
    }

    persist_composition_definition(&app.manifest_path, &source_key, None)?;
    app.expanded_compositions.remove(&source_key);
    app.reload_glyphs()?;
    app.status = Some(format!("removed composition for {source_key}"));
    Ok(())
}

fn selected_visible_glyph_index(app: &App) -> Option<usize> {
    match app.selected_visible_row()? {
        VisibleGlyphRow::Single { glyph_idx }
        | VisibleGlyphRow::CompositionChild { glyph_idx, .. } => Some(glyph_idx),
        VisibleGlyphRow::CompositionParent { .. } => None,
    }
}

fn selected_glyph_mut(app: &mut App) -> Option<&mut InteractiveGlyph> {
    let idx = selected_visible_glyph_index(app)?;
    app.glyphs.get_mut(idx)
}

fn selected_glyph(app: &App) -> Option<&InteractiveGlyph> {
    let idx = selected_visible_glyph_index(app)?;
    app.glyphs.get(idx)
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

    let source_key = glyph.glyph.source_parent_key.clone();
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

    let source_key = glyph.glyph.source_parent_key.clone();
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
    if app.first_install_notice_open {
        return handle_first_install_notice_key(app, code);
    }
    if matches!(code, KeyCode::Char('v') | KeyCode::Char('V'))
        && !app.welcome_input_editing
        && app.renaming_input.is_none()
    {
        app.verbose_paths = !app.verbose_paths;
        app.status = Some(format!(
            "verbose paths {}",
            if app.verbose_paths {
                "enabled"
            } else {
                "disabled"
            }
        ));
        tui_debug_log("handle_key_event.exit_global", app_debug_state(app));
        return Ok(());
    }
    let is_global_panel_jump = matches!(code, KeyCode::Tab | KeyCode::BackTab)
        || (matches!(code, KeyCode::Char('1') | KeyCode::Char('2'))
            && !app.welcome_input_editing
            && app.renaming_input.is_none());

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

    if app.view == AppView::Glyphs && !is_global_panel_jump {
        tui_debug_log("handle_key_event.route_glyphs", app_debug_state(app));
        let result = handle_glyphs_key(app, key);
        tui_debug_log("handle_key_event.exit_glyphs", app_debug_state(app));
        return result;
    }

    match code {
        KeyCode::Esc | KeyCode::Char('q') => app.quit = true,
        KeyCode::Char('1') => {
            app.welcome_input_editing = false;
            if app.view == AppView::Glyphs && app.active_project.is_some() {
                app.welcome_focus = WelcomeFocus::BuildButton;
            }
            app.view = AppView::Welcome;
            app.grid_config = None;
            app.selecting_for_grid = false;
        }
        KeyCode::Char('2') => {
            app.welcome_input_editing = false;
            app.view = AppView::Glyphs;
            app.normalize_glyphs_focus();
        }
        KeyCode::Tab => {
            app.welcome_input_editing = false;
            app.view = match app.view {
                AppView::Welcome => AppView::Glyphs,
                AppView::Glyphs => AppView::Welcome,
            };
            if app.view == AppView::Glyphs {
                app.normalize_glyphs_focus();
            }
            if app.view == AppView::Welcome && app.active_project.is_some() {
                app.welcome_focus = WelcomeFocus::BuildButton;
            }
        }
        KeyCode::BackTab => {
            app.welcome_input_editing = false;
            app.view = match app.view {
                AppView::Welcome => AppView::Glyphs,
                AppView::Glyphs => AppView::Welcome,
            };
            if app.view == AppView::Glyphs {
                app.normalize_glyphs_focus();
            }
            if app.view == AppView::Welcome && app.active_project.is_some() {
                app.welcome_focus = WelcomeFocus::InstalledFontList;
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
                let row_count = app.visible_glyph_rows().len();
                if row_count > 0 {
                    app.selected_visible = (app.selected_visible + 1).min(row_count - 1);
                    app.clamp_glyph_selection();
                }
            }
        }
        KeyCode::Char('j') => {
            if app.view == AppView::Glyphs {
                let row_count = app.visible_glyph_rows().len();
                if row_count > 0 {
                    app.selected_visible = (app.selected_visible + 1).min(row_count - 1);
                    app.clamp_glyph_selection();
                }
            }
        }
        KeyCode::Up => {
            if app.view == AppView::Glyphs {
                app.selected_visible = app.selected_visible.saturating_sub(1);
                app.clamp_glyph_selection();
            }
        }
        KeyCode::Char('k') => {
            if app.view == AppView::Glyphs {
                app.selected_visible = app.selected_visible.saturating_sub(1);
                app.clamp_glyph_selection();
            }
        }
        KeyCode::Enter | KeyCode::Char(' ') => {
            if app.view == AppView::Glyphs {
                app.toggle_selected_composition_expansion();
            }
        }
        KeyCode::Char('c') => {
            if app.view == AppView::Glyphs {
                apply_default_composition_to_selected(app)?;
            }
        }
        KeyCode::Char('C') => {
            if app.view == AppView::Glyphs {
                clear_selected_composition(app)?;
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

    if hty_full_repaint_enabled() {
        // hty-specific workaround: fully repaint every frame with plain ASCII
        // so terminal default-bg quirks do not leak black tiles.
        draw_ascii_full_repaint_frame(frame, area);
    } else {
        frame.render_widget(Clear, area);
    }

    if !terminal_size_supported(area) {
        draw_terminal_too_small(frame, area, accent, muted);
        return;
    }
    let area = centered_bounded_viewport(area);

    let root = if app.debug_enabled {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // Header
                Constraint::Min(0),    // Body
                Constraint::Length(1), // Footer keys
                Constraint::Length((DEBUG_LOG_VISIBLE_LINES as u16).saturating_add(2)),
            ])
            .split(area)
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // Header
                Constraint::Min(0),    // Body
                Constraint::Length(1), // Footer keys
            ])
            .split(area)
    };

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

    // Body
    let body_area = root[1];

    match app.view {
        AppView::Welcome => draw_welcome_view(frame, app, body_area, accent, muted),
        AppView::Glyphs => draw_glyphs_view(frame, app, body_area, accent, muted),
    }
    draw_delete_project_confirmation_popup(frame, app, area, accent, muted);
    draw_first_install_notice_popup(frame, app, area, accent, muted);
    draw_animation_popup(frame, app, area, accent, muted);

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
        Span::styled(" v ", Style::default().fg(accent)),
        Span::raw("verbose paths  "),
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
        } else if app.welcome_focus == WelcomeFocus::VerbosePathsToggle {
            "toggle verbose paths  "
        } else if app.welcome_focus == WelcomeFocus::ProjectList {
            if app.renaming_input.is_some() {
                "confirm rename  "
            } else {
                "open project  "
            }
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
        } else {
            "start creating  "
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
        } else if app.renaming_input.is_some() {
            footer_spans.extend(vec![
                Span::styled(" Esc ", Style::default().fg(accent)),
                Span::raw("cancel rename  "),
            ]);
        }
    }
    if app.view == AppView::Glyphs {
        footer_spans.extend(vec![
            Span::styled(" \u{2191}/\u{2193} ", Style::default().fg(accent)),
            Span::raw("select  "),
            Span::styled(" Enter/Space ", Style::default().fg(accent)),
            Span::raw("expand  "),
            Span::styled(" c/C ", Style::default().fg(accent)),
            Span::raw("add/remove composition  "),
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
    frame.render_widget(footer, root[2]);

    if app.debug_enabled {
        let debug_lines = if app.debug_log_lines.is_empty() {
            vec![Line::from(vec![Span::styled(
                "no debug pipeline logs yet",
                Style::default().fg(Color::DarkGray),
            )])]
        } else {
            app.debug_log_lines
                .iter()
                .map(|line| Line::from(Span::raw(line.clone())))
                .collect::<Vec<_>>()
        };
        let title = app
            .debug_log_path
            .as_ref()
            .map(|p| format!(" Debug Log ({}) ", p.display()))
            .unwrap_or_else(|| " Debug Log ".to_string());
        let debug_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::DarkGray))
            .title(Span::styled(title, Style::default().fg(Color::Cyan)));
        frame.render_widget(
            Paragraph::new(debug_lines)
                .block(debug_block)
                .wrap(Wrap { trim: false })
                .style(Style::default().fg(Color::Gray)),
            root[3],
        );
    }
}

fn hty_full_repaint_enabled() -> bool {
    *HTY_FULL_REPAINT_ENABLED.get_or_init(|| {
        std::env::var_os(HTY_FULL_REPAINT_ENV)
            .map(|value| {
                let value = value.to_string_lossy();
                !(value.eq_ignore_ascii_case("0")
                    || value.eq_ignore_ascii_case("false")
                    || value.eq_ignore_ascii_case("off")
                    || value.is_empty())
            })
            .unwrap_or(false)
    })
}

fn draw_ascii_full_repaint_frame(frame: &mut Frame, area: Rect) {
    let width = usize::from(area.width);
    let height = usize::from(area.height);
    let line = " ".repeat(width);
    let lines = (0..height)
        .map(|_| Line::from(line.clone()))
        .collect::<Vec<_>>();
    let hty_bg = Color::Rgb(40, 44, 52);
    frame.render_widget(
        Paragraph::new(lines).style(Style::default().bg(hty_bg).fg(hty_bg)),
        area,
    );
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
    let selected_style = Style::default()
        .fg(Color::Black)
        .bg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    let danger_style = Style::default()
        .fg(Color::White)
        .bg(Color::Red)
        .add_modifier(Modifier::BOLD);
    let idle_style = Style::default().fg(Color::White).bg(Color::DarkGray);
    let buttons_row = Line::from(vec![
        Span::raw("  "),
        Span::styled(
            " CANCEL ",
            if selection == DELETE_CONFIRM_CANCEL_INDEX {
                selected_style
            } else {
                idle_style
            },
        ),
        Span::raw("  "),
        Span::styled(
            " DELETE ",
            if selection == DELETE_CONFIRM_DELETE_INDEX {
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
                "Start on CANCEL. Move right once to select DELETE.",
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
        buttons_row,
    ];
    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false }),
        popup,
    );
}

fn draw_first_install_notice_popup(
    frame: &mut Frame,
    app: &App,
    area: Rect,
    accent: Color,
    muted: Color,
) {
    if !app.first_install_notice_open {
        return;
    }

    let restart_target = detected_terminal_name()
        .map(|name| format!("all {name} terminals"))
        .unwrap_or_else(|| "all terminals".to_string());
    let popup = centered_popup_rect(area, 106, 15);
    frame.render_widget(Clear, popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::Yellow))
        .title(Span::styled(
            " First Install Guidance ",
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        ));
    let lines = vec![
        Line::from(""),
        Line::from(vec![Span::styled(
            format!(
                "To load the newly installed glyphs of a new font, you need to restart {restart_target}."
            ),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(""),
        Line::from(Span::styled(
            "If you do not restart, this current terminal session may render glyphs as errors.",
            Style::default().fg(Color::White),
        )),
        Line::from(Span::styled(
            "Preview output can be misleading until the terminal process is restarted.",
            Style::default().fg(Color::White),
        )),
        Line::from(Span::styled(
            "After restart, relaunch petiglyph and verify sample/preview again.",
            Style::default().fg(Color::White),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Press Enter, Esc, or Space to dismiss this message.",
            Style::default().fg(muted),
        )),
    ];
    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true }),
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

fn draw_animation_popup(frame: &mut Frame, app: &App, area: Rect, accent: Color, muted: Color) {
    let text = match &app.glyph_tool_mode {
        GlyphToolMode::None => return,
        GlyphToolMode::ChooseAnimationType { focus } => {
            let standard = if *focus == AnimationTypeChoiceFocus::Standard {
                "[Standard]"
            } else {
                " Standard "
            };
            let grid = if *focus == AnimationTypeChoiceFocus::Grid {
                "[Grid]"
            } else {
                " Grid "
            };
            vec![
                Line::from("Choose animation type"),
                Line::from(""),
                Line::from(format!("  {standard}   {grid}")),
                Line::from(""),
                Line::from("Left/Right to choose, Enter to continue, Esc to cancel."),
            ]
        }
        GlyphToolMode::ImportAnimationFrames => {
            let mut lines = vec![
                Line::from("Importing animation frames"),
                Line::from(""),
            ];
            if let Some(spinner) = app.animation_import_spinner_frame() {
                lines.push(Line::from(format!("{spinner} Loading animation frames...")));
            } else {
                let box_width = 26usize;
                let inner = box_width.saturating_sub(2);
                let top = format!("╭{}╮", dashed_pattern(inner));
                let bottom = format!("╰{}╯", dashed_pattern(inner));
                let label = center_label("DRAG/PASTE IMAGES", inner);
                let border_style = Style::default().fg(accent);
                let label_style = Style::default().fg(accent).add_modifier(Modifier::BOLD);
                lines.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(top, border_style),
                ]));
                lines.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled("│", border_style),
                    Span::styled(label, label_style),
                    Span::styled("│", border_style),
                ]));
                lines.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(bottom, border_style),
                ]));
                lines.push(Line::from("Press Enter when done, Esc to cancel."));
            }
            lines.push(Line::from(format!(
                "Current draft frames: {}",
                app.animation_selection_order.len()
            )));
            lines
        }
        GlyphToolMode::SelectAnimationFrames(animation_type) => vec![
            Line::from(format!("Selecting {:?} animation frames", animation_type)),
            Line::from(""),
            Line::from("Space toggles selected imported glyph row as frame."),
            Line::from("Enter to configure, Esc to cancel."),
            Line::from(format!(
                "Current draft frames: {}",
                app.animation_selection_order.len()
            )),
        ],
        GlyphToolMode::ConfigureAnimation(config) => {
            let type_label = match config.animation_type {
                AnimationType::Standard => "standard",
                AnimationType::Grid => "grid",
            };
            let focus_label = |target: AnimationConfigFocus, label: String| {
                if config.focus == target {
                    format!("[{label}]")
                } else {
                    label
                }
            };
            let mut lines = vec![
                Line::from(format!("Configure {type_label} animation")),
                Line::from(""),
                Line::from(focus_label(
                    AnimationConfigFocus::Name,
                    format!("Name: {}", config.name_input.value()),
                )),
                Line::from(focus_label(
                    AnimationConfigFocus::Fps,
                    format!("FPS: {}", config.fps),
                )),
            ];
            if config.animation_type == AnimationType::Grid {
                lines.push(Line::from(focus_label(
                    AnimationConfigFocus::Rows,
                    format!("Rows: {}", config.rows),
                )));
                lines.push(Line::from(focus_label(
                    AnimationConfigFocus::Cols,
                    format!("Cols: {}", config.cols),
                )));
                lines.push(Line::from(focus_label(
                    AnimationConfigFocus::HorizontalBleed,
                    format!("L/R bleed: {}", bleed_level_label(config.horizontal_bleed)),
                )));
                lines.push(Line::from(focus_label(
                    AnimationConfigFocus::VerticalBleed,
                    format!("T/B bleed: {}", bleed_level_label(config.vertical_bleed)),
                )));
            }
            lines.push(Line::from(format!(
                "Frames: {}",
                config.selected_frames.len()
            )));
            lines.push(Line::from(focus_label(
                AnimationConfigFocus::Create,
                "Create animation".to_string(),
            )));
            lines.push(Line::from(
                "Left/Right move focus, Up/Down adjust, Enter creates, Esc cancels.",
            ));
            lines
        }
    };
    let popup = centered_popup_rect(area, 90, 14);
    frame.render_widget(Clear, popup);
    let block = Block::default()
        .title(Span::styled(
            " Animation ",
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(muted));
    frame.render_widget(
        Paragraph::new(text)
            .block(block)
            .wrap(Wrap { trim: true })
            .style(Style::default().fg(Color::White)),
        popup,
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

    let size_layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(11), // Rows
            Constraint::Length(11), // Cols
            Constraint::Length(28), // Left/right bleed
            Constraint::Length(28), // Top/bottom bleed
            Constraint::Min(0),
            Constraint::Length(15), // Create
        ])
        .split(layout[1]);
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

    let left_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Min(0)])
        .split(chunks[0]);

    let grid_button_style = if app.glyphs_focus == GlyphsFocus::GridButton {
        Style::default()
            .fg(Color::Black)
            .bg(accent)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White).bg(Color::DarkGray)
    };

    let animate_button_style = if app.glyphs_focus == GlyphsFocus::AnimateButton {
        Style::default()
            .fg(Color::Black)
            .bg(accent)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White).bg(Color::DarkGray)
    };

    let button_line = Line::from(vec![
        Span::raw("  "),
        Span::styled(" Create Grid ", grid_button_style),
        Span::raw(" "),
        Span::styled(" Create Animation ", animate_button_style),
    ]);
    frame.render_widget(Paragraph::new(button_line), left_chunks[0]);

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

    let preview_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(muted))
        .title(Span::styled(" Preview ", Style::default().fg(accent)));

    let preview_area = preview_block.inner(chunks[1]);

    let selected_source_for_animation = selected_source_parent_key(app);
    let active_animation = selected_source_for_animation
        .as_ref()
        .and_then(|source_key| {
            app.config.animations.iter().find(|a| {
                a.frames.iter().any(|f| f == source_key)
                    || a.frames
                        .iter()
                        .any(|f| f.starts_with(&format!("{source_key}#compose:")))
            })
        });
    let active_animation_frame = active_animation.and_then(|animation| {
        let preview = app.animation_preview.as_ref()?;
        if preview.animation_name != animation.name {
            return None;
        }
        animation
            .frames
            .get(
                preview
                    .frame_index
                    .min(animation.frames.len().saturating_sub(1)),
            )
            .cloned()
    });

    let mut preview_content = if visible_rows.is_empty() {
        vec![
            Line::from(""),
            Line::from("  Add or drag images into this project."),
        ]
    } else {
        match &visible_rows[app.selected_visible.min(visible_rows.len() - 1)] {
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
                        Span::raw("  Animation: "),
                        Span::styled(
                            active_animation
                                .map(|a| {
                                    format!("{} ({:?}, {} fps)", a.name, a.animation_type, a.fps)
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
                    Line::from(vec![
                        Span::raw("  Preview: "),
                        Span::styled("Assembled composition", Style::default().fg(muted)),
                    ]),
                    Line::from(vec![
                        Span::raw("  Animation: "),
                        Span::styled(
                            active_animation
                                .map(|a| {
                                    format!("{} ({:?}, {} fps)", a.name, a.animation_type, a.fps)
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
                        .find(|g| glyph_matches_animation_frame_source(g, frame_source_key))
                        .map(|g| g.working_threshold)
                        .unwrap_or(app.config.base_threshold);
                    let mut ascii = if animation.animation_type == AnimationType::Grid {
                        let rows = animation.rows.unwrap_or(2);
                        let cols = animation.cols.unwrap_or(2);
                        let tiles = app
                            .glyphs
                            .iter()
                            .filter(|g| g.glyph.source_parent_key == *frame_source_key)
                            .collect::<Vec<_>>();
                        composition_preview_lines_stable_frame(
                            &tiles,
                            threshold,
                            rows,
                            cols,
                            preview_area.width.saturating_sub(4) / 2,
                            preview_area.height.saturating_sub(8),
                        )
                    } else {
                        app.glyphs
                            .iter()
                            .find(|g| glyph_matches_animation_frame_source(g, frame_source_key))
                            .map(|g| {
                                preview_lines_stable_frame(
                                    &g.glyph,
                                    threshold,
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
                        VisibleGlyphRow::Single { glyph_idx }
                        | VisibleGlyphRow::CompositionChild { glyph_idx, .. } => {
                            let active = &app.glyphs[*glyph_idx];
                            let mut ascii = preview_lines(
                                &active.glyph,
                                active.working_threshold,
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
                            let mut ascii = composition_preview_lines(
                                &tiles,
                                threshold,
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
                matrix[dst_idx] = tile.glyph.coverage[src_idx] >= threshold;
            }
        }
    }

    if let Some((cropped, cropped_w, cropped_h)) =
        crop_binary_matrix_to_active_bounds(&matrix, width, height)
    {
        render_binary_preview_lines(&cropped, cropped_w, cropped_h, max_w, max_h, false, true)
    } else {
        vec![Line::from("    [No visible pixels at threshold]")]
    }
}

fn composition_preview_lines_stable_frame(
    tiles: &[&InteractiveGlyph],
    threshold: u8,
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
                matrix[dst_idx] = tile.glyph.coverage[src_idx] >= threshold;
            }
        }
    }

    render_binary_preview_lines(&matrix, width, height, max_w, max_h, true, false)
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
    let out_w = ((usize::max(1, base_w) as f32) * PREVIEW_X_COMP)
        .round()
        .max(1.0) as usize;
    let out_h = usize::max(
        1,
        if allow_upscale {
            max_h
        } else {
            src_h.min(max_h)
        },
    );
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
            if glyph.coverage[idx] >= threshold {
                matrix[idx] = true;
            }
        }
    }

    let Some((cropped, crop_w, crop_h)) =
        crop_binary_matrix_to_active_y_bounds(&matrix, src_w, src_h)
    else {
        return vec![Line::from("    [No visible pixels at threshold]")];
    };

    render_binary_preview_lines(&cropped, crop_w, crop_h, max_w, max_h, true, true)
}

fn preview_lines_stable_frame(
    glyph: &PreprocessedGlyph,
    threshold: u8,
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
            if glyph.coverage[idx] >= threshold {
                matrix[idx] = true;
            }
        }
    }

    render_binary_preview_lines(&matrix, src_w, src_h, max_w, max_h, true, false)
}

fn looks_like_path_payload(payload: &str) -> bool {
    let trimmed = payload.trim();
    if trimmed.is_empty() {
        return false;
    }
    trimmed.contains('/') || trimmed.starts_with("file://") || trimmed.contains('\\')
}

fn count_supported_sources(input_dir: &Path) -> Result<usize> {
    if !input_dir.exists() {
        return Ok(0);
    }

    let mut count = 0usize;
    for entry in WalkDir::new(input_dir).follow_links(true) {
        let entry =
            entry.with_context(|| format!("failed while scanning {}", input_dir.display()))?;
        if entry.file_type().is_file() && is_supported_source(entry.path()) {
            count += 1;
        }
    }

    Ok(count)
}

fn glyph_source_fingerprint(input_dir: &Path) -> Result<u64> {
    if !input_dir.exists() {
        return Ok(0);
    }

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for entry in WalkDir::new(input_dir).follow_links(true) {
        let entry =
            entry.with_context(|| format!("failed while scanning {}", input_dir.display()))?;
        if !entry.file_type().is_file() || !is_supported_source(entry.path()) {
            continue;
        }

        entry.path().hash(&mut hasher);
        let metadata = entry
            .metadata()
            .with_context(|| format!("failed to read metadata for {}", entry.path().display()))?;
        metadata.len().hash(&mut hasher);
        let modified = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|value| value.as_nanos())
            .unwrap_or_default();
        modified.hash(&mut hasher);
    }

    Ok(hasher.finish())
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

fn import_image_files_to_input(input_dir: &Path, payload: &str) -> Result<DropImportResult> {
    fs::create_dir_all(input_dir)
        .with_context(|| format!("failed to create {}", input_dir.display()))?;

    let dropped_paths = collect_dropped_paths(payload);
    if dropped_paths.is_empty() {
        bail!("drop did not include readable file paths");
    }

    let mut imported = 0usize;
    let mut renamed = 0usize;
    let mut skipped_existing = 0usize;
    let mut skipped_unsupported = 0usize;
    let mut skipped_missing = 0usize;
    let mut imported_source_keys = Vec::new();

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

        let canonical_destination = input_dir.join(file_name);
        if paths_resolve_to_same_file(&source, &canonical_destination) {
            skipped_existing += 1;
            continue;
        }

        let (destination, was_renamed) = next_available_import_destination(input_dir, file_name);
        fs::copy(&source, &destination).with_context(|| {
            format!(
                "failed to import {} into {}",
                source.display(),
                destination.display()
            )
        })?;

        imported_source_keys.push(source_key_from_input_path(input_dir, &destination));
        imported += 1;
        if was_renamed {
            renamed += 1;
        }
    }

    Ok(DropImportResult {
        imported,
        renamed,
        skipped_existing,
        skipped_unsupported,
        skipped_missing,
        imported_source_keys,
    })
}

fn load_interactive_glyphs_from_config(config: &RuntimeConfig) -> Result<LoadedGlyphs> {
    let mut sources = Vec::new();
    for entry in WalkDir::new(&config.input_dir).follow_links(true) {
        let entry = entry
            .with_context(|| format!("failed while scanning {}", config.input_dir.display()))?;
        if entry.file_type().is_file() && is_supported_source(entry.path()) {
            sources.push(entry.path().to_path_buf());
        }
    }
    sources.sort();

    let glyphs = preprocess_sources_with_compositions(
        &sources,
        &config.input_dir,
        config.glyph_size,
        &config.compositions,
    )?
    .into_iter()
    .map(|glyph| {
        let saved_threshold = config
            .threshold_overrides
            .get(&glyph.source_parent_key)
            .copied();
        let working_threshold = saved_threshold.unwrap_or(config.base_threshold);
        InteractiveGlyph {
            glyph,
            saved_threshold,
            working_threshold,
        }
    })
    .collect::<Vec<_>>();

    Ok(LoadedGlyphs {
        glyphs,
        source_fingerprint: glyph_source_fingerprint(&config.input_dir)?,
    })
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
    let centered_label = center_label("DRAG/PASTE IMAGES HERE", inner_width);
    let border_style = Style::default().fg(accent);
    let label_style = Style::default().fg(accent).add_modifier(Modifier::BOLD);

    let inner_rows = available_height.saturating_sub(2);
    let label_row = usize::from(inner_rows / 2);

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

fn pad_to_char_width(text: &str, target_chars: usize) -> String {
    let width = text.chars().count();
    if width >= target_chars {
        return text.to_string();
    }
    format!("{text}{}", " ".repeat(target_chars - width))
}

fn block_elements_reference_lines(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .map(|line| {
            line.chars()
                .map(|ch| if ch.is_whitespace() { ' ' } else { '█' })
                .collect::<String>()
        })
        .collect()
}

pub(crate) fn installed_font_block_display_lines_with_reference(
    block: &str,
    max_chars: usize,
) -> Vec<String> {
    let multiline = block.split('\n').nth(1).is_some();
    if !multiline {
        return installed_font_block_display_lines(block, max_chars);
    }

    let separator_width = INSTALLED_FONT_BLOCK_REFERENCE_SEPARATOR.chars().count();
    let available = max_chars.max(1);
    if available <= separator_width + 2 {
        return installed_font_block_display_lines(block, max_chars);
    }

    let pane_width = (available - separator_width) / 2;
    let left_width = pane_width.max(1);
    let right_width = (available - separator_width - left_width).max(1);

    let left_lines = installed_font_block_display_lines(block, left_width);
    if left_lines.is_empty() {
        return left_lines;
    }
    let reference_lines = block_elements_reference_lines(&left_lines);
    let mut out = Vec::with_capacity(left_lines.len().max(reference_lines.len()));
    for idx in 0..left_lines.len().max(reference_lines.len()) {
        let left = left_lines.get(idx).cloned().unwrap_or_default();
        let mut right = reference_lines.get(idx).cloned().unwrap_or_default();
        right = right.chars().take(right_width).collect::<String>();
        out.push(format!(
            "{}{}{}",
            pad_to_char_width(&left, left_width),
            INSTALLED_FONT_BLOCK_REFERENCE_SEPARATOR,
            right
        ));
    }
    out
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
        .map(|block| installed_font_block_display_lines_with_reference(block, max_chars))
}

#[cfg(test)]
mod tests {
    use super::{
        AnimationConfig, AnimationConfigFocus, AnimationType, App, AppView, BleedLevel, Input,
        InteractiveGlyph, KeyCode, RuntimeConfig, drag_images_here_lines,
        glyph_matches_animation_frame_source, handle_key, installed_animation_frame_index,
        installed_animation_source_block, preview_lines,
        scrollbar_thumb_geometry, visible_window_bounds,
    };
    use crate::build::PreprocessedGlyph;
    use crate::project::{Manifest, read_manifest, write_manifest};
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    fn make_temp_dir(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time is valid")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("petiglyph-tui-{name}-{nonce}"));
        fs::create_dir_all(&dir).expect("temp dir is created");
        dir
    }

    #[test]
    fn first_install_notice_must_be_dismissed_before_global_shortcuts_resume() {
        let project_dir = make_temp_dir("first-install-notice");
        let manifest_path = project_dir.join("petiglyph.toml");
        let config = RuntimeConfig {
            project_dir: project_dir.clone(),
            project_id: "test-first-install-popup".to_string(),
            input_dir: project_dir.join("icons"),
            out_dir: project_dir.join("build"),
            font_name: "Petiglyph".to_string(),
            glyph_size: 8,
            base_threshold: 64,
            threshold_overrides: BTreeMap::new(),
            compositions: BTreeMap::new(),
            animations: Vec::new(),
            codepoint_start: 0x10_0000,
        };

        let mut app = App::new(manifest_path, config);
        app.view = AppView::Welcome;
        app.first_install_notice_open = true;

        handle_key(&mut app, KeyCode::Char('q')).expect("popup should intercept quit");
        assert!(!app.quit, "quit should not fire while popup is open");
        assert!(
            app.first_install_notice_open,
            "non-dismiss keys should keep popup open"
        );

        handle_key(&mut app, KeyCode::Char(' ')).expect("space should dismiss popup");
        assert!(
            !app.first_install_notice_open,
            "space should close first-install popup"
        );

        handle_key(&mut app, KeyCode::Char('q')).expect("quit should work once popup is closed");
        assert!(app.quit, "quit should resume after popup dismissal");

        fs::remove_dir_all(project_dir).expect("temp dir is removed");
    }

    #[test]
    fn verbose_paths_toggle_switches_with_v_shortcut() {
        let project_dir = make_temp_dir("verbose-toggle");
        let manifest_path = project_dir.join("petiglyph.toml");
        let config = RuntimeConfig {
            project_dir: project_dir.clone(),
            project_id: "test-verbose-toggle".to_string(),
            input_dir: project_dir.join("icons"),
            out_dir: project_dir.join("build"),
            font_name: "Petiglyph".to_string(),
            glyph_size: 8,
            base_threshold: 64,
            threshold_overrides: BTreeMap::new(),
            compositions: BTreeMap::new(),
            animations: Vec::new(),
            codepoint_start: 0x10_0000,
        };

        let mut app = App::new(manifest_path, config);
        assert!(!app.verbose_paths, "verbose paths should default to off");

        handle_key(&mut app, KeyCode::Char('v')).expect("v should toggle verbose paths on");
        assert!(app.verbose_paths, "verbose paths should toggle on");

        handle_key(&mut app, KeyCode::Char('V')).expect("V should toggle verbose paths off");
        assert!(!app.verbose_paths, "verbose paths should toggle off");

        fs::remove_dir_all(project_dir).expect("temp dir is removed");
    }

    #[test]
    fn installed_animation_preview_advances_one_frame_at_a_time() {
        let started_at = Instant::now();

        assert_eq!(
            installed_animation_frame_index(4, 3, started_at, started_at),
            0
        );
        assert_eq!(
            installed_animation_frame_index(
                4,
                3,
                started_at,
                started_at + Duration::from_millis(250),
            ),
            1
        );
        assert_eq!(
            installed_animation_frame_index(
                4,
                3,
                started_at,
                started_at + Duration::from_millis(500),
            ),
            2
        );
        assert_eq!(
            installed_animation_frame_index(
                4,
                3,
                started_at,
                started_at + Duration::from_millis(750),
            ),
            0
        );
    }

    #[test]
    fn installed_animation_source_block_remaps_unambiguous_compose_row_col() {
        let by_source = BTreeMap::from([
            (
                "strip.png#compose:1x4:0:0".to_string(),
                "U+100000".to_string(),
            ),
            (
                "strip.png#compose:1x4:0:1".to_string(),
                "U+100001".to_string(),
            ),
        ]);
        let block0 = installed_animation_source_block(&by_source, "strip.png#compose:1x4:0:0");
        let block1 = installed_animation_source_block(&by_source, "strip.png#compose:1x4:0:1");

        assert_eq!(block0, Some(char::from_u32(0x100000).unwrap().to_string()));
        assert_eq!(block1, Some(char::from_u32(0x100001).unwrap().to_string()));
    }

    #[test]
    fn glyph_matches_animation_frame_source_requires_matching_grid_dims() {
        let glyph = InteractiveGlyph {
            glyph: PreprocessedGlyph {
                source_path: PathBuf::from("icons/strip.png"),
                source_key: "strip.png#compose:1x4:0:1".to_string(),
                source_parent_key: "strip.png".to_string(),
                glyph_name: "strip_r1_c2".to_string(),
                width: 8,
                height: 8,
                coverage: vec![0; 64],
                image_fingerprint: "fnv1a64:test".to_string(),
                composition_tile: None,
            },
            working_threshold: 64,
            saved_threshold: None,
        };

        assert!(!glyph_matches_animation_frame_source(
            &glyph,
            "strip.png#compose:1x2:0:1"
        ));
        assert!(glyph_matches_animation_frame_source(
            &glyph,
            "strip.png#compose:1x4:0:1"
        ));
    }

    #[test]
    fn installed_animation_source_block_does_not_remap_ambiguous_compose_row_col() {
        let by_source = BTreeMap::from([
            (
                "strip.png#compose:1x4:0:1".to_string(),
                "U+100001".to_string(),
            ),
            (
                "strip.png#compose:1x8:0:1".to_string(),
                "U+1000AA".to_string(),
            ),
        ]);
        let block = installed_animation_source_block(&by_source, "strip.png#compose:1x2:0:1");
        assert_eq!(block, None);
    }

    #[test]
    fn verbose_paths_toggle_does_not_fire_while_typing_project_name() {
        let project_dir = make_temp_dir("verbose-toggle-input");
        let manifest_path = project_dir.join("petiglyph.toml");
        let config = RuntimeConfig {
            project_dir: project_dir.clone(),
            project_id: "test-verbose-toggle-input".to_string(),
            input_dir: project_dir.join("icons"),
            out_dir: project_dir.join("build"),
            font_name: "Petiglyph".to_string(),
            glyph_size: 8,
            base_threshold: 64,
            threshold_overrides: BTreeMap::new(),
            compositions: BTreeMap::new(),
            animations: Vec::new(),
            codepoint_start: 0x10_0000,
        };

        let mut app = App::new(manifest_path, config);
        app.view = AppView::Welcome;
        app.welcome_focus = super::WelcomeFocus::CreateInput;
        app.welcome_input_editing = true;

        handle_key(&mut app, KeyCode::Char('v')).expect("typing should accept v");
        assert!(
            !app.verbose_paths,
            "verbose paths should not toggle during project-name typing"
        );
        assert_eq!(
            app.create_input.value(),
            "v",
            "v should be inserted into input"
        );

        fs::remove_dir_all(project_dir).expect("temp dir is removed");
    }

    #[test]
    fn verbose_paths_toggle_is_focusable_with_arrows_and_enter() {
        let project_dir = make_temp_dir("verbose-toggle-focus");
        let manifest_path = project_dir.join("petiglyph.toml");
        let config = RuntimeConfig {
            project_dir: project_dir.clone(),
            project_id: "test-verbose-toggle-focus".to_string(),
            input_dir: project_dir.join("icons"),
            out_dir: project_dir.join("build"),
            font_name: "Petiglyph".to_string(),
            glyph_size: 8,
            base_threshold: 64,
            threshold_overrides: BTreeMap::new(),
            compositions: BTreeMap::new(),
            animations: Vec::new(),
            codepoint_start: 0x10_0000,
        };

        let mut app = App::new(manifest_path, config);
        app.view = AppView::Welcome;
        assert!(!app.verbose_paths, "verbose paths should start off");

        for focus in [
            super::WelcomeFocus::BuildButton,
            super::WelcomeFocus::InstallButton,
            super::WelcomeFocus::DeleteProjectButton,
        ] {
            app.welcome_focus = focus;
            handle_key(&mut app, KeyCode::Up)
                .expect("up should move from current-project actions to settings");
            assert_eq!(
                app.welcome_focus,
                super::WelcomeFocus::VerbosePathsToggle,
                "settings toggle should be focusable from current-project actions"
            );
            handle_key(&mut app, KeyCode::Down)
                .expect("down from settings should return to install action");
            assert_eq!(
                app.welcome_focus,
                super::WelcomeFocus::InstallButton,
                "down from settings should land on install (not delete)"
            );
        }

        app.welcome_focus = super::WelcomeFocus::VerbosePathsToggle;
        handle_key(&mut app, KeyCode::Enter).expect("enter should toggle settings row");
        assert!(
            app.verbose_paths,
            "enter on settings should toggle verbose paths"
        );

        fs::remove_dir_all(project_dir).expect("temp dir is removed");
    }

    #[test]
    fn visible_window_bounds_center_and_clamp_selection() {
        assert_eq!(visible_window_bounds(0, 0, 5), (0, 0));
        assert_eq!(visible_window_bounds(5, 0, 10), (0, 5));
        assert_eq!(visible_window_bounds(20, 0, 5), (0, 5));
        assert_eq!(visible_window_bounds(20, 10, 5), (8, 13));
        assert_eq!(visible_window_bounds(20, 99, 5), (15, 20));
    }

    #[test]
    fn scrollbar_thumb_geometry_tracks_start_and_end_positions() {
        assert_eq!(scrollbar_thumb_geometry(0, 10, 0), (0, 0));
        assert_eq!(scrollbar_thumb_geometry(5, 10, 0), (0, 0));
        assert_eq!(scrollbar_thumb_geometry(100, 10, 0), (0, 1));
        assert_eq!(scrollbar_thumb_geometry(100, 10, 90), (9, 1));
    }

    #[test]
    fn standard_preview_upscales_cropped_cell_to_preview_viewport() {
        let mut coverage = vec![0; 32 * 64];
        for y in 15..49 {
            for x in 0..32 {
                coverage[y * 32 + x] = 255;
            }
        }
        let glyph = PreprocessedGlyph {
            source_path: PathBuf::from("codex-1.png"),
            source_key: "codex-1.png".to_string(),
            source_parent_key: "codex-1.png".to_string(),
            glyph_name: "codex_1".to_string(),
            width: 32,
            height: 64,
            coverage,
            image_fingerprint: "test".to_string(),
            composition_tile: None,
        };

        let lines = preview_lines(&glyph, 64, 37, 37);
        let rendered = lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        assert_eq!(rendered.len(), 37);
        assert!(
            rendered.iter().all(|line| line.chars().count() == 70),
            "37-cell preview width with x compensation should render as 4 spaces + 33 block pairs"
        );
    }

    #[test]
    fn create_grid_animation_sorts_frames_alphabetically_before_persisting() {
        let project_dir = make_temp_dir("animation-frame-sort");
        let manifest_path = project_dir.join("petiglyph.toml");
        write_manifest(&manifest_path, &Manifest::default()).expect("manifest is written");

        let icons_dir = project_dir.join("icons");
        fs::create_dir_all(&icons_dir).expect("icons dir is created");

        let config = RuntimeConfig {
            project_dir: project_dir.clone(),
            project_id: "test-animation-frame-sort".to_string(),
            input_dir: icons_dir,
            out_dir: project_dir.join("build"),
            font_name: "Petiglyph".to_string(),
            glyph_size: 8,
            base_threshold: 64,
            threshold_overrides: BTreeMap::new(),
            compositions: BTreeMap::new(),
            animations: Vec::new(),
            codepoint_start: 0x10_0000,
        };

        let mut app = App::new(manifest_path.clone(), config);
        let animation_config = AnimationConfig {
            selected_frames: vec![
                "f_03.png".to_string(),
                "f_01.png".to_string(),
                "f_02.png".to_string(),
            ],
            name_input: Input::new("walk".to_string()),
            animation_type: AnimationType::Grid,
            fps: 8,
            rows: 1,
            cols: 1,
            horizontal_bleed: BleedLevel::Weak,
            vertical_bleed: BleedLevel::Off,
            focus: AnimationConfigFocus::Name,
        };

        app.create_animation_from_config(&animation_config)
            .expect("animation should persist");

        let manifest = read_manifest(&manifest_path).expect("manifest reloads");
        assert_eq!(manifest.animations.len(), 1);
        assert_eq!(
            manifest.animations[0].frames,
            vec![
                "f_01.png".to_string(),
                "f_02.png".to_string(),
                "f_03.png".to_string()
            ]
        );

        fs::remove_dir_all(project_dir).expect("temp dir is removed");
    }

    #[test]
    fn drag_images_placeholder_handles_small_and_regular_regions() {
        assert!(
            drag_images_here_lines(6, 2, ratatui::style::Color::Cyan).is_empty(),
            "very small regions should skip drag placeholder rendering"
        );

        let lines = drag_images_here_lines(40, 7, ratatui::style::Color::Cyan);
        assert_eq!(lines.len(), 7, "placeholder should fill requested height");
        let rendered = lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();
        assert!(
            rendered
                .iter()
                .any(|line| line.contains("DRAG/PASTE IMAGES HERE")),
            "placeholder body should include drag/paste label"
        );
    }
}
