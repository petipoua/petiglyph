const CLI_VERSION: &str = env!("CARGO_PKG_VERSION");
const INSTALL_SPINNER_FRAMES: [&str; 4] = ["-", "\\", "|", "/"];
const UNINSTALL_SPINNER_FRAMES: [&str; 4] = ["/", "|", "\\", "-"];
const ANIMATION_IMPORT_SPINNER_FRAMES: [&str; 4] = ["-", "\\", "|", "/"];
const FONT_TASK_SPINNER_FRAME_MS: u64 = 43;
const WELCOME_SAMPLE_LIMIT: usize = 15;
const WELCOME_INPUT_WIDTH: usize = 15;
const SWITCH_NOTICE_MS: u64 = 2500;
const EVENT_POLL_MS: u64 = 33;
const TUI_DEBUG_LOG_ENV: &str = "PETIGLYPH_TUI_DEBUG_LOG";
const MOCK_CLIPBOARD_ENV: &str = "PETIGLYPH_MOCK_CLIPBOARD";
const TUI_DEBUG_LOG_FILE_NAME: &str = "petiglyph-tui-debug.log";
const TUI_MIN_WIDTH: u16 = 96;
const TUI_MIN_HEIGHT: u16 = 40;
const TUI_MAX_WIDTH: u16 = 148;
const TUI_MAX_HEIGHT: u16 = 92;
const GLYPHS_PANEL_MAX_HEIGHT: u16 = 45;
const DECPNM_NUMERIC_KEYPAD_MODE: &str = "\x1B>";
const WELCOME_HINT_WIDTH: usize = 27;
const DELETE_CONFIRM_CANCEL_INDEX: usize = 0;
const DELETE_CONFIRM_DELETE_INDEX: usize = 1;
const HTY_FULL_REPAINT_ENV: &str = "PETIGLYPH_TUI_HTY_FULL_REPAINT";
const GLYPH_SOURCE_COUNT_REFRESH_MS: u64 = 300;
const INSTALL_METADATA_PREFIX: &str = ".petiglyph-install-";
const INSTALL_METADATA_SUFFIX: &str = ".json";
const DEBUG_LOG_VISIBLE_LINES: usize = 6;
const GRAYSCALE_BRIGHTNESS_MIN: i16 = -80;
const GRAYSCALE_BRIGHTNESS_MAX: i16 = 80;
const GRAYSCALE_CONTRAST_MIN: i16 = -80;
const GRAYSCALE_CONTRAST_MAX: i16 = 80;
const GRAYSCALE_GAMMA_MIN: u16 = 50;
const GRAYSCALE_GAMMA_MAX: u16 = 200;
const EXPORT_TEST_FRAMES_MIN: u16 = 1;
const EXPORT_TEST_FRAMES_MAX: u16 = 120;
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
    manifest_warning: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct InstalledFontSample {
    pub(crate) file_name: String,
    pub(crate) path: PathBuf,
    pub(crate) blocks: Vec<InstalledFontBlock>,
    pub(crate) animation_rows: Vec<String>,
    pub(crate) animation_previews: Vec<InstalledFontAnimationPreview>,
    pub(crate) animation_exports: Vec<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct InstalledFontBlock {
    pub(crate) label: String,
    pub(crate) block: String,
    pub(crate) export: String,
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
    grayscale_processing: Option<animation_media::AnimationImportProcessingOptions>,
    #[serde(default)]
    uniform_threshold: Option<u8>,
    #[serde(default)]
    variable_threshold: bool,
    #[serde(default)]
    frame_blocks: Vec<String>,
}

type InstalledFontSamplePayload = (
    Vec<InstalledFontBlock>,
    Vec<String>,
    Vec<InstalledFontAnimationPreview>,
    Vec<String>,
);

enum InstalledFontMetadataSample {
    Matched(InstalledFontSamplePayload),
    MissingSampleForMatchedMetadata,
    NoMetadataMatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WelcomeFocus {
    PanelTabs,
    VerbosePathsToggle,
    ProjectList,
    CreateInput,
    HomeCreateButtons,
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

    reset_tui_debug_log();
    let mut session = TerminalSession::start()?;
    let (startup_tx, startup_rx) = mpsc::channel();
    thread::spawn(move || {
        let result = App::new_workspace(workspace_root, initial_manifest, launch_overrides);
        let _ = startup_tx.send(result);
    });

    let started_at = Instant::now();
    let mut spinner_frame = 0usize;
    let mut app = loop {
        match startup_rx.try_recv() {
            Ok(result) => break result?,
            Err(TryRecvError::Empty) => {
                session
                    .terminal
                    .draw(|frame| draw_startup_loading(frame, spinner_frame, started_at.elapsed()))?;
                spinner_frame = spinner_frame.wrapping_add(1);
                thread::sleep(Duration::from_millis(80));
            }
            Err(TryRecvError::Disconnected) => {
                bail!("application startup worker stopped unexpectedly");
            }
        }
    };

    tui_debug_log("tui.start", app_debug_state(&app));

    let mut log_next_draw_after_esc = false;
    while !app.quit {
        app.poll_font_task();
        app.poll_project_switch_task();
        app.poll_animation_import_task();
        app.poll_animation_create_task();
        app.poll_home_import_task();
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

fn draw_startup_loading(frame: &mut Frame<'_>, spinner_frame: usize, elapsed: Duration) {
    const SPINNER: &[char] = &['|', '/', '-', '\\'];
    let spinner = SPINNER[spinner_frame % SPINNER.len()];
    let detail = if elapsed >= Duration::from_secs(1) {
        "Loading projects, installed fonts, and glyphs..."
    } else {
        "Loading..."
    };
    let paragraph = Paragraph::new(vec![
        Line::from(Span::styled(
            "petiglyph",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(format!("{spinner} {detail}")),
    ])
    .alignment(Alignment::Center);

    let frame_area = frame.area();
    let width = frame_area.width.min(54);
    let height = frame_area.height.min(7);
    let area = Rect::new(
        frame_area.x + frame_area.width.saturating_sub(width) / 2,
        frame_area.y + frame_area.height.saturating_sub(height) / 2,
        width,
        height,
    );
    frame.render_widget(Clear, area);
    frame.render_widget(
        paragraph.block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded),
        ),
        area,
    );
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AppView {
    Welcome,
    Glyphs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GlyphsFocus {
    PanelTabs,
    InstallButton,
    List,
    Preview,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HomeCreationKind {
    Glyph,
    Grid,
    AnimatedGlyph,
    AnimatedGridGlyph,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HomeLauncherFocus {
    CreateGlyph,
    CreateGrid,
    CreateAnimatedGlyph,
    CreateAnimatedGridGlyph,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HomeWorkflow {
    Launcher,
    Import(HomeCreationKind),
    Tweaking(HomeCreationKind),
    ConfigureGrid,
    ConfigureAnimation(AnimationType),
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
    Fps,
    Rows,
    Cols,
    HorizontalBleed,
    VerticalBleed,
    Create,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AnimationImportSettingsFocus {
    GrayscaleToggle,
    GrayscaleOptionsButton,
    Threshold,
    FramesButton,
    ExportTestImageButton,
    Continue,
    Back,
    SkipAll,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GrayscaleKnobFocus {
    Brightness,
    Contrast,
    Gamma,
}

#[derive(Debug, Clone)]
struct GrayscaleOptionsEditor {
    original: animation_media::AnimationGrayscaleOptions,
    draft: animation_media::AnimationGrayscaleOptions,
    focus: GrayscaleKnobFocus,
}

#[derive(Debug, Clone)]
struct AnimationImportSettingsState {
    focus: AnimationImportSettingsFocus,
    grayscale_enabled: bool,
    grayscale_options: animation_media::AnimationGrayscaleOptions,
    threshold: u8,
    grayscale_editor: Option<GrayscaleOptionsEditor>,
    export_frame_count: u16,
    preview_frame_index: usize,
    last_exported_test_image: Option<PathBuf>,
}

impl Default for AnimationImportSettingsState {
    fn default() -> Self {
        Self {
            focus: AnimationImportSettingsFocus::Continue,
            grayscale_enabled: true,
            grayscale_options: animation_media::AnimationGrayscaleOptions::default(),
            threshold: 64,
            grayscale_editor: None,
            export_frame_count: 5,
            preview_frame_index: 0,
            last_exported_test_image: None,
        }
    }
}

#[derive(Debug, Clone)]
struct AnimationConfig {
    selected_frames: Vec<String>,
    animation_name: String,
    animation_type: AnimationType,
    fps: u8,
    rows: u32,
    cols: u32,
    horizontal_bleed: BleedLevel,
    vertical_bleed: BleedLevel,
    grayscale_processing: Option<animation_media::AnimationImportProcessingOptions>,
    focus: AnimationConfigFocus,
}

#[derive(Debug, Clone)]
struct AnimationPreview {
    animation_name: String,
    frame_index: usize,
    last_frame_at: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GlyphPreviewControl {
    Threshold,
    Fps,
    Invert,
}

#[derive(Debug, Clone)]
enum GlyphToolMode {
    None,
    ChooseAnimationType { focus: AnimationTypeChoiceFocus },
    ImportAnimationFrames,
    SelectAnimationFrames(AnimationType),
    ConfigureAnimation(AnimationConfig),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct LivePreviewCoverageKey {
    source_path: PathBuf,
    file_len: u64,
    file_modified_ns: Option<u128>,
    glyph_size: u32,
    grayscale_enabled: bool,
    grayscale_brightness: i16,
    grayscale_contrast: i16,
    grayscale_gamma_percent: u16,
    fit_mode: SourceFitMode,
}

#[derive(Debug, Default)]
struct LivePreviewCoverageCache {
    entries: HashMap<LivePreviewCoverageKey, Vec<u8>>,
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
    pua_usage_summary: Option<crate::install::PuaUsageSummary>,
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
    expanded_animations: BTreeSet<String>,
    pub(crate) quit: bool,
    pub(crate) status: Option<String>,
    pub(crate) view: AppView,
    pub(crate) panel_selection: AppView,
    pub(crate) glyphs_focus: GlyphsFocus,
    pub(crate) grid_config: Option<GridConfig>,
    pub(crate) selecting_for_grid: bool,
    glyph_tool_mode: GlyphToolMode,
    glyph_preview_control: GlyphPreviewControl,
    live_preview_coverage_cache: RefCell<LivePreviewCoverageCache>,
    animation_selection_order: Vec<String>,
    animation_selection_set: BTreeSet<String>,
    animation_imported_set: BTreeSet<String>,
    animation_preview: Option<AnimationPreview>,
    selecting_for_animation_frames: bool,
    home_launcher_focus: HomeLauncherFocus,
    home_workflow: HomeWorkflow,
    home_workflow_import_count: usize,
    animation_import_settings: AnimationImportSettingsState,
    home_workflow_recent_imported_source_keys: Vec<String>,
    home_workflow_created_source_keys: Vec<String>,
    home_workflow_tweak_source_queue: Vec<String>,
    home_workflow_tweak_source_index: usize,
    home_workflow_grid_source_key: Option<String>,
    home_workflow_grid_inline_notice: Option<String>,
    home_workflow_error: Option<String>,
    discard_next_animation_import_result: bool,
    pub(crate) last_build: Option<BuildSummary>,
    pub(crate) last_sample: Option<String>,
    pub(crate) installed_font_path: Option<PathBuf>,
    delete_project_confirm_selection: Option<usize>,
    renaming_input: Option<Input>,
    renaming_original: Option<String>,
    first_install_notice_open: bool,
    launch_overrides: TuiLaunchOverrides,
    install_task: Option<InstallTask>,
    project_switch_task: Option<ProjectSwitchTask>,
    animation_import_task: Option<AnimationImportTask>,
    home_import_task: Option<HomeImportTask>,
    queued_drop_payload: Option<String>,
    animation_create_task: Option<AnimationCreateTask>,
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
    pub(crate) saved_invert: bool,
    pub(crate) working_invert: bool,
}

#[derive(Debug, Clone)]
enum VisibleGlyphRow {
    AnimationParent {
        animation_idx: usize,
    },
    AnimationFrame {
        animation_idx: usize,
        frame_idx: usize,
        source_key: String,
        glyph_idx: Option<usize>,
    },
    Single {
        glyph_idx: usize,
    },
    CompositionParent {
        source_key: String,
        rows: usize,
        cols: usize,
        emitted_cols: usize,
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

struct InstallTask {
    kind: FontTaskKind,
    receiver: Receiver<Result<InstallTaskOutput, String>>,
    spinner_index: usize,
    spinner_last_frame_at: Instant,
}

struct ProjectSwitchTask {
    target_manifest_path: PathBuf,
    receiver: Receiver<Result<ProjectSwitchTaskOutput, String>>,
    spinner_index: usize,
    spinner_last_frame_at: Instant,
}

struct AnimationImportTask {
    receiver: Receiver<Result<AnimationImportTaskOutput, String>>,
    spinner_index: usize,
    spinner_last_frame_at: Instant,
}

struct AnimationCreateTask {
    receiver: Receiver<Result<AnimationCreateTaskOutput, String>>,
    spinner_index: usize,
    spinner_last_frame_at: Instant,
}

struct HomeImportTask {
    receiver: Receiver<Result<DropImportResult, String>>,
    spinner_index: usize,
    spinner_last_frame_at: Instant,
}

#[derive(Debug, Clone)]
struct DropImportResult {
    imported: usize,
    renamed: usize,
    skipped_existing: usize,
    skipped_unsupported: usize,
    skipped_missing: usize,
    imported_source_keys: Vec<String>,
    created_source_keys: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExistingImportPolicy {
    Rename,
    ReuseIdentical,
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
    detail_status: Option<String>,
}

#[derive(Debug, Clone)]
struct AnimationCreateTaskOutput {
    name: String,
    duplicated_for_grid_conflicts: usize,
    config: RuntimeConfig,
    loaded: LoadedGlyphs,
    last_build: Option<BuildSummary>,
    last_sample: Option<String>,
    installed_font_path: Option<PathBuf>,
}

#[derive(Debug, Clone)]
enum InstallTaskOutput {
    Install {
        summary: Box<BuildSummary>,
        sample: Option<String>,
        installed_path: PathBuf,
        first_install_on_machine: bool,
    },
    Uninstall {
        status_message: String,
    },
}

#[derive(Debug, Clone)]
struct ProjectSwitchTaskOutput {
    manifest_path: PathBuf,
    config: RuntimeConfig,
    loaded: LoadedGlyphs,
    last_build: Option<BuildSummary>,
    last_sample: Option<String>,
    installed_font_path: Option<PathBuf>,
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
