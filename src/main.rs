use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use crossterm::ExecutableCommand;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use image::imageops::FilterType;
use image::{ImageBuffer, Rgba, RgbaImage};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, BorderType, List, ListItem, ListState, Paragraph, Tabs, Wrap};
use ratatui::{Frame, Terminal};
use resvg::tiny_skia::{Pixmap, Transform};
use resvg::usvg;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::env;
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;
use std::time::Duration;
use walkdir::WalkDir;

#[derive(Debug, Parser)]
#[command(
    name = "petiglyph",
    version,
    about = "TUI-first CLI for building self-contained monochrome glyph font projects."
)]
struct Cli {
    #[command(subcommand)]
    command: Option<CliCommand>,
}

#[derive(Debug, Subcommand)]
enum CliCommand {
    /// Create a new self-contained petiglyph project in the current directory.
    Create {
        /// Project directory name to create inside the current working directory.
        name: String,
        /// Skip the post-create prompt and do not launch the TUI.
        #[arg(long)]
        no_launch: bool,
    },
    /// Launch the petiglyph TUI for a project.
    Tui {
        /// Path to the manifest file. Defaults to ./petiglyph.toml.
        #[arg(short, long)]
        manifest: Option<PathBuf>,
        /// Override source image folder from manifest.
        #[arg(long)]
        input_dir: Option<PathBuf>,
        /// Override preview threshold (0-255).
        #[arg(long)]
        threshold: Option<u8>,
        /// Override glyph pixel size.
        #[arg(long)]
        glyph_size: Option<u32>,
        /// Override starting Unicode codepoint (for example U+100000).
        #[arg(long)]
        codepoint_start: Option<String>,
    },
    /// Build monochrome glyph previews, mapping metadata, and a BDF/TTF font.
    Build {
        /// Path to the manifest file. Defaults to ./petiglyph.toml.
        #[arg(short, long)]
        manifest: Option<PathBuf>,
        /// Override source image folder from manifest.
        #[arg(long)]
        input_dir: Option<PathBuf>,
        /// Override output directory from manifest.
        #[arg(short, long)]
        out_dir: Option<PathBuf>,
        /// Override monochrome threshold (0-255).
        #[arg(long)]
        threshold: Option<u8>,
        /// Override output glyph pixel size.
        #[arg(long)]
        glyph_size: Option<u32>,
        /// Override starting Unicode codepoint (for example U+100000).
        #[arg(long)]
        codepoint_start: Option<String>,
        /// Emit machine-readable JSON to stdout.
        #[arg(long)]
        json: bool,
    },
    /// Build the font and print the sample private-use string.
    Sample {
        /// Path to the manifest file. Defaults to ./petiglyph.toml.
        #[arg(short, long)]
        manifest: Option<PathBuf>,
        /// Override source image folder from manifest.
        #[arg(long)]
        input_dir: Option<PathBuf>,
        /// Override output directory from manifest.
        #[arg(short, long)]
        out_dir: Option<PathBuf>,
        /// Override preview threshold (0-255).
        #[arg(long)]
        threshold: Option<u8>,
        /// Override glyph pixel size.
        #[arg(long)]
        glyph_size: Option<u32>,
        /// Override starting Unicode codepoint (for example U+100000).
        #[arg(long)]
        codepoint_start: Option<String>,
        /// Emit machine-readable JSON to stdout.
        #[arg(long)]
        json: bool,
    },
    /// Build the font and install it into the user font directory.
    InstallFont {
        /// Path to the manifest file. Defaults to ./petiglyph.toml.
        #[arg(short, long)]
        manifest: Option<PathBuf>,
        /// Override source image folder from manifest.
        #[arg(long)]
        input_dir: Option<PathBuf>,
        /// Override output directory from manifest.
        #[arg(short, long)]
        out_dir: Option<PathBuf>,
        /// Override preview threshold (0-255).
        #[arg(long)]
        threshold: Option<u8>,
        /// Override glyph pixel size.
        #[arg(long)]
        glyph_size: Option<u32>,
        /// Override starting Unicode codepoint (for example U+100000).
        #[arg(long)]
        codepoint_start: Option<String>,
        /// Emit machine-readable JSON to stdout.
        #[arg(long)]
        json: bool,
    },
    /// Uninstall the previously installed font for this project scope.
    UninstallFont {
        /// Path to the manifest file. Defaults to ./petiglyph.toml.
        #[arg(short, long)]
        manifest: Option<PathBuf>,
        /// Emit machine-readable JSON to stdout.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Manifest {
    input_dir: String,
    out_dir: String,
    font_name: String,
    glyph_size: u32,
    threshold: u8,
    codepoint_start: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    threshold_overrides: BTreeMap<String, u8>,
}

impl Default for Manifest {
    fn default() -> Self {
        Self {
            input_dir: "icons".to_string(),
            out_dir: "build".to_string(),
            font_name: "Petiglyph".to_string(),
            glyph_size: 64,
            threshold: 64,
            codepoint_start: "U+100000".to_string(),
            threshold_overrides: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone)]
struct RuntimeConfig {
    input_dir: PathBuf,
    out_dir: PathBuf,
    font_name: String,
    glyph_size: u32,
    base_threshold: u8,
    threshold_overrides: BTreeMap<String, u8>,
    codepoint_start: u32,
}

#[derive(Debug, Clone)]
struct PreprocessedGlyph {
    source_path: PathBuf,
    source_key: String,
    glyph_name: String,
    size: u32,
    coverage: Vec<u8>,
}

#[derive(Debug, Clone)]
struct GlyphBitmap {
    size: u32,
    pixels: Vec<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
struct MappingEntry {
    glyph_name: String,
    source_file: String,
    codepoint: String,
}

#[derive(Debug, Clone)]
struct BuildSummary {
    glyph_count: usize,
    bdf_path: PathBuf,
    ttf_path: PathBuf,
    mapping_path: PathBuf,
    sample_path: PathBuf,
    previews_dir: PathBuf,
}

const CLI_VERSION: &str = env!("CARGO_PKG_VERSION");
const INSTALL_METADATA_FILE: &str = ".petiglyph-install.json";

#[derive(Debug, Serialize)]
struct ApiResponse<T: Serialize> {
    ok: bool,
    command: &'static str,
    version: &'static str,
    data: T,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<ApiErrorPayload>,
}

#[derive(Debug, Serialize)]
struct ApiErrorPayload {
    message: String,
    causes: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum FontPlatform {
    Linux,
    Macos,
    Windows,
}

#[derive(Debug, Serialize)]
struct BuildCommandData {
    manifest: String,
    input_dir: String,
    out_dir: String,
    font_name: String,
    glyph_count: usize,
    threshold: u8,
    threshold_overrides: usize,
    glyph_size: u32,
    codepoint_start: String,
    bdf: String,
    ttf: String,
    map: String,
    sample: String,
    previews: String,
}

#[derive(Debug, Serialize)]
struct SampleCommandData {
    build: BuildCommandData,
    sample_string: String,
}

#[derive(Debug, Serialize)]
struct InstallFontCommandData {
    build: BuildCommandData,
    platform: FontPlatform,
    install_dir: String,
    installed_ttf: String,
    replaced_previous_ttf_count: usize,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum UninstallOutcome {
    Removed,
    AlreadyAbsent,
}

#[derive(Debug, Serialize)]
struct UninstallFontCommandData {
    manifest: String,
    platform: FontPlatform,
    install_dir: String,
    outcome: UninstallOutcome,
    removed_ttf_count: usize,
}

#[derive(Debug, Serialize, Deserialize)]
struct InstalledFontMetadata {
    manifest_path: String,
    font_name: String,
    installed_ttf: String,
    version: String,
}

enum CliRunError {
    Plain(anyhow::Error),
    Json {
        command: &'static str,
        error: anyhow::Error,
    },
}

fn main() {
    let cli = Cli::parse();
    let exit_code = match run_cli(cli) {
        Ok(()) => 0,
        Err(CliRunError::Plain(error)) => {
            eprintln!("{error:#}");
            1
        }
        Err(CliRunError::Json { command, error }) => {
            emit_json_error(command, &error);
            1
        }
    };
    std::process::exit(exit_code);
}

fn run_cli(cli: Cli) -> std::result::Result<(), CliRunError> {
    match cli.command {
        None => {
            let manifest = manifest_path_from_option(None).map_err(CliRunError::Plain)?;
            tui(manifest, None, None, None, None).map_err(CliRunError::Plain)
        }
        Some(CliCommand::Create { name, no_launch }) => {
            create_project(&name, no_launch).map_err(CliRunError::Plain)
        }
        Some(CliCommand::Tui {
            manifest,
            input_dir,
            threshold,
            glyph_size,
            codepoint_start,
        }) => tui(
            manifest_path_from_option(manifest).map_err(CliRunError::Plain)?,
            input_dir,
            threshold,
            glyph_size,
            codepoint_start,
        )
        .map_err(CliRunError::Plain),
        Some(CliCommand::Build {
            manifest,
            input_dir,
            out_dir,
            threshold,
            glyph_size,
            codepoint_start,
            json,
        }) => run_automation_command(
            "build",
            json,
            || {
                build_font(
                    manifest_path_from_option(manifest)?,
                    input_dir,
                    out_dir,
                    threshold,
                    glyph_size,
                    codepoint_start,
                )
            },
            print_build_result,
        ),
        Some(CliCommand::Sample {
            manifest,
            input_dir,
            out_dir,
            threshold,
            glyph_size,
            codepoint_start,
            json,
        }) => run_automation_command(
            "sample",
            json,
            || {
                sample_command(
                    manifest_path_from_option(manifest)?,
                    input_dir,
                    out_dir,
                    threshold,
                    glyph_size,
                    codepoint_start,
                )
            },
            print_sample_result,
        ),
        Some(CliCommand::InstallFont {
            manifest,
            input_dir,
            out_dir,
            threshold,
            glyph_size,
            codepoint_start,
            json,
        }) => run_automation_command(
            "install-font",
            json,
            || {
                install_font_command(
                    manifest_path_from_option(manifest)?,
                    input_dir,
                    out_dir,
                    threshold,
                    glyph_size,
                    codepoint_start,
                )
            },
            print_install_result,
        ),
        Some(CliCommand::UninstallFont { manifest, json }) => run_automation_command(
            "uninstall-font",
            json,
            || uninstall_font_command(manifest_path_from_option(manifest)?),
            print_uninstall_result,
        ),
    }
}

fn run_automation_command<T, F, H>(
    command: &'static str,
    json: bool,
    operation: F,
    human_printer: H,
) -> std::result::Result<(), CliRunError>
where
    T: Serialize,
    F: FnOnce() -> Result<T>,
    H: FnOnce(&T),
{
    match operation() {
        Ok(data) => {
            if json {
                emit_json_success(command, &data);
            } else {
                human_printer(&data);
            }
            Ok(())
        }
        Err(error) => {
            if json {
                Err(CliRunError::Json { command, error })
            } else {
                Err(CliRunError::Plain(error))
            }
        }
    }
}

fn emit_json_success<T: Serialize>(command: &'static str, data: &T) {
    let payload = ApiResponse {
        ok: true,
        command,
        version: CLI_VERSION,
        data,
        error: None,
    };
    if let Ok(line) = serde_json::to_string(&payload) {
        println!("{line}");
    } else {
        println!(
            "{{\"ok\":true,\"command\":\"{command}\",\"version\":\"{CLI_VERSION}\",\"data\":{{}},\"error\":null}}"
        );
    }
}

fn emit_json_error(command: &'static str, error: &anyhow::Error) {
    let causes = error
        .chain()
        .skip(1)
        .map(|cause| cause.to_string())
        .collect::<Vec<_>>();
    let payload = ApiResponse {
        ok: false,
        command,
        version: CLI_VERSION,
        data: serde_json::json!({}),
        error: Some(ApiErrorPayload {
            message: error.to_string(),
            causes,
        }),
    };
    if let Ok(line) = serde_json::to_string(&payload) {
        println!("{line}");
    } else {
        println!(
            "{{\"ok\":false,\"command\":\"{command}\",\"version\":\"{CLI_VERSION}\",\"data\":{{}},\"error\":{{\"message\":\"failed to serialize error payload\",\"causes\":[]}}}}"
        );
    }
}

fn format_codepoint(codepoint: u32) -> String {
    format!("U+{:04X}", codepoint)
}

fn build_command_data(
    manifest_path: &Path,
    config: &RuntimeConfig,
    summary: &BuildSummary,
) -> BuildCommandData {
    BuildCommandData {
        manifest: manifest_path.display().to_string(),
        input_dir: config.input_dir.display().to_string(),
        out_dir: config.out_dir.display().to_string(),
        font_name: config.font_name.clone(),
        glyph_count: summary.glyph_count,
        threshold: config.base_threshold,
        threshold_overrides: config.threshold_overrides.len(),
        glyph_size: config.glyph_size,
        codepoint_start: format_codepoint(config.codepoint_start),
        bdf: summary.bdf_path.display().to_string(),
        ttf: summary.ttf_path.display().to_string(),
        map: summary.mapping_path.display().to_string(),
        sample: summary.sample_path.display().to_string(),
        previews: summary.previews_dir.display().to_string(),
    }
}

fn print_build_result(data: &BuildCommandData) {
    println!("build complete");
    println!("  manifest: {}", data.manifest);
    println!("  input-dir: {}", data.input_dir);
    println!("  out-dir: {}", data.out_dir);
    println!("  font: {}", data.font_name);
    println!("  glyphs: {}", data.glyph_count);
    println!("  threshold: {}", data.threshold);
    println!("  threshold-overrides: {}", data.threshold_overrides);
    println!("  glyph-size: {}", data.glyph_size);
    println!("  codepoint-start: {}", data.codepoint_start);
    println!("  bdf: {}", data.bdf);
    println!("  ttf: {}", data.ttf);
    println!("  map: {}", data.map);
    println!("  sample: {}", data.sample);
    println!("  previews: {}", data.previews);
}

fn print_sample_result(data: &SampleCommandData) {
    println!("petiglyph sample");
    println!("font: {}", data.build.font_name);
    println!("glyphs: {}", data.build.glyph_count);
    println!("threshold: {}", data.build.threshold);
    println!("threshold-overrides: {}", data.build.threshold_overrides);
    println!("glyph-size: {}", data.build.glyph_size);
    println!("ttf: {}", data.build.ttf);
    println!("sample: {}", data.build.sample);
    println!();
    println!("{}", data.sample_string);
}

fn print_install_result(data: &InstallFontCommandData) {
    println!("font installed");
    println!("  source: {}", data.build.ttf);
    println!("  installed: {}", data.installed_ttf);
    println!("  install-dir: {}", data.install_dir);
    println!(
        "  replaced-previous-ttfs: {}",
        data.replaced_previous_ttf_count
    );
}

fn print_uninstall_result(data: &UninstallFontCommandData) {
    match data.outcome {
        UninstallOutcome::Removed => {
            println!("font uninstalled");
            println!("  manifest: {}", data.manifest);
            println!("  install-dir: {}", data.install_dir);
            println!("  removed-ttfs: {}", data.removed_ttf_count);
        }
        UninstallOutcome::AlreadyAbsent => {
            println!("font already absent");
            println!("  manifest: {}", data.manifest);
            println!("  install-dir: {}", data.install_dir);
        }
    }
}

fn manifest_path_from_option(manifest: Option<PathBuf>) -> Result<PathBuf> {
    match manifest {
        Some(path) => Ok(path),
        None => {
            let current_dir = env::current_dir().context("failed to read current directory")?;
            let manifest_path = current_dir.join("petiglyph.toml");
            if manifest_path.exists() {
                Ok(manifest_path)
            } else {
                bail!(
                    "no petiglyph project found in {} (run `petiglyph create <name>` or pass --manifest)",
                    current_dir.display()
                );
            }
        }
    }
}

fn create_project(project_name: &str, no_launch: bool) -> Result<()> {
    if project_name.trim().is_empty() {
        bail!("project name cannot be empty");
    }

    let current_dir = env::current_dir().context("failed to read current directory")?;
    let project_dir = current_dir.join(project_name);
    if project_dir.exists() {
        bail!(
            "project directory already exists: {}",
            project_dir.display()
        );
    }

    let icons_dir = project_dir.join("icons");
    let out_dir = project_dir.join("build");
    fs::create_dir_all(&icons_dir)
        .with_context(|| format!("failed to create {}", icons_dir.display()))?;
    fs::create_dir_all(&out_dir)
        .with_context(|| format!("failed to create {}", out_dir.display()))?;

    let manifest_path = project_dir.join("petiglyph.toml");
    let display_name = project_dir
        .file_name()
        .and_then(|name| name.to_str())
        .map(humanize_project_name)
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| "Petiglyph".to_string());
    let manifest = Manifest {
        font_name: display_name,
        ..Manifest::default()
    };
    write_manifest(&manifest_path, &manifest)?;

    println!("created petiglyph project: {}", project_dir.display());
    println!("  project: {}", project_dir.display());
    println!("  manifest: {}", manifest_path.display());
    println!("  images: {}", icons_dir.display());
    println!("  build output: {}", out_dir.display());
    println!();
    println!("next steps:");
    println!("  1. add your source images to {}", icons_dir.display());
    println!("  2. run `cd {}`", project_dir.display());
    println!("  3. launch the TUI with `petiglyph` or `petiglyph tui`");

    if no_launch {
        return Ok(());
    }

    if io::stdin().is_terminal() && io::stdout().is_terminal() {
        println!();
        print!(
            "add your images to {}. press Enter to launch the TUI, or type `skip` to exit: ",
            icons_dir.display()
        );
        io::stdout().flush().context("failed to flush prompt")?;
        let mut answer = String::new();
        io::stdin()
            .read_line(&mut answer)
            .context("failed to read launch confirmation")?;
        if answer.trim().eq_ignore_ascii_case("skip") {
            return Ok(());
        }
        tui(manifest_path, None, None, None, None)
    } else {
        println!("non-interactive shell detected; skipping automatic TUI launch");
        Ok(())
    }
}

fn read_manifest(manifest_path: &Path) -> Result<Manifest> {
    let data = fs::read_to_string(manifest_path)
        .with_context(|| format!("failed to read {}", manifest_path.display()))?;
    toml::from_str(&data).with_context(|| format!("failed to parse {}", manifest_path.display()))
}

fn write_manifest(manifest_path: &Path, manifest: &Manifest) -> Result<()> {
    let data = toml::to_string_pretty(manifest).context("failed to serialize manifest")?;
    fs::write(manifest_path, data)
        .with_context(|| format!("failed to write {}", manifest_path.display()))
}

fn build_font(
    manifest_path: PathBuf,
    input_override: Option<PathBuf>,
    out_override: Option<PathBuf>,
    threshold_override: Option<u8>,
    glyph_size_override: Option<u32>,
    codepoint_start_override: Option<String>,
) -> Result<BuildCommandData> {
    let config = load_runtime_config(
        &manifest_path,
        input_override,
        out_override,
        threshold_override,
        glyph_size_override,
        codepoint_start_override,
    )?;

    if config.glyph_size == 0 {
        bail!("glyph_size must be > 0");
    }

    let summary = build_outputs(&config)?;
    Ok(build_command_data(&manifest_path, &config, &summary))
}

fn build_outputs(config: &RuntimeConfig) -> Result<BuildSummary> {
    let sources = collect_source_files(&config.input_dir)?;
    let glyphs = preprocess_sources(&sources, &config.input_dir, config.glyph_size)?;

    let previews_dir = config.out_dir.join("previews");
    fs::create_dir_all(&previews_dir)
        .with_context(|| format!("failed to create {}", previews_dir.display()))?;

    let mut mapping = Vec::with_capacity(glyphs.len());
    let mut bdf_glyphs = Vec::with_capacity(glyphs.len());

    for (idx, glyph) in glyphs.iter().enumerate() {
        let codepoint = config.codepoint_start + idx as u32;
        let threshold = effective_threshold(
            config.base_threshold,
            &config.threshold_overrides,
            &glyph.source_key,
        );
        let bitmap = threshold_bitmap(glyph, threshold);

        let preview_path = previews_dir.join(format!("{}.png", glyph.glyph_name));
        write_preview_png(&preview_path, &bitmap)
            .with_context(|| format!("failed to write {}", preview_path.display()))?;

        mapping.push(MappingEntry {
            glyph_name: glyph.glyph_name.clone(),
            source_file: glyph.source_key.clone(),
            codepoint: format!("U+{:04X}", codepoint),
        });

        bdf_glyphs.push((glyph.glyph_name.clone(), codepoint, bitmap));
    }

    fs::create_dir_all(&config.out_dir)
        .with_context(|| format!("failed to create {}", config.out_dir.display()))?;

    let mapping_path = config.out_dir.join("glyph-map.json");
    let mapping_json =
        serde_json::to_string_pretty(&mapping).context("failed to serialize mapping")?;
    fs::write(&mapping_path, mapping_json)
        .with_context(|| format!("failed to write {}", mapping_path.display()))?;

    let font_file_stem = expected_font_file_stem(&config.font_name);
    let bdf_path = config.out_dir.join(format!("{font_file_stem}.bdf"));
    write_bdf(&bdf_path, &config.font_name, config.glyph_size, &bdf_glyphs)?;

    let ttf_path = config.out_dir.join(format!("{font_file_stem}.ttf"));
    write_ttf(&ttf_path, &config.font_name, config.glyph_size, &bdf_glyphs)?;

    let sample_path = config.out_dir.join("glyph-sample.txt");
    let sample = glyph_sample_string(config.codepoint_start, bdf_glyphs.len());
    fs::write(&sample_path, format!("{sample}\n"))
        .with_context(|| format!("failed to write {}", sample_path.display()))?;

    Ok(BuildSummary {
        glyph_count: bdf_glyphs.len(),
        bdf_path,
        ttf_path,
        mapping_path,
        sample_path,
        previews_dir,
    })
}

fn expected_font_file_stem(font_name: &str) -> String {
    let slug = slugify(font_name);
    if slug.is_empty() {
        "petiglyph".to_string()
    } else {
        slug
    }
}

fn expected_ttf_path(config: &RuntimeConfig) -> PathBuf {
    config.out_dir.join(format!(
        "{}.ttf",
        expected_font_file_stem(&config.font_name)
    ))
}

fn expected_bdf_path(config: &RuntimeConfig) -> PathBuf {
    config.out_dir.join(format!(
        "{}.bdf",
        expected_font_file_stem(&config.font_name)
    ))
}

fn tui(
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

fn sample_command(
    manifest_path: PathBuf,
    input_override: Option<PathBuf>,
    out_override: Option<PathBuf>,
    threshold_override: Option<u8>,
    glyph_size_override: Option<u32>,
    codepoint_start_override: Option<String>,
) -> Result<SampleCommandData> {
    let config = load_runtime_config(
        &manifest_path,
        input_override,
        out_override,
        threshold_override,
        glyph_size_override,
        codepoint_start_override,
    )?;

    let summary = build_outputs(&config)?;
    let sample = fs::read_to_string(&summary.sample_path)
        .with_context(|| format!("failed to read {}", summary.sample_path.display()))?;
    Ok(SampleCommandData {
        build: build_command_data(&manifest_path, &config, &summary),
        sample_string: sample.trim_end().to_string(),
    })
}

fn install_font_command(
    manifest_path: PathBuf,
    input_override: Option<PathBuf>,
    out_override: Option<PathBuf>,
    threshold_override: Option<u8>,
    glyph_size_override: Option<u32>,
    codepoint_start_override: Option<String>,
) -> Result<InstallFontCommandData> {
    let config = load_runtime_config(
        &manifest_path,
        input_override,
        out_override,
        threshold_override,
        glyph_size_override,
        codepoint_start_override,
    )?;

    let summary = build_outputs(&config)?;
    let install_result = install_built_font(
        &manifest_path,
        &config.font_name,
        &summary.ttf_path,
        summary.glyph_count,
    )?;
    Ok(InstallFontCommandData {
        build: build_command_data(&manifest_path, &config, &summary),
        platform: install_result.platform,
        install_dir: install_result.install_dir.display().to_string(),
        installed_ttf: install_result.install_path.display().to_string(),
        replaced_previous_ttf_count: install_result.replaced_previous_ttf_count,
    })
}

fn uninstall_font_command(manifest_path: PathBuf) -> Result<UninstallFontCommandData> {
    let uninstall = uninstall_project_font(&manifest_path)?;
    Ok(UninstallFontCommandData {
        manifest: manifest_path.display().to_string(),
        platform: uninstall.platform,
        install_dir: uninstall.install_dir.display().to_string(),
        outcome: uninstall.outcome,
        removed_ttf_count: uninstall.removed_ttf_count,
    })
}

fn load_runtime_config(
    manifest_path: &Path,
    input_override: Option<PathBuf>,
    out_override: Option<PathBuf>,
    threshold_override: Option<u8>,
    glyph_size_override: Option<u32>,
    codepoint_start_override: Option<String>,
) -> Result<RuntimeConfig> {
    let manifest = read_manifest(manifest_path)?;

    let base = manifest_path.parent().unwrap_or_else(|| Path::new("."));

    let input_dir = input_override.unwrap_or_else(|| base.join(&manifest.input_dir));
    let out_dir = out_override.unwrap_or_else(|| base.join(&manifest.out_dir));

    let base_threshold = threshold_override.unwrap_or(manifest.threshold);
    let glyph_size = glyph_size_override.unwrap_or(manifest.glyph_size);

    let codepoint_start = parse_codepoint(
        codepoint_start_override
            .as_deref()
            .unwrap_or(&manifest.codepoint_start),
    )?;

    Ok(RuntimeConfig {
        input_dir,
        out_dir,
        font_name: manifest.font_name,
        glyph_size,
        base_threshold,
        threshold_overrides: manifest.threshold_overrides,
        codepoint_start,
    })
}

fn humanize_project_name(project_name: &str) -> String {
    let mut out = String::new();
    let mut capitalize = true;
    for ch in project_name.chars() {
        if matches!(ch, '-' | '_' | ' ') {
            if !out.ends_with(' ') && !out.is_empty() {
                out.push(' ');
            }
            capitalize = true;
            continue;
        }

        if capitalize {
            for upper in ch.to_uppercase() {
                out.push(upper);
            }
            capitalize = false;
        } else {
            out.push(ch);
        }
    }

    let trimmed = out.trim();
    if trimmed.is_empty() {
        "Petiglyph".to_string()
    } else {
        trimmed.to_string()
    }
}

#[derive(Debug, Clone)]
struct FontInstallResult {
    platform: FontPlatform,
    install_dir: PathBuf,
    install_path: PathBuf,
    replaced_previous_ttf_count: usize,
}

#[derive(Debug, Clone)]
struct FontUninstallResult {
    platform: FontPlatform,
    install_dir: PathBuf,
    outcome: UninstallOutcome,
    removed_ttf_count: usize,
}

fn current_platform() -> Result<FontPlatform> {
    match env::consts::OS {
        "linux" => Ok(FontPlatform::Linux),
        "macos" => Ok(FontPlatform::Macos),
        "windows" => Ok(FontPlatform::Windows),
        other => bail!("font install/uninstall is not supported on this OS: {other}"),
    }
}

fn home_dir() -> Result<PathBuf> {
    if let Some(home) = env::var_os("HOME") {
        return Ok(PathBuf::from(home));
    }

    if let Some(profile) = env::var_os("USERPROFILE") {
        return Ok(PathBuf::from(profile));
    }

    bail!("cannot resolve user home directory from HOME or USERPROFILE")
}

fn user_font_root() -> Result<PathBuf> {
    let platform = current_platform()?;
    let home = home_dir()?;
    let root = match platform {
        FontPlatform::Linux => home.join(".local/share/fonts"),
        FontPlatform::Macos => home.join("Library/Fonts"),
        FontPlatform::Windows => {
            let local = env::var_os("LOCALAPPDATA")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join("AppData/Local"));
            local.join("Microsoft/Windows/Fonts")
        }
    };
    Ok(root)
}

fn install_dir_for_project(manifest_path: &Path, font_root: &Path) -> Result<PathBuf> {
    let project_dir = manifest_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .canonicalize()
        .with_context(|| format!("failed to resolve {}", manifest_path.display()))?;
    let project_name = project_dir
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or("project");
    Ok(font_root.join("petiglyph").join(slugify(project_name)))
}

fn install_dir_for_manifest(manifest_path: &Path) -> Result<PathBuf> {
    let font_root = user_font_root()?;
    install_dir_for_project(manifest_path, &font_root)
}

fn run_refresh_command(mut command: ProcessCommand, description: &str) -> Result<()> {
    let status = command
        .status()
        .with_context(|| format!("failed to run {description}"))?;
    if !status.success() {
        bail!("{description} failed with status {status}");
    }
    Ok(())
}

fn refresh_font_cache(font_root: &Path, platform: FontPlatform) -> Result<()> {
    match platform {
        FontPlatform::Linux => run_refresh_command(
            {
                let mut command = ProcessCommand::new("fc-cache");
                command.arg("-f").arg(font_root);
                command
            },
            "Linux font cache refresh (`fc-cache -f`)",
        ),
        FontPlatform::Macos => run_refresh_command(
            {
                let mut command = ProcessCommand::new("atsutil");
                command.arg("databases").arg("-removeUser");
                command
            },
            "macOS font cache refresh (`atsutil databases -removeUser`)",
        ),
        FontPlatform::Windows => run_refresh_command(
            {
                let mut command = ProcessCommand::new("powershell");
                command.arg("-NoProfile").arg("-Command").arg(
                    "$sig='[DllImport(\"user32.dll\", SetLastError=true)] public static extern IntPtr SendMessageTimeout(IntPtr hWnd, uint Msg, UIntPtr wParam, string lParam, uint fuFlags, uint uTimeout, out UIntPtr lpdwResult);'; $type = Add-Type -MemberDefinition $sig -Name User32SendMessageTimeout -Namespace Petiglyph -PassThru; $result=[UIntPtr]::Zero; [void]$type::SendMessageTimeout([IntPtr]0xffff, 0x001D, [UIntPtr]::Zero, $null, 2, 3000, [ref]$result)",
                );
                command
            },
            "Windows font change broadcast via PowerShell",
        ),
    }
}

fn font_file_name(font_name: &str) -> String {
    let slug = slugify(font_name);
    if slug.is_empty() {
        "petiglyph.ttf".to_string()
    } else {
        format!("{slug}.ttf")
    }
}

fn install_built_font(
    manifest_path: &Path,
    font_name: &str,
    built_ttf: &Path,
    glyph_count: usize,
) -> Result<FontInstallResult> {
    if glyph_count == 0 {
        bail!("cannot install a font with zero glyphs");
    }

    let platform = current_platform()?;
    let font_root = user_font_root()?;
    let install_dir = install_dir_for_project(manifest_path, &font_root)?;
    fs::create_dir_all(&install_dir)
        .with_context(|| format!("failed to create {}", install_dir.display()))?;

    let mut replaced_previous_ttf_count = 0usize;
    for entry in fs::read_dir(&install_dir)
        .with_context(|| format!("failed to read {}", install_dir.display()))?
    {
        let entry =
            entry.with_context(|| format!("failed to inspect {}", install_dir.display()))?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("ttf") {
            fs::remove_file(&path)
                .with_context(|| format!("failed to remove {}", path.display()))?;
            replaced_previous_ttf_count += 1;
        }
    }

    let file_name = font_file_name(font_name);
    let install_path = install_dir.join(file_name);
    fs::copy(built_ttf, &install_path).with_context(|| {
        format!(
            "failed to copy built font {} to {}",
            built_ttf.display(),
            install_path.display()
        )
    })?;

    let manifest_canonical = manifest_path
        .canonicalize()
        .with_context(|| format!("failed to resolve {}", manifest_path.display()))?;
    let metadata = InstalledFontMetadata {
        manifest_path: manifest_canonical.display().to_string(),
        font_name: font_name.to_string(),
        installed_ttf: install_path.display().to_string(),
        version: CLI_VERSION.to_string(),
    };
    let metadata_path = install_dir.join(INSTALL_METADATA_FILE);
    let metadata_json =
        serde_json::to_string_pretty(&metadata).context("failed to serialize install metadata")?;
    fs::write(&metadata_path, metadata_json)
        .with_context(|| format!("failed to write {}", metadata_path.display()))?;

    refresh_font_cache(&font_root, platform)?;
    Ok(FontInstallResult {
        platform,
        install_dir,
        install_path,
        replaced_previous_ttf_count,
    })
}

fn uninstall_project_font(manifest_path: &Path) -> Result<FontUninstallResult> {
    let platform = current_platform()?;
    let font_root = user_font_root()?;
    let install_dir = install_dir_for_project(manifest_path, &font_root)?;
    let manifest_canonical = manifest_path
        .canonicalize()
        .with_context(|| format!("failed to resolve {}", manifest_path.display()))?;

    if !install_dir.exists() {
        return Ok(FontUninstallResult {
            platform,
            install_dir,
            outcome: UninstallOutcome::AlreadyAbsent,
            removed_ttf_count: 0,
        });
    }

    let mut ttf_files = Vec::new();
    let mut metadata_path: Option<PathBuf> = None;
    for entry in fs::read_dir(&install_dir)
        .with_context(|| format!("failed to read {}", install_dir.display()))?
    {
        let entry =
            entry.with_context(|| format!("failed to inspect {}", install_dir.display()))?;
        let path = entry.path();

        if path.is_dir() {
            bail!(
                "blocked uninstall for {}: nested directory found: {}",
                install_dir.display(),
                path.display()
            );
        }

        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        if file_name == INSTALL_METADATA_FILE {
            metadata_path = Some(path);
            continue;
        }

        if path.extension().and_then(|ext| ext.to_str()) == Some("ttf") {
            ttf_files.push(path);
            continue;
        }

        bail!(
            "blocked uninstall for {}: refusing to remove unexpected file {}",
            install_dir.display(),
            path.display()
        );
    }

    if ttf_files.is_empty() && metadata_path.is_none() {
        if fs::read_dir(&install_dir)?.next().is_none() {
            fs::remove_dir(&install_dir)
                .with_context(|| format!("failed to remove {}", install_dir.display()))?;
        }
        return Ok(FontUninstallResult {
            platform,
            install_dir,
            outcome: UninstallOutcome::AlreadyAbsent,
            removed_ttf_count: 0,
        });
    }

    if let Some(metadata_path) = &metadata_path {
        let metadata_raw = fs::read_to_string(metadata_path)
            .with_context(|| format!("failed to read {}", metadata_path.display()))?;
        let metadata: InstalledFontMetadata = serde_json::from_str(&metadata_raw)
            .with_context(|| format!("failed to parse {}", metadata_path.display()))?;
        if metadata.manifest_path != manifest_canonical.display().to_string() {
            bail!(
                "blocked uninstall for {}: install metadata belongs to {}",
                install_dir.display(),
                metadata.manifest_path
            );
        }
    }

    for path in &ttf_files {
        fs::remove_file(path).with_context(|| format!("failed to remove {}", path.display()))?;
    }

    if let Some(metadata_path) = metadata_path {
        fs::remove_file(&metadata_path)
            .with_context(|| format!("failed to remove {}", metadata_path.display()))?;
    }

    if fs::read_dir(&install_dir)?.next().is_none() {
        fs::remove_dir(&install_dir)
            .with_context(|| format!("failed to remove {}", install_dir.display()))?;
    }

    refresh_font_cache(&font_root, platform)?;
    Ok(FontUninstallResult {
        platform,
        install_dir,
        outcome: UninstallOutcome::Removed,
        removed_ttf_count: ttf_files.len(),
    })
}

fn parse_codepoint(value: &str) -> Result<u32> {
    let raw = value.trim();
    if raw.is_empty() {
        bail!("codepoint_start cannot be empty");
    }

    let cleaned = raw
        .trim_start_matches("U+")
        .trim_start_matches("u+")
        .trim_start_matches("0x")
        .trim_start_matches("0X");

    let parsed = u32::from_str_radix(cleaned, 16)
        .with_context(|| format!("invalid codepoint_start: {raw}"))?;

    if parsed > 0x10_FFFF || (0xD800..=0xDFFF).contains(&parsed) {
        bail!("codepoint_start is not a valid Unicode scalar value: {raw}");
    }

    Ok(parsed)
}

fn collect_source_files(input_dir: &Path) -> Result<Vec<PathBuf>> {
    if !input_dir.exists() {
        bail!("input_dir does not exist: {}", input_dir.display());
    }

    let mut files = Vec::new();
    for entry in WalkDir::new(input_dir).follow_links(true) {
        let entry =
            entry.with_context(|| format!("failed while scanning {}", input_dir.display()))?;
        if !entry.file_type().is_file() {
            continue;
        }

        let path = entry.path();
        if is_supported_source(path) {
            files.push(path.to_path_buf());
        }
    }

    files.sort();
    if files.is_empty() {
        bail!("no supported images found in {}", input_dir.display());
    }

    Ok(files)
}

fn is_supported_source(path: &Path) -> bool {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some(ext) => {
            let ext = ext.to_ascii_lowercase();
            matches!(
                ext.as_str(),
                "png" | "jpg" | "jpeg" | "webp" | "avif" | "bmp" | "gif" | "svg"
            )
        }
        None => false,
    }
}

fn source_manifest_key(source: &Path, input_dir: &Path) -> String {
    let relative = source.strip_prefix(input_dir).unwrap_or(source);
    relative.to_string_lossy().replace('\\', "/")
}

fn effective_threshold(
    base_threshold: u8,
    overrides: &BTreeMap<String, u8>,
    source_key: &str,
) -> u8 {
    overrides.get(source_key).copied().unwrap_or(base_threshold)
}

fn preprocess_sources(
    sources: &[PathBuf],
    input_dir: &Path,
    glyph_size: u32,
) -> Result<Vec<PreprocessedGlyph>> {
    let mut used_names = HashSet::new();
    let mut out = Vec::with_capacity(sources.len());

    for source in sources {
        let source_rgba = load_source_rgba(source, glyph_size)?;
        let coverage = coverage_map(&source_rgba, glyph_size)?;
        let glyph_name = unique_glyph_name(source, &mut used_names);
        let source_key = source_manifest_key(source, input_dir);
        out.push(PreprocessedGlyph {
            source_path: source.clone(),
            source_key,
            glyph_name,
            size: glyph_size,
            coverage,
        });
    }

    Ok(out)
}

fn load_source_rgba(path: &Path, glyph_size: u32) -> Result<RgbaImage> {
    let ext = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();

    if ext == "svg" {
        render_svg(path, glyph_size)
    } else {
        let img = image::open(path)
            .with_context(|| format!("failed to decode image {}", path.display()))?;
        Ok(img.to_rgba8())
    }
}

fn render_svg(path: &Path, glyph_size: u32) -> Result<RgbaImage> {
    let data = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    let options = usvg::Options::default();
    let tree = usvg::Tree::from_data(&data, &options)
        .with_context(|| format!("failed to parse SVG {}", path.display()))?;

    let size = tree.size().to_int_size();
    let src_w = size.width().max(1);
    let src_h = size.height().max(1);
    let target = (glyph_size.max(16) * 4).max(64);

    let scale = (target as f32 / src_w as f32).min(target as f32 / src_h as f32);
    let out_w = ((src_w as f32 * scale).round() as u32).max(1);
    let out_h = ((src_h as f32 * scale).round() as u32).max(1);

    let mut pixmap = Pixmap::new(out_w, out_h)
        .ok_or_else(|| anyhow::anyhow!("failed to allocate SVG render target"))?;

    let transform = Transform::from_scale(out_w as f32 / src_w as f32, out_h as f32 / src_h as f32);
    resvg::render(&tree, transform, &mut pixmap.as_mut());

    let pixels = pixmap.data().to_vec();
    ImageBuffer::from_raw(out_w, out_h, pixels)
        .ok_or_else(|| anyhow::anyhow!("failed to convert rendered SVG to RGBA image"))
}

fn coverage_map(source: &RgbaImage, glyph_size: u32) -> Result<Vec<u8>> {
    if glyph_size == 0 {
        bail!("glyph_size must be > 0");
    }

    let fitted = fit_to_canvas(source, glyph_size);
    let has_transparency = source.pixels().any(|p| p[3] < 255);
    let background = (!has_transparency).then(|| estimate_background_rgb(source));

    let mut out = vec![0u8; (glyph_size as usize) * (glyph_size as usize)];

    for (idx, pixel) in fitted.pixels().enumerate() {
        let coverage = if has_transparency {
            pixel[3]
        } else {
            opaque_coverage(
                pixel,
                background.expect("background exists for opaque sources"),
            )
        };

        out[idx] = coverage;
    }

    Ok(out)
}

fn estimate_background_rgb(source: &RgbaImage) -> [u8; 3] {
    let (width, height) = source.dimensions();
    let max_x = width.saturating_sub(1);
    let max_y = height.saturating_sub(1);
    let coords = [(0, 0), (max_x, 0), (0, max_y), (max_x, max_y)];

    let mut sum = [0u32; 3];
    for (x, y) in coords {
        let pixel = source.get_pixel(x, y);
        sum[0] += pixel[0] as u32;
        sum[1] += pixel[1] as u32;
        sum[2] += pixel[2] as u32;
    }

    [
        (sum[0] / coords.len() as u32) as u8,
        (sum[1] / coords.len() as u32) as u8,
        (sum[2] / coords.len() as u32) as u8,
    ]
}

fn opaque_coverage(pixel: &Rgba<u8>, background: [u8; 3]) -> u8 {
    if pixel[3] == 0 {
        return 0;
    }

    let dr = pixel[0].abs_diff(background[0]) as u16;
    let dg = pixel[1].abs_diff(background[1]) as u16;
    let db = pixel[2].abs_diff(background[2]) as u16;
    ((dr + dg + db) / 3) as u8
}

fn fit_to_canvas(source: &RgbaImage, glyph_size: u32) -> RgbaImage {
    let (width, height) = source.dimensions();
    let width = width.max(1);
    let height = height.max(1);

    let scale = (glyph_size as f32 / width as f32).min(glyph_size as f32 / height as f32);
    let scaled_w = ((width as f32 * scale).round() as u32).max(1);
    let scaled_h = ((height as f32 * scale).round() as u32).max(1);

    let resized = image::imageops::resize(source, scaled_w, scaled_h, FilterType::Lanczos3);

    let mut canvas = RgbaImage::from_pixel(glyph_size, glyph_size, Rgba([255, 255, 255, 0]));
    let offset_x = ((glyph_size - scaled_w) / 2) as i64;
    let offset_y = ((glyph_size - scaled_h) / 2) as i64;
    image::imageops::overlay(&mut canvas, &resized, offset_x, offset_y);

    canvas
}

fn threshold_bitmap(glyph: &PreprocessedGlyph, threshold: u8) -> GlyphBitmap {
    let pixels = glyph.coverage.iter().map(|v| *v >= threshold).collect();
    GlyphBitmap {
        size: glyph.size,
        pixels,
    }
}

fn write_preview_png(path: &Path, bitmap: &GlyphBitmap) -> Result<()> {
    let mut img = RgbaImage::from_pixel(bitmap.size, bitmap.size, Rgba([255, 255, 255, 0]));

    for y in 0..bitmap.size as usize {
        for x in 0..bitmap.size as usize {
            let idx = y * bitmap.size as usize + x;
            if bitmap.pixels[idx] {
                img.put_pixel(x as u32, y as u32, Rgba([0, 0, 0, 255]));
            }
        }
    }

    img.save(path)
        .with_context(|| format!("failed to save {}", path.display()))?;
    Ok(())
}

fn write_bdf(
    path: &Path,
    font_name: &str,
    glyph_size: u32,
    glyphs: &[(String, u32, GlyphBitmap)],
) -> Result<()> {
    let mut out = String::new();

    out.push_str("STARTFONT 2.1\n");
    out.push_str(&format!("FONT {}\n", bdf_font_name(font_name, glyph_size)));
    out.push_str(&format!("SIZE {} 75 75\n", glyph_size));
    out.push_str(&format!(
        "FONTBOUNDINGBOX {} {} 0 0\n",
        glyph_size, glyph_size
    ));
    out.push_str("STARTPROPERTIES 2\n");
    out.push_str(&format!("FONT_ASCENT {}\n", glyph_size));
    out.push_str("FONT_DESCENT 0\n");
    out.push_str("ENDPROPERTIES\n");
    out.push_str(&format!("CHARS {}\n", glyphs.len()));

    for (name, codepoint, bitmap) in glyphs {
        out.push_str(&format!("STARTCHAR {}\n", name));
        out.push_str(&format!("ENCODING {}\n", codepoint));
        out.push_str("SWIDTH 500 0\n");
        out.push_str(&format!("DWIDTH {} 0\n", glyph_size));
        out.push_str(&format!("BBX {} {} 0 0\n", glyph_size, glyph_size));
        out.push_str("BITMAP\n");
        out.push_str(&bitmap_to_bdf_rows(bitmap));
        out.push_str("ENDCHAR\n");
    }

    out.push_str("ENDFONT\n");
    fs::write(path, out).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

#[derive(Debug)]
struct TtfGlyph {
    codepoint: Option<u32>,
    advance_width: u16,
    left_side_bearing: i16,
    x_min: i16,
    y_min: i16,
    x_max: i16,
    y_max: i16,
    contour_count: u16,
    point_count: u16,
    data: Vec<u8>,
}

fn write_ttf(
    path: &Path,
    font_name: &str,
    glyph_size: u32,
    glyphs: &[(String, u32, GlyphBitmap)],
) -> Result<()> {
    let bytes = build_ttf(font_name, glyph_size, glyphs)?;
    fs::write(path, bytes).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

fn build_ttf(
    font_name: &str,
    glyph_size: u32,
    glyphs: &[(String, u32, GlyphBitmap)],
) -> Result<Vec<u8>> {
    let units_per_em = glyph_size
        .checked_mul(16)
        .context("glyph_size is too large for TTF export")?;
    let units_per_em =
        u16::try_from(units_per_em).context("glyph_size is too large for TTF export")?;

    let mut ttf_glyphs = Vec::with_capacity(glyphs.len() + 2);
    ttf_glyphs.push(bitmap_glyph_to_ttf(
        &notdef_bitmap(glyph_size),
        None,
        units_per_em,
        units_per_em,
    )?);
    ttf_glyphs.push(bitmap_glyph_to_ttf(
        &GlyphBitmap {
            size: glyph_size,
            pixels: vec![false; (glyph_size as usize) * (glyph_size as usize)],
        },
        Some(0x0020),
        units_per_em,
        units_per_em / 2,
    )?);

    for (_, codepoint, bitmap) in glyphs {
        ttf_glyphs.push(bitmap_glyph_to_ttf(
            bitmap,
            Some(*codepoint),
            units_per_em,
            units_per_em,
        )?);
    }

    let num_glyphs =
        u16::try_from(ttf_glyphs.len()).context("too many glyphs for simple TTF export")?;
    let mappings: Vec<(u32, u16)> = ttf_glyphs
        .iter()
        .enumerate()
        .filter_map(|(glyph_id, glyph)| {
            glyph.codepoint.map(|codepoint| {
                (
                    codepoint,
                    u16::try_from(glyph_id).expect("glyph id fits in u16"),
                )
            })
        })
        .collect();

    let mut glyf = Vec::new();
    let mut loca = Vec::with_capacity(ttf_glyphs.len() + 1);
    let mut hmtx = Vec::with_capacity(ttf_glyphs.len() * 4);
    let mut max_points = 0u16;
    let mut max_contours = 0u16;
    let mut x_min = i16::MAX;
    let mut y_min = i16::MAX;
    let mut x_max = i16::MIN;
    let mut y_max = i16::MIN;
    let mut advance_width_max = 0u16;
    let mut min_left_side_bearing = i16::MAX;
    let mut min_right_side_bearing = i16::MAX;
    let mut x_max_extent = i16::MIN;

    for glyph in &ttf_glyphs {
        loca.push(u32::try_from(glyf.len()).context("glyf table exceeded 32-bit offset limit")?);
        glyf.extend_from_slice(&glyph.data);
        while glyf.len() % 4 != 0 {
            glyf.push(0);
        }

        push_u16(&mut hmtx, glyph.advance_width);
        push_i16(&mut hmtx, glyph.left_side_bearing);

        max_points = max_points.max(glyph.point_count);
        max_contours = max_contours.max(glyph.contour_count);
        advance_width_max = advance_width_max.max(glyph.advance_width);
        min_left_side_bearing = min_left_side_bearing.min(glyph.left_side_bearing);
        let right_side_bearing = glyph.advance_width as i32
            - i32::from(glyph.left_side_bearing)
            - i32::from(glyph.x_max);
        min_right_side_bearing = min_right_side_bearing.min(right_side_bearing as i16);
        x_max_extent = x_max_extent.max(glyph.left_side_bearing.saturating_add(glyph.x_max));

        if !glyph.data.is_empty() {
            x_min = x_min.min(glyph.x_min);
            y_min = y_min.min(glyph.y_min);
            x_max = x_max.max(glyph.x_max);
            y_max = y_max.max(glyph.y_max);
        }
    }
    loca.push(u32::try_from(glyf.len()).context("glyf table exceeded 32-bit offset limit")?);

    if x_min == i16::MAX {
        x_min = 0;
        y_min = 0;
        x_max = 0;
        y_max = 0;
    }
    if min_left_side_bearing == i16::MAX {
        min_left_side_bearing = 0;
    }
    if min_right_side_bearing == i16::MAX {
        min_right_side_bearing = 0;
    }
    if x_max_extent == i16::MIN {
        x_max_extent = 0;
    }

    let head = build_head_table(units_per_em, x_min, y_min, x_max, y_max);
    let hhea = build_hhea_table(
        units_per_em,
        advance_width_max,
        min_left_side_bearing,
        min_right_side_bearing,
        x_max_extent,
        num_glyphs,
    );
    let maxp = build_maxp_table(num_glyphs, max_points, max_contours);
    let loca_table = build_loca_table(&loca);
    let cmap = build_cmap_table(&mappings);
    let name = build_name_table(font_name);
    let post = build_post_table();
    let os2 = build_os2_table(units_per_em, &mappings, advance_width_max);

    let mut tables = vec![
        (*b"OS/2", os2),
        (*b"cmap", cmap),
        (*b"glyf", glyf),
        (*b"head", head),
        (*b"hhea", hhea),
        (*b"hmtx", hmtx),
        (*b"loca", loca_table),
        (*b"maxp", maxp),
        (*b"name", name),
        (*b"post", post),
    ];
    tables.sort_by_key(|(tag, _)| *tag);

    build_sfnt(tables)
}

fn notdef_bitmap(size: u32) -> GlyphBitmap {
    let mut pixels = vec![false; (size as usize) * (size as usize)];
    let thickness = (size / 16).max(1);

    for y in 0..size {
        for x in 0..size {
            let border = x < thickness
                || y < thickness
                || x >= size.saturating_sub(thickness)
                || y >= size.saturating_sub(thickness);
            if border {
                let idx = y as usize * size as usize + x as usize;
                pixels[idx] = true;
            }
        }
    }

    GlyphBitmap { size, pixels }
}

fn bitmap_glyph_to_ttf(
    bitmap: &GlyphBitmap,
    codepoint: Option<u32>,
    units_per_em: u16,
    advance_width: u16,
) -> Result<TtfGlyph> {
    if bitmap.size == 0 {
        bail!("glyph bitmap size must be > 0 for TTF export");
    }

    let pixel_units = i16::try_from(u32::from(units_per_em) / bitmap.size)
        .context("invalid pixel scaling for TTF export")?;

    let mut points = Vec::new();
    let mut end_points = Vec::new();
    let mut x_min = i16::MAX;
    let mut y_min = i16::MAX;
    let mut x_max = i16::MIN;
    let mut y_max = i16::MIN;

    for y in 0..bitmap.size as usize {
        for x in 0..bitmap.size as usize {
            if !bitmap.pixels[y * bitmap.size as usize + x] {
                continue;
            }

            let x0 = i16::try_from(x)
                .context("x coordinate overflow in TTF export")?
                .saturating_mul(pixel_units);
            let x1 = x0.saturating_add(pixel_units);
            let top = i16::try_from(units_per_em)
                .context("units_per_em overflow in TTF export")?
                .saturating_sub(
                    i16::try_from(y)
                        .context("y coordinate overflow in TTF export")?
                        .saturating_mul(pixel_units),
                );
            let bottom = top.saturating_sub(pixel_units);

            let contour = [(x0, bottom), (x0, top), (x1, top), (x1, bottom)];
            for (px, py) in contour {
                x_min = x_min.min(px);
                y_min = y_min.min(py);
                x_max = x_max.max(px);
                y_max = y_max.max(py);
                points.push((px, py));
            }
            end_points
                .push(u16::try_from(points.len() - 1).context("too many points for TTF contour")?);
        }
    }

    if points.is_empty() {
        return Ok(TtfGlyph {
            codepoint,
            advance_width,
            left_side_bearing: 0,
            x_min: 0,
            y_min: 0,
            x_max: 0,
            y_max: 0,
            contour_count: 0,
            point_count: 0,
            data: Vec::new(),
        });
    }

    let contour_count =
        u16::try_from(end_points.len()).context("too many contours for TTF export")?;
    let point_count = u16::try_from(points.len()).context("too many points for TTF export")?;

    let mut data = Vec::new();
    push_i16(
        &mut data,
        i16::try_from(contour_count).context("too many contours for TTF export")?,
    );
    push_i16(&mut data, x_min);
    push_i16(&mut data, y_min);
    push_i16(&mut data, x_max);
    push_i16(&mut data, y_max);

    for end_point in &end_points {
        push_u16(&mut data, *end_point);
    }
    push_u16(&mut data, 0);

    data.extend(std::iter::repeat_n(0x01, points.len()));

    let mut prev_x = 0i16;
    for (x, _) in &points {
        push_i16(&mut data, x.saturating_sub(prev_x));
        prev_x = *x;
    }

    let mut prev_y = 0i16;
    for (_, y) in &points {
        push_i16(&mut data, y.saturating_sub(prev_y));
        prev_y = *y;
    }

    Ok(TtfGlyph {
        codepoint,
        advance_width,
        left_side_bearing: 0,
        x_min,
        y_min,
        x_max,
        y_max,
        contour_count,
        point_count,
        data,
    })
}

fn build_head_table(units_per_em: u16, x_min: i16, y_min: i16, x_max: i16, y_max: i16) -> Vec<u8> {
    let mut out = Vec::with_capacity(54);
    push_u32(&mut out, 0x0001_0000);
    push_u32(&mut out, 0x0001_0000);
    push_u32(&mut out, 0);
    push_u32(&mut out, 0x5F0F_3CF5);
    push_u16(&mut out, 0x000B);
    push_u16(&mut out, units_per_em);
    push_i64(&mut out, 0);
    push_i64(&mut out, 0);
    push_i16(&mut out, x_min);
    push_i16(&mut out, y_min);
    push_i16(&mut out, x_max);
    push_i16(&mut out, y_max);
    push_u16(&mut out, 0);
    push_u16(&mut out, 8);
    push_i16(&mut out, 2);
    push_i16(&mut out, 1);
    push_i16(&mut out, 0);
    out
}

fn build_hhea_table(
    units_per_em: u16,
    advance_width_max: u16,
    min_left_side_bearing: i16,
    min_right_side_bearing: i16,
    x_max_extent: i16,
    number_of_h_metrics: u16,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(36);
    push_u32(&mut out, 0x0001_0000);
    push_i16(&mut out, units_per_em as i16);
    push_i16(&mut out, 0);
    push_i16(&mut out, 0);
    push_u16(&mut out, advance_width_max);
    push_i16(&mut out, min_left_side_bearing);
    push_i16(&mut out, min_right_side_bearing);
    push_i16(&mut out, x_max_extent);
    push_i16(&mut out, 1);
    push_i16(&mut out, 0);
    push_i16(&mut out, 0);
    push_i16(&mut out, 0);
    push_i16(&mut out, 0);
    push_i16(&mut out, 0);
    push_i16(&mut out, 0);
    push_i16(&mut out, 0);
    push_u16(&mut out, number_of_h_metrics);
    out
}

fn build_maxp_table(num_glyphs: u16, max_points: u16, max_contours: u16) -> Vec<u8> {
    let mut out = Vec::with_capacity(32);
    push_u32(&mut out, 0x0001_0000);
    push_u16(&mut out, num_glyphs);
    push_u16(&mut out, max_points);
    push_u16(&mut out, max_contours);
    push_u16(&mut out, 0);
    push_u16(&mut out, 0);
    push_u16(&mut out, 2);
    push_u16(&mut out, 0);
    push_u16(&mut out, 0);
    push_u16(&mut out, 0);
    push_u16(&mut out, 0);
    push_u16(&mut out, 0);
    push_u16(&mut out, 0);
    push_u16(&mut out, 0);
    push_u16(&mut out, 0);
    out
}

fn build_loca_table(offsets: &[u32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(offsets.len() * 4);
    for offset in offsets {
        push_u32(&mut out, *offset);
    }
    out
}

fn build_cmap_table(mappings: &[(u32, u16)]) -> Vec<u8> {
    let bmp_mappings: Vec<(u16, u16)> = mappings
        .iter()
        .filter_map(|(codepoint, glyph_id)| {
            u16::try_from(*codepoint).ok().map(|cp| (cp, *glyph_id))
        })
        .collect();

    let mut subtables = Vec::new();
    if !bmp_mappings.is_empty() {
        let format4 = build_cmap_format4(&bmp_mappings);
        subtables.push((0u16, 3u16, format4.clone()));
        subtables.push((3u16, 1u16, format4));
    }

    let format12 = build_cmap_format12(mappings);
    subtables.push((0u16, 4u16, format12.clone()));
    subtables.push((3u16, 10u16, format12));

    let header_len = 4 + subtables.len() * 8;
    let mut offset = header_len;
    let mut records = Vec::with_capacity(subtables.len());
    for (platform_id, encoding_id, subtable) in &subtables {
        records.push((*platform_id, *encoding_id, offset));
        offset += subtable.len();
    }

    let mut out = Vec::with_capacity(offset);
    push_u16(&mut out, 0);
    push_u16(
        &mut out,
        u16::try_from(subtables.len()).expect("cmap subtable count fits in u16"),
    );

    for (platform_id, encoding_id, subtable_offset) in records {
        push_u16(&mut out, platform_id);
        push_u16(&mut out, encoding_id);
        push_u32(
            &mut out,
            u32::try_from(subtable_offset).expect("cmap subtable offset fits in u32"),
        );
    }

    for (_, _, subtable) in subtables {
        out.extend_from_slice(&subtable);
    }

    out
}

fn build_cmap_format4(mappings: &[(u16, u16)]) -> Vec<u8> {
    let mut sorted = mappings.to_vec();
    sorted.sort_unstable_by_key(|(codepoint, _)| *codepoint);

    let mut segments = Vec::new();
    if let Some(&(first_codepoint, first_glyph_id)) = sorted.first() {
        let mut start = first_codepoint;
        let mut end = first_codepoint;
        let mut start_glyph_id = first_glyph_id;
        let mut previous_glyph_id = first_glyph_id;

        for &(codepoint, glyph_id) in sorted.iter().skip(1) {
            if codepoint == end.saturating_add(1) && glyph_id == previous_glyph_id.saturating_add(1)
            {
                end = codepoint;
                previous_glyph_id = glyph_id;
                continue;
            }

            segments.push((start, end, start_glyph_id));
            start = codepoint;
            end = codepoint;
            start_glyph_id = glyph_id;
            previous_glyph_id = glyph_id;
        }

        segments.push((start, end, start_glyph_id));
    }

    let seg_count = u16::try_from(segments.len() + 1).expect("segment count fits in u16");
    let seg_count_x2 = seg_count * 2;
    let max_power = 1u16 << (15 - seg_count.leading_zeros() as u16);
    let search_range = max_power * 2;
    let entry_selector = 15u16 - seg_count.leading_zeros() as u16;
    let range_shift = seg_count_x2 - search_range;
    let length = 16 + usize::from(seg_count) * 8;

    let mut subtable = Vec::with_capacity(length);
    push_u16(&mut subtable, 4);
    push_u16(&mut subtable, length as u16);
    push_u16(&mut subtable, 0);
    push_u16(&mut subtable, seg_count_x2);
    push_u16(&mut subtable, search_range);
    push_u16(&mut subtable, entry_selector);
    push_u16(&mut subtable, range_shift);

    for (_, end, _) in &segments {
        push_u16(&mut subtable, *end);
    }
    push_u16(&mut subtable, 0xFFFF);
    push_u16(&mut subtable, 0);

    for (start, _, _) in &segments {
        push_u16(&mut subtable, *start);
    }
    push_u16(&mut subtable, 0xFFFF);

    for (start, _, start_glyph_id) in &segments {
        push_i16(&mut subtable, start_glyph_id.wrapping_sub(*start) as i16);
    }
    push_i16(&mut subtable, 1);

    for _ in 0..seg_count {
        push_u16(&mut subtable, 0);
    }

    subtable
}

fn build_cmap_format12(mappings: &[(u32, u16)]) -> Vec<u8> {
    let mut sorted = mappings.to_vec();
    sorted.sort_unstable_by_key(|(codepoint, _)| *codepoint);

    let mut groups = Vec::new();
    if let Some(&(first_codepoint, first_glyph_id)) = sorted.first() {
        let mut start = first_codepoint;
        let mut end = first_codepoint;
        let mut start_glyph_id = u32::from(first_glyph_id);
        let mut previous_glyph_id = u32::from(first_glyph_id);

        for &(codepoint, glyph_id) in sorted.iter().skip(1) {
            let glyph_id = u32::from(glyph_id);
            if codepoint == end.saturating_add(1) && glyph_id == previous_glyph_id.saturating_add(1)
            {
                end = codepoint;
                previous_glyph_id = glyph_id;
                continue;
            }

            groups.push((start, end, start_glyph_id));
            start = codepoint;
            end = codepoint;
            start_glyph_id = glyph_id;
            previous_glyph_id = glyph_id;
        }

        groups.push((start, end, start_glyph_id));
    }

    let mut out = Vec::with_capacity(16 + groups.len() * 12);
    push_u16(&mut out, 12);
    push_u16(&mut out, 0);
    push_u32(
        &mut out,
        u32::try_from(16 + groups.len() * 12).expect("format 12 cmap length fits in u32"),
    );
    push_u32(&mut out, 0);
    push_u32(
        &mut out,
        u32::try_from(groups.len()).expect("format 12 cmap group count fits in u32"),
    );

    for (start, end, start_glyph_id) in groups {
        push_u32(&mut out, start);
        push_u32(&mut out, end);
        push_u32(&mut out, start_glyph_id);
    }

    out
}

fn build_name_table(font_name: &str) -> Vec<u8> {
    let family = font_name.trim();
    let family = if family.is_empty() {
        "Petiglyph"
    } else {
        family
    };
    let postscript = postscript_name(family);
    let full_name = format!("{family} Regular");
    let unique = format!("{family};Regular");

    let records = [
        (1u16, family.to_string()),
        (2u16, "Regular".to_string()),
        (3u16, unique),
        (4u16, full_name),
        (5u16, format!("Version {}", env!("CARGO_PKG_VERSION"))),
        (6u16, postscript),
    ];

    let mut string_data = Vec::new();
    let mut name_records = Vec::new();

    for (name_id, value) in records {
        let encoded = utf16be(&value);
        let offset = u16::try_from(string_data.len()).expect("name string offset fits in u16");
        let length = u16::try_from(encoded.len()).expect("name string length fits in u16");
        string_data.extend_from_slice(&encoded);
        name_records.push((name_id, length, offset));
    }

    let count = u16::try_from(name_records.len()).expect("name record count fits in u16");
    let string_offset = 6 + count * 12;

    let mut out = Vec::with_capacity(string_offset as usize + string_data.len());
    push_u16(&mut out, 0);
    push_u16(&mut out, count);
    push_u16(&mut out, string_offset);

    for (name_id, length, offset) in name_records {
        push_u16(&mut out, 3);
        push_u16(&mut out, 1);
        push_u16(&mut out, 0x0409);
        push_u16(&mut out, name_id);
        push_u16(&mut out, length);
        push_u16(&mut out, offset);
    }

    out.extend_from_slice(&string_data);
    out
}

fn build_post_table() -> Vec<u8> {
    let mut out = Vec::with_capacity(32);
    push_u32(&mut out, 0x0003_0000);
    push_u32(&mut out, 0);
    push_i16(&mut out, -75);
    push_i16(&mut out, 50);
    push_u32(&mut out, 1);
    push_u32(&mut out, 0);
    push_u32(&mut out, 0);
    push_u32(&mut out, 0);
    push_u32(&mut out, 0);
    out
}

fn build_os2_table(units_per_em: u16, mappings: &[(u32, u16)], advance_width: u16) -> Vec<u8> {
    let (first_char, last_char) = mappings
        .iter()
        .filter_map(|(codepoint, _)| u16::try_from(*codepoint).ok())
        .fold((u16::MAX, 0u16), |(min_cp, max_cp), cp| {
            (min_cp.min(cp), max_cp.max(cp))
        });
    let (first_char, last_char) = if first_char == u16::MAX {
        (0u16, 0u16)
    } else {
        (first_char, last_char)
    };

    let mut out = Vec::with_capacity(96);
    push_u16(&mut out, 4);
    push_i16(&mut out, advance_width as i16);
    push_u16(&mut out, 400);
    push_u16(&mut out, 5);
    push_u16(&mut out, 0);
    push_i16(&mut out, units_per_em as i16 / 2);
    push_i16(&mut out, units_per_em as i16 / 2);
    push_i16(&mut out, 0);
    push_i16(&mut out, units_per_em as i16 / 4);
    push_i16(&mut out, units_per_em as i16 / 2);
    push_i16(&mut out, units_per_em as i16 / 2);
    push_i16(&mut out, 0);
    push_i16(&mut out, units_per_em as i16 / 4);
    push_i16(&mut out, units_per_em as i16 / 20);
    push_i16(&mut out, units_per_em as i16 / 2);
    push_i16(&mut out, 0);
    out.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    push_u32(&mut out, 0);
    push_u32(&mut out, 1 << 28);
    push_u32(&mut out, 0);
    push_u32(&mut out, 0);
    out.extend_from_slice(b"PTGL");
    push_u16(&mut out, 0x0040);
    push_u16(&mut out, first_char);
    push_u16(&mut out, last_char);
    push_i16(&mut out, units_per_em as i16);
    push_i16(&mut out, 0);
    push_i16(&mut out, 0);
    push_u16(&mut out, units_per_em);
    push_u16(&mut out, 0);
    push_u32(&mut out, 0);
    push_u32(&mut out, 0);
    push_i16(&mut out, units_per_em as i16 / 2);
    push_i16(&mut out, units_per_em as i16);
    push_u16(&mut out, 0);
    push_u16(&mut out, 0x0020);
    push_u16(&mut out, 0);
    out
}

fn build_sfnt(mut tables: Vec<([u8; 4], Vec<u8>)>) -> Result<Vec<u8>> {
    let num_tables = u16::try_from(tables.len()).context("too many SFNT tables")?;
    let entry_selector = if num_tables == 0 {
        0
    } else {
        15u16 - num_tables.leading_zeros() as u16
    };
    let search_range = (1u16 << entry_selector) * 16;
    let range_shift = num_tables * 16 - search_range;

    let directory_len = 12 + tables.len() * 16;
    let mut table_infos = Vec::with_capacity(tables.len());
    let mut offset = directory_len;

    for (tag, data) in &mut tables {
        let checksum = table_checksum(data);
        let length = data.len();
        table_infos.push((*tag, checksum, offset, length));
        offset += align4(length);
    }

    let mut out = Vec::with_capacity(offset);
    push_u32(&mut out, 0x0001_0000);
    push_u16(&mut out, num_tables);
    push_u16(&mut out, search_range);
    push_u16(&mut out, entry_selector);
    push_u16(&mut out, range_shift);

    for (tag, checksum, table_offset, length) in &table_infos {
        out.extend_from_slice(tag);
        push_u32(&mut out, *checksum);
        push_u32(
            &mut out,
            u32::try_from(*table_offset).context("SFNT table offset overflow")?,
        );
        push_u32(
            &mut out,
            u32::try_from(*length).context("SFNT table length overflow")?,
        );
    }

    for (_, data) in &tables {
        out.extend_from_slice(data);
        while out.len() % 4 != 0 {
            out.push(0);
        }
    }

    let head_offset = table_infos
        .iter()
        .find_map(|(tag, _, offset, _)| if tag == b"head" { Some(*offset) } else { None })
        .context("head table missing from SFNT")?;
    let adjustment = 0xB1B0_AFBAu32.wrapping_sub(table_checksum(&out));
    out[head_offset + 8..head_offset + 12].copy_from_slice(&adjustment.to_be_bytes());

    Ok(out)
}

fn table_checksum(data: &[u8]) -> u32 {
    let mut sum = 0u32;
    for chunk in data.chunks(4) {
        let mut padded = [0u8; 4];
        padded[..chunk.len()].copy_from_slice(chunk);
        sum = sum.wrapping_add(u32::from_be_bytes(padded));
    }
    sum
}

fn align4(value: usize) -> usize {
    (value + 3) & !3
}

fn utf16be(value: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(value.len() * 2);
    for unit in value.encode_utf16() {
        out.extend_from_slice(&unit.to_be_bytes());
    }
    out
}

fn postscript_name(font_name: &str) -> String {
    let slug = slugify(font_name).replace('_', "-");
    if slug.is_empty() {
        "Petiglyph-Regular".to_string()
    } else {
        format!("{slug}-Regular")
    }
}

fn push_u16(buf: &mut Vec<u8>, value: u16) {
    buf.extend_from_slice(&value.to_be_bytes());
}

fn push_i16(buf: &mut Vec<u8>, value: i16) {
    buf.extend_from_slice(&value.to_be_bytes());
}

fn push_u32(buf: &mut Vec<u8>, value: u32) {
    buf.extend_from_slice(&value.to_be_bytes());
}

fn push_i64(buf: &mut Vec<u8>, value: i64) {
    buf.extend_from_slice(&value.to_be_bytes());
}

fn bitmap_to_bdf_rows(bitmap: &GlyphBitmap) -> String {
    let width = bitmap.size as usize;
    let height = bitmap.size as usize;
    let bytes_per_row = width.div_ceil(8);

    let mut rows = String::new();
    for y in 0..height {
        let mut row_bytes = vec![0u8; bytes_per_row];

        for x in 0..width {
            let idx = y * width + x;
            if bitmap.pixels[idx] {
                let byte_idx = x / 8;
                let bit_idx = 7 - (x % 8);
                row_bytes[byte_idx] |= 1 << bit_idx;
            }
        }

        for byte in row_bytes {
            rows.push_str(&format!("{byte:02X}"));
        }
        rows.push('\n');
    }

    rows
}

fn unique_glyph_name(path: &Path, used: &mut HashSet<String>) -> String {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .map(slugify)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "glyph".to_string());

    if !used.contains(&stem) {
        used.insert(stem.clone());
        return stem;
    }

    let mut n = 2u32;
    loop {
        let candidate = format!("{stem}_{n}");
        if !used.contains(&candidate) {
            used.insert(candidate.clone());
            return candidate;
        }
        n += 1;
    }
}

fn slugify(input: &str) -> String {
    let mut out = String::new();
    let mut prev_underscore = false;

    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_underscore = false;
            continue;
        }

        if !prev_underscore {
            out.push('_');
            prev_underscore = true;
        }
    }

    out.trim_matches('_').to_string()
}

fn bdf_font_name(font_name: &str, glyph_size: u32) -> String {
    let slug = slugify(font_name);
    let glyph_size = glyph_size.max(1);
    if slug.is_empty() {
        format!(
            "-misc-petiglyph-medium-r-normal--{glyph_size}-{glyph_size}0-75-75-c-{glyph_size}0-iso10646-1"
        )
    } else {
        format!(
            "-misc-{slug}-medium-r-normal--{glyph_size}-{glyph_size}0-75-75-c-{glyph_size}0-iso10646-1"
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppView {
    Home,
    Glyphs,
    Font,
}

struct App {
    manifest_path: PathBuf,
    project_dir: PathBuf,
    config: RuntimeConfig,
    selected: usize,
    glyphs: Vec<InteractiveGlyph>,
    quit: bool,
    status: Option<String>,
    view: AppView,
    last_build: Option<BuildSummary>,
    last_sample: Option<String>,
    installed_font_path: Option<PathBuf>,
}

#[derive(Debug, Clone)]
struct InteractiveGlyph {
    glyph: PreprocessedGlyph,
    saved_threshold: Option<u8>,
    working_threshold: u8,
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
    fn new(manifest_path: PathBuf, config: RuntimeConfig) -> Self {
        let project_dir = manifest_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        Self {
            manifest_path,
            project_dir,
            config,
            selected: 0,
            glyphs: Vec::new(),
            quit: false,
            status: None,
            view: AppView::Home,
            last_build: None,
            last_sample: None,
            installed_font_path: None,
        }
    }

    fn reload_config(&mut self) -> Result<()> {
        self.config = load_runtime_config(&self.manifest_path, None, None, None, None, None)?;
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

fn persist_threshold_override(
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

fn handle_key(app: &mut App, code: KeyCode) -> Result<()> {
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
            app.selected = (app.selected + 1).min(app.glyphs.len().saturating_sub(1));
            app.view = AppView::Glyphs;
        }
        KeyCode::Up | KeyCode::Char('k') => {
            app.selected = app.selected.saturating_sub(1);
            app.view = AppView::Glyphs;
        }
        KeyCode::Right | KeyCode::Char('l') | KeyCode::Char('+') | KeyCode::Char('=') => {
            if let Some(glyph) = selected_glyph(app) {
                let next = glyph.working_threshold.saturating_add(1);
                set_selected_threshold(app, next);
                app.view = AppView::Glyphs;
            }
        }
        KeyCode::Left | KeyCode::Char('h') | KeyCode::Char('-') => {
            if let Some(glyph) = selected_glyph(app) {
                let next = glyph.working_threshold.saturating_sub(1);
                set_selected_threshold(app, next);
                app.view = AppView::Glyphs;
            }
        }
        KeyCode::PageUp => {
            if let Some(glyph) = selected_glyph(app) {
                let next = glyph.working_threshold.saturating_add(10);
                set_selected_threshold(app, next);
                app.view = AppView::Glyphs;
            }
        }
        KeyCode::PageDown => {
            if let Some(glyph) = selected_glyph(app) {
                let next = glyph.working_threshold.saturating_sub(10);
                set_selected_threshold(app, next);
                app.view = AppView::Glyphs;
            }
        }
        KeyCode::Char('r') => {
            remove_selected_threshold_override(app);
            app.view = AppView::Glyphs;
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
        .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).title(Line::from(vec![
            Span::styled(" petiglyph ", Style::default().fg(accent).add_modifier(Modifier::BOLD)),
            Span::styled(format!(" v{} ", CLI_VERSION), Style::default().fg(muted)),
        ])))
        .select(match app.view { AppView::Home => 0, AppView::Glyphs => 1, AppView::Font => 2 })
        .style(Style::default().fg(Color::White))
        .highlight_style(Style::default().fg(Color::Black).bg(accent).add_modifier(Modifier::BOLD))
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
        Span::styled(" q/esc ", Style::default().fg(accent)), Span::raw("quit  "),
        Span::styled(" 1-3 ", Style::default().fg(accent)), Span::raw("views  "),
        Span::styled(" R ", Style::default().fg(accent)), Span::raw("rescan  "),
        Span::styled(" b ", Style::default().fg(accent)), Span::raw("build  "),
        Span::styled(" i ", Style::default().fg(accent)), Span::raw("install  "),
    ];

    if app.view == AppView::Glyphs {
        footer_spans.extend(vec![
            Span::styled(" \u{2191}/\u{2193} ", Style::default().fg(accent)), Span::raw("nav  "),
            Span::styled(" +/- ", Style::default().fg(accent)), Span::raw("thresh  "),
            Span::styled(" r ", Style::default().fg(accent)), Span::raw("reset  "),
        ]);
    }

    if let Some(status) = &app.status {
        footer_spans.push(Span::styled(" | ", Style::default().fg(muted)));
        footer_spans.push(Span::styled(status.clone(), Style::default().fg(Color::LightRed)));
    }

    let footer = Paragraph::new(Line::from(footer_spans))
        .alignment(Alignment::Center)
        .style(Style::default().fg(muted));
    frame.render_widget(footer, root[2]);
}

fn draw_home_view(frame: &mut Frame, app: &App, area: ratatui::layout::Rect, accent: Color, muted: Color) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(muted))
        .title(Span::styled(" Project Overview ", Style::default().fg(accent)));

    let text = vec![
        Line::from(""),
        Line::from(vec![Span::raw("  "), Span::styled("Font Name: ", Style::default().fg(muted)), Span::styled(&app.config.font_name, Style::default().fg(Color::White).add_modifier(Modifier::BOLD))]),
        Line::from(vec![Span::raw("  "), Span::styled("Manifest:  ", Style::default().fg(muted)), Span::raw(app.manifest_path.display().to_string())]),
        Line::from(vec![Span::raw("  "), Span::styled("Icons Dir: ", Style::default().fg(muted)), Span::raw(app.config.input_dir.display().to_string())]),
        Line::from(vec![Span::raw("  "), Span::styled("Build Dir: ", Style::default().fg(muted)), Span::raw(app.config.out_dir.display().to_string())]),
        Line::from(""),
        Line::from(vec![Span::raw("  "), Span::styled("Glyphs:    ", Style::default().fg(muted)), Span::styled(app.glyphs.len().to_string(), Style::default().fg(accent))]),
        Line::from(vec![Span::raw("  "), Span::styled("Size:      ", Style::default().fg(muted)), Span::raw(format!("{}px", app.config.glyph_size))]),
        Line::from(vec![Span::raw("  "), Span::styled("Threshold: ", Style::default().fg(muted)), Span::raw(app.config.base_threshold.to_string())]),
        Line::from(vec![Span::raw("  "), Span::styled("Codepoint: ", Style::default().fg(muted)), Span::raw(format!("U+{:04X}", app.config.codepoint_start))]),
        Line::from(""),
        Line::from(vec![Span::raw("  "), Span::styled("Workflow:", Style::default().fg(accent).add_modifier(Modifier::BOLD))]),
        Line::from(vec![Span::raw("  "), Span::styled("1. ", Style::default().fg(muted)), Span::raw("Add/update images in icons/")]),
        Line::from(vec![Span::raw("  "), Span::styled("2. ", Style::default().fg(muted)), Span::raw("Press R to rescan, tune thresholds in Glyphs view")]),
        Line::from(vec![Span::raw("  "), Span::styled("3. ", Style::default().fg(muted)), Span::raw("Press b to build TTF/BDF and mappings")]),
        Line::from(vec![Span::raw("  "), Span::styled("4. ", Style::default().fg(muted)), Span::raw("Press i to install the font to your system")]),
    ];

    let p = Paragraph::new(text).block(block).wrap(Wrap { trim: false });
    frame.render_widget(p, area);
}

fn draw_glyphs_view(frame: &mut Frame, app: &App, area: ratatui::layout::Rect, accent: Color, muted: Color) {
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
                let marker = if g.saved_threshold.is_some() { " *" } else { "  " };
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
        .highlight_style(Style::default().fg(Color::Black).bg(accent).add_modifier(Modifier::BOLD))
        .highlight_symbol(" \u{2023} ");

    frame.render_stateful_widget(list, chunks[0], &mut list_state);

    let preview_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(muted))
        .title(Span::styled(" Preview ", Style::default().fg(accent)));

    let preview_area = preview_block.inner(chunks[1]);
    
    let mut preview_content = if app.glyphs.is_empty() {
        vec![Line::from(""), Line::from("  Add images and press R to rescan.")]
    } else {
        let active = &app.glyphs[app.selected];
        vec![
            Line::from(""),
            Line::from(vec![
                Span::raw("  File: "), 
                Span::styled(active.glyph.source_path.to_string_lossy().to_string(), Style::default().fg(Color::White))
            ]),
            Line::from(vec![
                Span::raw("  Threshold: "), 
                Span::styled(format!("{:3}", active.working_threshold), Style::default().fg(accent).add_modifier(Modifier::BOLD)),
                Span::styled(if active.saved_threshold.is_some() { " (overridden)" } else { " (default)" }, Style::default().fg(muted)),
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

    let p = Paragraph::new(preview_content).block(preview_block).wrap(Wrap { trim: false });
    frame.render_widget(p, chunks[1]);
}

fn draw_font_view(frame: &mut Frame, app: &App, area: ratatui::layout::Rect, accent: Color, muted: Color) {
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
        None => Span::styled(format!("Not built yet (target: {})", expected_ttf_path(&app.config).display()), missing_style),
    };
    let bdf_status = match build_summary {
        Some(s) => Span::styled(s.bdf_path.display().to_string(), ok_style),
        None => Span::styled(format!("Not built yet (target: {})", expected_bdf_path(&app.config).display()), missing_style),
    };
    let installed_status = match &app.installed_font_path {
        Some(p) => Span::styled(p.display().to_string(), ok_style),
        None => {
            let target = install_dir_for_manifest(&app.manifest_path)
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| "~/.local/share/fonts/petiglyph/<project>".to_string());
            Span::styled(format!("Not installed in this session (target: {})", target), missing_style)
        }
    };

    let text = vec![
        Line::from(""),
        Line::from(vec![Span::raw("  "), Span::styled("Sample string:", Style::default().fg(accent))]),
        Line::from(vec![Span::raw("  "), Span::raw(sample)]),
        Line::from(""),
        Line::from(vec![Span::raw("  "), Span::styled("TTF Output: ", Style::default().fg(muted)), ttf_status]),
        Line::from(vec![Span::raw("  "), Span::styled("BDF Output: ", Style::default().fg(muted)), bdf_status]),
        Line::from(vec![Span::raw("  "), Span::styled("System:     ", Style::default().fg(muted)), installed_status]),
        Line::from(""),
        Line::from(vec![Span::raw("  "), Span::styled("Actions:", Style::default().fg(accent))]),
        Line::from(vec![Span::raw("  "), Span::styled("Press 'b' to build the font files.", Style::default().fg(Color::White))]),
        Line::from(vec![Span::raw("  "), Span::styled("Press 'i' to install the built TTF to your OS.", Style::default().fg(Color::White))]),
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

fn glyph_sample_string(codepoint_start: u32, glyph_count: usize) -> String {
    let mut out = String::new();
    for idx in 0..glyph_count {
        if let Some(ch) = char::from_u32(codepoint_start + idx as u32) {
            out.push(ch);
            out.push(' ');
        }
    }
    out.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn make_temp_dir(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time is valid")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("petiglyph-{name}-{nonce}"));
        fs::create_dir_all(&dir).expect("temp dir is created");
        dir
    }

    fn write_test_png(path: &Path) {
        let mut img = RgbaImage::from_pixel(8, 8, Rgba([255, 255, 255, 0]));
        img.put_pixel(2, 2, Rgba([0, 0, 0, 255]));
        img.put_pixel(5, 5, Rgba([0, 0, 0, 255]));
        img.save(path).expect("test image is written");
    }

    #[test]
    fn parse_codepoint_accepts_common_formats() {
        assert_eq!(parse_codepoint("U+E000").expect("parse U+"), 0xE000);
        assert_eq!(
            parse_codepoint("U+100000").expect("parse supplementary plane"),
            0x10_0000
        );
        assert_eq!(parse_codepoint("0x41").expect("parse hex"), 0x41);
        assert_eq!(
            parse_codepoint("10ffff").expect("parse bare hex"),
            0x10_FFFF
        );
        assert!(parse_codepoint("D800").is_err());
    }

    #[test]
    fn parse_codepoint_rejects_empty_and_out_of_range_values() {
        assert!(parse_codepoint("").is_err());
        assert!(parse_codepoint(" ").is_err());
        assert!(parse_codepoint("110000").is_err());
    }

    #[test]
    fn glyph_sample_string_skips_non_scalar_values() {
        let sample = glyph_sample_string(0xDFFF, 2);
        let expected = char::from_u32(0xE000).expect("valid").to_string();
        assert_eq!(sample, expected);
    }

    #[test]
    fn coverage_map_uses_alpha_for_transparent_sources() {
        let mut image = RgbaImage::from_pixel(2, 2, Rgba([255, 255, 255, 0]));
        image.put_pixel(0, 0, Rgba([255, 255, 255, 255]));

        let coverage = coverage_map(&image, 2).expect("coverage map succeeds");

        assert_eq!(coverage[0], 255);
        assert_eq!(coverage[1], 0);
        assert_eq!(coverage[2], 0);
        assert_eq!(coverage[3], 0);
    }

    #[test]
    fn coverage_map_detects_foreground_on_opaque_background() {
        let mut image = RgbaImage::from_pixel(3, 3, Rgba([0, 0, 0, 255]));
        image.put_pixel(1, 1, Rgba([255, 255, 255, 255]));

        let coverage = coverage_map(&image, 3).expect("coverage map succeeds");

        assert_eq!(coverage[0], 0);
        assert_eq!(coverage[4], 255);
    }

    #[test]
    fn bitmap_to_bdf_rows_packs_pixels_into_hex_rows() {
        let bitmap = GlyphBitmap {
            size: 8,
            pixels: vec![
                true, false, true, false, false, false, false, true, false, true, false, true,
                false, false, false, false, false, false, false, false, true, true, false, false,
                false, false, false, false, false, false, false, false, true, true, true, true,
                true, true, true, true, false, false, false, false, false, false, false, false,
                false, false, false, false, false, false, false, false, true, false, false, false,
                false, false, false, false,
            ],
        };

        assert_eq!(
            bitmap_to_bdf_rows(&bitmap),
            "A1\n50\n0C\n00\nFF\n00\n00\n80\n"
        );
    }

    #[test]
    fn supported_source_extensions_include_avif() {
        assert!(is_supported_source(Path::new("icon.avif")));
        assert!(is_supported_source(Path::new("ICON.AVIF")));
        assert!(!is_supported_source(Path::new("icon.tiff")));
    }

    #[test]
    fn build_outputs_generates_non_empty_repo_icon_font() {
        let out_dir = make_temp_dir("icons-e2e");
        let config = RuntimeConfig {
            input_dir: PathBuf::from("icons"),
            out_dir: out_dir.clone(),
            font_name: "Petiglyph".to_string(),
            glyph_size: 64,
            base_threshold: 64,
            threshold_overrides: BTreeMap::new(),
            codepoint_start: 0x10_0000,
        };

        let summary = build_outputs(&config).expect("build succeeds");
        let mapping_json = fs::read_to_string(&summary.mapping_path).expect("glyph map is written");
        let mapping: Vec<MappingEntry> =
            serde_json::from_str(&mapping_json).expect("glyph map parses");
        let bdf = fs::read_to_string(&summary.bdf_path).expect("bdf is written");
        let ttf = fs::read(&summary.ttf_path).expect("ttf is written");
        let sample = fs::read_to_string(&summary.sample_path).expect("sample is written");
        let sources = collect_source_files(Path::new("icons")).expect("icons are readable");
        let face = ttf_parser::Face::parse(&ttf, 0).expect("ttf parses");

        assert_eq!(summary.glyph_count, sources.len());
        assert_eq!(mapping.len(), sources.len());
        assert!(bdf.contains(&format!("CHARS {}", sources.len())));
        assert_eq!(face.number_of_glyphs(), sources.len() as u16 + 2);
        assert_eq!(
            sample.trim_end(),
            glyph_sample_string(config.codepoint_start, sources.len())
        );
        assert!(face.glyph_index(' ').is_some(), "space glyph should exist");

        for entry in &mapping {
            assert!(bdf.contains(&format!("STARTCHAR {}", entry.glyph_name)));
            let codepoint = parse_codepoint(&entry.codepoint).expect("codepoint parses");
            assert!(bdf.contains(&format!("ENCODING {}", codepoint)));
            assert!(
                face.glyph_index(char::from_u32(codepoint).expect("bmp codepoint"))
                    .is_some(),
                "ttf should map {}",
                entry.glyph_name
            );

            let preview_path = summary
                .previews_dir
                .join(format!("{}.png", entry.glyph_name));
            let preview = image::open(&preview_path)
                .expect("preview opens")
                .to_rgba8();
            assert!(
                preview.pixels().any(|pixel| pixel[3] > 0),
                "preview should contain visible pixels: {}",
                preview_path.display()
            );
        }

        fs::remove_dir_all(out_dir).expect("temp output dir is removed");
    }

    #[test]
    fn build_outputs_supports_upper_unicode_edge() {
        let project_dir = make_temp_dir("unicode-edge");
        let input_dir = project_dir.join("icons");
        let out_dir = project_dir.join("build");
        fs::create_dir_all(&input_dir).expect("icons dir is created");
        fs::create_dir_all(&out_dir).expect("build dir is created");
        write_test_png(&input_dir.join("a.png"));
        write_test_png(&input_dir.join("b.png"));

        let config = RuntimeConfig {
            input_dir,
            out_dir: out_dir.clone(),
            font_name: "Edge".to_string(),
            glyph_size: 32,
            base_threshold: 64,
            threshold_overrides: BTreeMap::new(),
            codepoint_start: 0x10_FFFE,
        };

        let summary = build_outputs(&config).expect("build succeeds");
        let mapping_json = fs::read_to_string(&summary.mapping_path).expect("glyph map is written");
        let mapping: Vec<MappingEntry> =
            serde_json::from_str(&mapping_json).expect("glyph map parses");

        assert_eq!(mapping.len(), 2);
        assert_eq!(mapping[0].codepoint, "U+10FFFE");
        assert_eq!(mapping[1].codepoint, "U+10FFFF");
        assert_eq!(
            fs::read_to_string(summary.sample_path)
                .expect("sample is written")
                .trim()
                .to_string(),
            glyph_sample_string(0x10_FFFE, 2)
        );

        fs::remove_dir_all(project_dir).expect("temp project dir is removed");
    }

    #[test]
    fn persist_threshold_override_roundtrip() {
        let project_dir = make_temp_dir("override-roundtrip");
        let manifest_path = project_dir.join("petiglyph.toml");
        write_manifest(&manifest_path, &Manifest::default()).expect("manifest is written");

        persist_threshold_override(&manifest_path, "icon.png", Some(77))
            .expect("override should persist");
        let manifest = read_manifest(&manifest_path).expect("manifest reads");
        assert_eq!(manifest.threshold_overrides.get("icon.png"), Some(&77));

        persist_threshold_override(&manifest_path, "icon.png", None)
            .expect("override should clear");
        let manifest = read_manifest(&manifest_path).expect("manifest reads");
        assert!(!manifest.threshold_overrides.contains_key("icon.png"));

        fs::remove_dir_all(project_dir).expect("temp project dir is removed");
    }

    #[test]
    fn handle_key_updates_and_clears_selected_threshold_override() {
        let project_dir = make_temp_dir("handle-key-threshold");
        let manifest_path = project_dir.join("petiglyph.toml");
        write_manifest(&manifest_path, &Manifest::default()).expect("manifest is written");

        let config = RuntimeConfig {
            input_dir: project_dir.join("icons"),
            out_dir: project_dir.join("build"),
            font_name: "Petiglyph".to_string(),
            glyph_size: 8,
            base_threshold: 64,
            threshold_overrides: BTreeMap::new(),
            codepoint_start: 0x10_0000,
        };

        let mut app = App::new(manifest_path.clone(), config);
        app.glyphs.push(InteractiveGlyph {
            glyph: PreprocessedGlyph {
                source_path: project_dir.join("icons/icon.png"),
                source_key: "icon.png".to_string(),
                glyph_name: "icon".to_string(),
                size: 8,
                coverage: vec![0; 64],
            },
            saved_threshold: None,
            working_threshold: 64,
        });
        app.selected = 0;

        handle_key(&mut app, KeyCode::Char('+')).expect("key handling should succeed");
        assert_eq!(app.glyphs[0].working_threshold, 65);
        assert_eq!(app.glyphs[0].saved_threshold, Some(65));
        assert_eq!(app.view, AppView::Glyphs);
        let manifest = read_manifest(&manifest_path).expect("manifest reads");
        assert_eq!(manifest.threshold_overrides.get("icon.png"), Some(&65));

        handle_key(&mut app, KeyCode::Char('r')).expect("key handling should succeed");
        assert_eq!(app.glyphs[0].working_threshold, 64);
        assert_eq!(app.glyphs[0].saved_threshold, None);
        assert_eq!(app.view, AppView::Glyphs);
        let manifest = read_manifest(&manifest_path).expect("manifest reads");
        assert!(!manifest.threshold_overrides.contains_key("icon.png"));

        fs::remove_dir_all(project_dir).expect("temp project dir is removed");
    }
}
