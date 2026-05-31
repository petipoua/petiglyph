use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use serde::Serialize;
use std::collections::BTreeSet;
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::artifact_warning::incompatible_artifact_warning;
use crate::build::is_supported_source;
use crate::build::{BuildOptions, BuildSummary, build_outputs_with_options};
use crate::doctor::{DoctorReport, doctor};
use crate::install::{
    DEFAULT_INSTALL_NAME_MODE, FontPlatform, UninstallOutcome, diagnose_sample_coverage,
    effective_font_name, install_built_font, supplementary_pua_usage_summary,
    uninstall_project_font, uninstall_tool_state,
};
use crate::project::{
    AnimationDef, AnimationType, BleedLevel, CompositionDef, Manifest, RuntimeConfig,
    create_project, delete_project_for_manifest, discover_project_manifests, format_codepoint,
    load_runtime_config, manifest_path_from_option, read_manifest, slugify, write_manifest,
};
use crate::tui::{tui, tui_workspace};

const CLI_VERSION: &str = env!("CARGO_PKG_VERSION");
const FFMPEG_SETUP_PROMPT_STATE_FILE_NAME: &str = ".ffmpeg-setup-prompt-v1.json";
const FFMPEG_SETUP_PROMPT_STATE_VERSION: u32 = 1;

#[derive(Debug, Parser)]
#[command(
    name = "petiglyph",
    version,
    about = "TUI-first CLI for building self-contained monochrome glyph font projects."
)]
struct Cli {
    /// Enable verbose image-to-glyph debug artifacts and logs (writes to project `debug/`).
    #[arg(long, global = true)]
    debug: bool,
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
    /// List projects in the workspace and globally installed petiglyph fonts.
    List {
        /// Emit machine-readable JSON to stdout.
        #[arg(long)]
        json: bool,
    },
    /// Delete a petiglyph project directory.
    Delete {
        /// Path to the manifest file. When omitted, auto-detect from the current directory or one level below.
        #[arg(short, long)]
        manifest: Option<PathBuf>,
        /// Emit machine-readable JSON to stdout.
        #[arg(long)]
        json: bool,
    },
    /// Shortcut for `glyph set-threshold`.
    SetThreshold {
        /// The filename of the source image in the icons folder (e.g., 'alpha.png').
        image_name: String,
        /// The threshold value to set (0-255).
        threshold: u8,
        /// Path to the manifest file. When omitted, auto-detect from the current directory or one level below.
        #[arg(short, long)]
        manifest: Option<PathBuf>,
        /// Emit machine-readable JSON to stdout.
        #[arg(long)]
        json: bool,
    },
    /// Shortcut for `glyph clear-threshold`.
    ClearThreshold {
        /// The filename of the source image in the icons folder (e.g., 'alpha.png').
        image_name: String,
        /// Path to the manifest file. When omitted, auto-detect from the current directory or one level below.
        #[arg(short, long)]
        manifest: Option<PathBuf>,
        /// Emit machine-readable JSON to stdout.
        #[arg(long)]
        json: bool,
    },
    /// Glyph operations: import/create and glyph-level overrides.
    Glyph {
        #[command(subcommand)]
        command: GlyphCommand,
    },
    /// Grid composition creation workflow commands.
    Grid {
        #[command(subcommand)]
        command: GridCommand,
    },
    /// Composition mutation commands.
    Composition {
        #[command(subcommand)]
        command: CompositionCommand,
    },
    /// Animation creation and mutation commands.
    Animation {
        #[command(subcommand)]
        command: AnimationCommand,
    },
    /// Launch the petiglyph TUI for a project.
    Tui {
        /// Path to the manifest file. When omitted, auto-detect from the current directory or one level below.
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
        /// Path to the manifest file. When omitted, auto-detect from the current directory or one level below.
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
        /// Discard existing glyph lock mappings and remap all codepoints for this build.
        #[arg(long)]
        force_remap: bool,
    },
    /// Build, install, refresh font cache, and print the sample private-use string.
    Sample {
        /// Path to the manifest file. When omitted, auto-detect from the current directory or one level below.
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
        /// Discard existing glyph lock mappings and remap all codepoints for this build.
        #[arg(long)]
        force_remap: bool,
    },
    /// Build the font and install it into the user font directory using a project-prefixed name.
    InstallFont {
        /// Path to the manifest file. When omitted, auto-detect from the current directory or one level below.
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
        /// Discard existing glyph lock mappings and remap all codepoints for this build.
        #[arg(long)]
        force_remap: bool,
    },
    /// Uninstall this project's managed installed font variants.
    UninstallFont {
        /// Path to the manifest file. When omitted, auto-detect from the current directory or one level below.
        #[arg(short, long)]
        manifest: Option<PathBuf>,
        /// Emit machine-readable JSON to stdout.
        #[arg(long)]
        json: bool,
    },
    /// Remove all petiglyph-managed user state (fonts, registry, and metadata).
    #[command(name = "nuke-everything")]
    NukeEverything {
        /// Emit machine-readable JSON to stdout.
        #[arg(long)]
        json: bool,
    },
    /// Stub for preventing accidental full uninstall.
    #[command(name = "uninstall", hide = true)]
    Uninstall,
    /// Inspect and repair glyph lock/Unicode registry health.
    Doctor {
        /// Path to the manifest file. When omitted, global checks run and project checks auto-detect when possible.
        #[arg(short, long)]
        manifest: Option<PathBuf>,
        /// Apply safe repairs for stale locks, orphan metadata, and missing project registry assignment.
        #[arg(long)]
        repair: bool,
        /// Emit machine-readable JSON to stdout.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
enum GlyphCommand {
    /// Import one or more source images and apply glyph-level defaults.
    Create {
        #[arg(short, long)]
        manifest: Option<PathBuf>,
        #[arg(long = "input", required = true)]
        input: Vec<PathBuf>,
        #[arg(long, default_value_t = 64)]
        threshold: u8,
        #[arg(long, value_enum, default_value_t = InvertValue::Off)]
        invert: InvertValue,
        #[arg(long, default_value_t = false)]
        grayscale_enabled: bool,
        #[arg(long, default_value_t = 0)]
        grayscale_brightness: i16,
        #[arg(long, default_value_t = 0)]
        grayscale_contrast: i16,
        #[arg(long, default_value_t = 100)]
        grayscale_gamma_percent: u16,
        #[arg(long)]
        json: bool,
    },
    /// Set glyph threshold override.
    SetThreshold {
        image_name: String,
        threshold: u8,
        #[arg(short, long)]
        manifest: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// Clear glyph threshold override.
    ClearThreshold {
        image_name: String,
        #[arg(short, long)]
        manifest: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// Set glyph invert override.
    SetInvert {
        image_name: String,
        #[arg(long, value_enum)]
        invert: InvertValue,
        #[arg(short, long)]
        manifest: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
enum GridCommand {
    /// Create/replace a grid composition from one imported image.
    Create {
        #[arg(short, long)]
        manifest: Option<PathBuf>,
        #[arg(long = "input", required = true)]
        input: Vec<PathBuf>,
        #[arg(long)]
        rows: usize,
        #[arg(long)]
        cols: usize,
        #[arg(long, value_enum, default_value_t = BleedValue::Weak)]
        horizontal_bleed: BleedValue,
        #[arg(long, value_enum, default_value_t = BleedValue::Off)]
        vertical_bleed: BleedValue,
        #[arg(long, default_value_t = 64)]
        threshold: u8,
        #[arg(long, value_enum, default_value_t = InvertValue::Off)]
        invert: InvertValue,
        #[arg(long, default_value_t = false)]
        grayscale_enabled: bool,
        #[arg(long, default_value_t = 0)]
        grayscale_brightness: i16,
        #[arg(long, default_value_t = 0)]
        grayscale_contrast: i16,
        #[arg(long, default_value_t = 100)]
        grayscale_gamma_percent: u16,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
enum CompositionCommand {
    Set {
        source_key: String,
        #[arg(short, long)]
        manifest: Option<PathBuf>,
        #[arg(long)]
        rows: usize,
        #[arg(long)]
        cols: usize,
        #[arg(long, value_enum, default_value_t = BleedValue::Weak)]
        horizontal_bleed: BleedValue,
        #[arg(long, value_enum, default_value_t = BleedValue::Off)]
        vertical_bleed: BleedValue,
        #[arg(long)]
        json: bool,
    },
    Clear {
        source_key: String,
        #[arg(short, long)]
        manifest: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
enum AnimationCommand {
    /// Import media frames and create a standard animation.
    CreateStandard {
        #[arg(short, long)]
        manifest: Option<PathBuf>,
        #[arg(long = "input", required = true)]
        input: Vec<PathBuf>,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        fps: u8,
        #[arg(long, default_value_t = 64)]
        threshold: u8,
        #[arg(long, value_enum, default_value_t = InvertValue::Off)]
        invert: InvertValue,
        #[arg(long, default_value_t = true)]
        grayscale_enabled: bool,
        #[arg(long, default_value_t = 0)]
        grayscale_brightness: i16,
        #[arg(long, default_value_t = 0)]
        grayscale_contrast: i16,
        #[arg(long, default_value_t = 100)]
        grayscale_gamma_percent: u16,
        #[arg(long)]
        json: bool,
    },
    /// Import media frames and create a grid animation.
    CreateGrid {
        #[arg(short, long)]
        manifest: Option<PathBuf>,
        #[arg(long = "input", required = true)]
        input: Vec<PathBuf>,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        fps: u8,
        #[arg(long)]
        rows: usize,
        #[arg(long)]
        cols: usize,
        #[arg(long, value_enum, default_value_t = BleedValue::Weak)]
        horizontal_bleed: BleedValue,
        #[arg(long, value_enum, default_value_t = BleedValue::Off)]
        vertical_bleed: BleedValue,
        #[arg(long, default_value_t = 64)]
        threshold: u8,
        #[arg(long, value_enum, default_value_t = InvertValue::Off)]
        invert: InvertValue,
        #[arg(long, default_value_t = true)]
        grayscale_enabled: bool,
        #[arg(long, default_value_t = 0)]
        grayscale_brightness: i16,
        #[arg(long, default_value_t = 0)]
        grayscale_contrast: i16,
        #[arg(long, default_value_t = 100)]
        grayscale_gamma_percent: u16,
        #[arg(long)]
        json: bool,
    },
    /// Update an animation's frames-per-second value.
    SetFps {
        name: String,
        #[arg(long)]
        fps: u8,
        #[arg(short, long)]
        manifest: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// Delete an animation definition from the project manifest.
    Delete {
        name: String,
        #[arg(short, long)]
        manifest: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum InvertValue {
    On,
    Off,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum BleedValue {
    Off,
    Weak,
    Strong,
}

impl From<BleedValue> for BleedLevel {
    fn from(value: BleedValue) -> Self {
        match value {
            BleedValue::Off => BleedLevel::Off,
            BleedValue::Weak => BleedLevel::Weak,
            BleedValue::Strong => BleedLevel::Strong,
        }
    }
}

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

#[derive(Debug, Serialize)]
struct ListCommandData {
    workspace_dir: String,
    projects: Vec<ListProjectData>,
    installed_fonts: Vec<ListInstalledFontData>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pua_usage: Option<crate::install::PuaUsageSummary>,
}

#[derive(Debug, Serialize)]
struct ListProjectData {
    manifest_path: String,
    font_name: String,
}

#[derive(Debug, Serialize)]
struct ListInstalledFontData {
    file_name: String,
    path: String,
}

#[derive(Debug, Serialize)]
struct DeleteCommandData {
    manifest: String,
    deleted_dir: String,
}

#[derive(Debug, Serialize)]
struct SetThresholdCommandData {
    manifest: String,
    image_name: String,
    threshold: u8,
}

#[derive(Debug, Serialize)]
struct ClearThresholdCommandData {
    manifest: String,
    image_name: String,
    was_present: bool,
}

#[derive(Debug, Serialize)]
struct ImportedSourcesCommandData {
    manifest: String,
    imported_sources: Vec<String>,
}

#[derive(Debug, Serialize)]
struct SetInvertCommandData {
    manifest: String,
    image_name: String,
    invert: bool,
}

#[derive(Debug, Serialize)]
struct CompositionCommandData {
    manifest: String,
    source_key: String,
    rows: Option<usize>,
    cols: Option<usize>,
}

#[derive(Debug, Serialize)]
struct AnimationMutationCommandData {
    manifest: String,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    fps: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    frame_count: Option<usize>,
}

#[derive(Debug, Serialize)]
pub(crate) struct BuildCommandData {
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
    installed_ttf: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    coverage: Option<crate::install::SampleCoverageDiagnosis>,
}

#[derive(Debug, Serialize)]
struct InstallFontCommandData {
    build: BuildCommandData,
    platform: FontPlatform,
    install_dir: String,
    installed_ttf: String,
    replaced_previous_ttf_count: usize,
}

#[derive(Debug, Serialize)]
struct UninstallFontCommandData {
    manifest: String,
    platform: FontPlatform,
    install_dir: String,
    outcome: UninstallOutcome,
    removed_ttf_count: usize,
}

#[derive(Debug, Serialize)]
struct UninstallToolCommandData {
    platform: FontPlatform,
    install_dir: String,
    outcome: UninstallOutcome,
    removed_ttf_count: usize,
    removed_metadata_count: usize,
    removed_state_file_count: usize,
    binary_path: Option<String>,
}

enum CliRunError {
    Plain(anyhow::Error),
    Json {
        command: &'static str,
        error: anyhow::Error,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DefaultTuiTarget {
    pub(crate) workspace_root: PathBuf,
    pub(crate) initial_project: Option<PathBuf>,
}

pub(crate) fn run() {
    let cli = Cli::parse();
    crate::glyph_debug::set_debug_enabled(cli.debug);
    maybe_offer_first_run_ffmpeg_setup(&cli);
    let exit_code = match run_cli(cli) {
        Ok(()) => 0,
        Err(CliRunError::Plain(error)) => {
            let rendered = format!("{error:#}");
            if let Some(warning) = incompatible_artifact_warning(&rendered, None) {
                if io::stderr().is_terminal() {
                    eprintln!("\x1b[31m{warning}\x1b[0m");
                } else {
                    eprintln!("{warning}");
                }
            }
            eprintln!("{rendered}");
            1
        }
        Err(CliRunError::Json { command, error }) => {
            emit_json_error(command, &error);
            1
        }
    };
    std::process::exit(exit_code);
}

#[derive(Debug)]
struct FfmpegInstallHint {
    detected_system: String,
    suggested_command: String,
    invocation: Option<CommandInvocation>,
}

#[derive(Debug)]
struct CommandInvocation {
    program: String,
    args: Vec<String>,
}

fn maybe_offer_first_run_ffmpeg_setup(cli: &Cli) {
    if !should_offer_first_run_ffmpeg_setup(cli) || ffmpeg_available_on_path() {
        return;
    }

    let Some(state_path) = ffmpeg_setup_prompt_state_path() else {
        return;
    };
    if state_path.exists() {
        return;
    }

    let hint = ffmpeg_install_hint_for_current_system();
    println!("FFmpeg was not found.");
    println!();
    println!("petiglyph requires FFmpeg for video/animated media processing.");
    println!();
    println!("Detected system: {}", hint.detected_system);
    println!("Suggested command:");
    println!();
    println!("  {}", hint.suggested_command);
    println!();
    print!("Run this now? [y/N] ");
    let _ = io::stdout().flush();

    let mut answer = String::new();
    let outcome = match io::stdin().read_line(&mut answer) {
        Ok(_) => {
            if answer.trim().eq_ignore_ascii_case("y") {
                match run_ffmpeg_install_command(&hint) {
                    Ok(()) => {
                        if ffmpeg_available_on_path() {
                            println!("FFmpeg install completed and is now available on PATH.");
                            "accepted_success"
                        } else {
                            println!(
                                "FFmpeg install command completed, but `ffmpeg` is still not on PATH."
                            );
                            println!("You may need to restart your terminal session.");
                            "accepted_not_on_path"
                        }
                    }
                    Err(error) => {
                        eprintln!("FFmpeg install command failed: {error}");
                        "accepted_failed"
                    }
                }
            } else {
                println!("Skipped FFmpeg auto-install. You can run the command manually later.");
                "declined"
            }
        }
        Err(error) => {
            eprintln!("Could not read prompt input: {error}");
            "prompt_input_failed"
        }
    };

    if let Err(error) = record_ffmpeg_setup_prompt_state(&state_path, outcome, &hint) {
        eprintln!("warning: failed to persist FFmpeg setup prompt state: {error}");
    }
}

fn should_offer_first_run_ffmpeg_setup(cli: &Cli) -> bool {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return false;
    }

    if command_emits_json(cli) {
        return false;
    }

    matches!(
        cli.command,
        None | Some(CliCommand::Create { .. })
            | Some(CliCommand::Tui { .. })
            | Some(CliCommand::Build { .. })
            | Some(CliCommand::Sample { .. })
            | Some(CliCommand::InstallFont { .. })
    )
}

fn command_emits_json(cli: &Cli) -> bool {
    matches!(
        cli.command,
        Some(CliCommand::List { json: true })
            | Some(CliCommand::Delete { json: true, .. })
            | Some(CliCommand::SetThreshold { json: true, .. })
            | Some(CliCommand::ClearThreshold { json: true, .. })
            | Some(CliCommand::Build { json: true, .. })
            | Some(CliCommand::Sample { json: true, .. })
            | Some(CliCommand::InstallFont { json: true, .. })
            | Some(CliCommand::UninstallFont { json: true, .. })
            | Some(CliCommand::NukeEverything { json: true })
            | Some(CliCommand::Doctor { json: true, .. })
            | Some(CliCommand::Glyph {
                command: GlyphCommand::Create { json: true, .. }
                    | GlyphCommand::SetThreshold { json: true, .. }
                    | GlyphCommand::ClearThreshold { json: true, .. }
                    | GlyphCommand::SetInvert { json: true, .. },
            })
            | Some(CliCommand::Grid {
                command: GridCommand::Create { json: true, .. },
            })
            | Some(CliCommand::Composition {
                command: CompositionCommand::Set { json: true, .. }
                    | CompositionCommand::Clear { json: true, .. },
            })
            | Some(CliCommand::Animation {
                command: AnimationCommand::CreateStandard { json: true, .. }
                    | AnimationCommand::CreateGrid { json: true, .. }
                    | AnimationCommand::SetFps { json: true, .. }
                    | AnimationCommand::Delete { json: true, .. },
            })
    )
}

fn ffmpeg_setup_prompt_state_path() -> Option<PathBuf> {
    crate::install::managed_install_dir()
        .ok()
        .map(|dir| dir.join(FFMPEG_SETUP_PROMPT_STATE_FILE_NAME))
}

fn ffmpeg_available_on_path() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn run_ffmpeg_install_command(hint: &FfmpegInstallHint) -> Result<()> {
    let Some(invocation) = &hint.invocation else {
        anyhow::bail!(
            "no executable install command is available for {}; run the suggested command manually",
            hint.detected_system
        );
    };

    let status = Command::new(&invocation.program)
        .args(invocation.args.iter())
        .status()
        .with_context(|| format!("failed to launch {}", invocation.program))?;

    if status.success() {
        Ok(())
    } else {
        let code = status
            .code()
            .map_or_else(|| "signal".to_string(), |c| c.to_string());
        anyhow::bail!("installer exited with status {code}");
    }
}

fn now_unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn record_ffmpeg_setup_prompt_state(
    state_path: &Path,
    outcome: &str,
    hint: &FfmpegInstallHint,
) -> Result<()> {
    if let Some(parent) = state_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let payload = serde_json::json!({
        "version": FFMPEG_SETUP_PROMPT_STATE_VERSION,
        "recorded_unix_ms": now_unix_ms(),
        "recorded_by_cli_version": CLI_VERSION,
        "outcome": outcome,
        "suggested_command": hint.suggested_command,
        "detected_system": hint.detected_system,
    });
    let raw = serde_json::to_string_pretty(&payload)
        .context("failed to serialize ffmpeg prompt state")?;
    fs::write(state_path, raw).with_context(|| format!("failed to write {}", state_path.display()))
}

fn ffmpeg_install_hint_for_current_system() -> FfmpegInstallHint {
    match std::env::consts::OS {
        "linux" => linux_ffmpeg_install_hint(),
        "macos" => FfmpegInstallHint {
            detected_system: "macOS".to_string(),
            suggested_command: "brew install ffmpeg".to_string(),
            invocation: Some(CommandInvocation {
                program: "brew".to_string(),
                args: vec!["install".to_string(), "ffmpeg".to_string()],
            }),
        },
        "windows" => FfmpegInstallHint {
            detected_system: "Windows".to_string(),
            suggested_command: "winget install --id Gyan.FFmpeg --exact --accept-package-agreements --accept-source-agreements".to_string(),
            invocation: Some(CommandInvocation {
                program: "winget".to_string(),
                args: vec![
                    "install".to_string(),
                    "--id".to_string(),
                    "Gyan.FFmpeg".to_string(),
                    "--exact".to_string(),
                    "--accept-package-agreements".to_string(),
                    "--accept-source-agreements".to_string(),
                ],
            }),
        },
        other => FfmpegInstallHint {
            detected_system: other.to_string(),
            suggested_command: "install ffmpeg with your system package manager".to_string(),
            invocation: None,
        },
    }
}

fn linux_ffmpeg_install_hint() -> FfmpegInstallHint {
    let (system_name, id, id_like) = linux_os_release_identity();
    if linux_family_matches(&id, &id_like, &["arch", "manjaro", "endeavouros"]) {
        return FfmpegInstallHint {
            detected_system: system_name,
            suggested_command: "sudo pacman -S --needed ffmpeg".to_string(),
            invocation: Some(CommandInvocation {
                program: "sudo".to_string(),
                args: vec![
                    "pacman".to_string(),
                    "-S".to_string(),
                    "--needed".to_string(),
                    "ffmpeg".to_string(),
                ],
            }),
        };
    }

    if linux_family_matches(&id, &id_like, &["debian", "ubuntu"]) {
        return FfmpegInstallHint {
            detected_system: system_name,
            suggested_command: "sudo apt install -y ffmpeg".to_string(),
            invocation: Some(CommandInvocation {
                program: "sudo".to_string(),
                args: vec![
                    "apt".to_string(),
                    "install".to_string(),
                    "-y".to_string(),
                    "ffmpeg".to_string(),
                ],
            }),
        };
    }

    if linux_family_matches(&id, &id_like, &["fedora", "rhel", "centos"]) {
        return FfmpegInstallHint {
            detected_system: system_name,
            suggested_command: "sudo dnf install -y ffmpeg".to_string(),
            invocation: Some(CommandInvocation {
                program: "sudo".to_string(),
                args: vec![
                    "dnf".to_string(),
                    "install".to_string(),
                    "-y".to_string(),
                    "ffmpeg".to_string(),
                ],
            }),
        };
    }

    if linux_family_matches(&id, &id_like, &["opensuse", "sles", "suse"]) {
        return FfmpegInstallHint {
            detected_system: system_name,
            suggested_command: "sudo zypper install -y ffmpeg".to_string(),
            invocation: Some(CommandInvocation {
                program: "sudo".to_string(),
                args: vec![
                    "zypper".to_string(),
                    "install".to_string(),
                    "-y".to_string(),
                    "ffmpeg".to_string(),
                ],
            }),
        };
    }

    if linux_family_matches(&id, &id_like, &["alpine"]) {
        return FfmpegInstallHint {
            detected_system: system_name,
            suggested_command: "sudo apk add ffmpeg".to_string(),
            invocation: Some(CommandInvocation {
                program: "sudo".to_string(),
                args: vec!["apk".to_string(), "add".to_string(), "ffmpeg".to_string()],
            }),
        };
    }

    FfmpegInstallHint {
        detected_system: if system_name.is_empty() {
            "Linux".to_string()
        } else {
            system_name
        },
        suggested_command: "install ffmpeg with your distribution package manager".to_string(),
        invocation: None,
    }
}

fn linux_family_matches(id: &str, id_like: &[String], targets: &[&str]) -> bool {
    targets
        .iter()
        .copied()
        .any(|target| id == target || id_like.iter().any(|like| like == target))
}

fn linux_os_release_identity() -> (String, String, Vec<String>) {
    let mut fields = fs::read_to_string("/etc/os-release")
        .ok()
        .as_deref()
        .map(parse_os_release_fields)
        .unwrap_or_default();

    if fields.is_empty() {
        fields = fs::read_to_string("/usr/lib/os-release")
            .ok()
            .as_deref()
            .map(parse_os_release_fields)
            .unwrap_or_default();
    }

    let pretty_name = fields.get("PRETTY_NAME").cloned().unwrap_or_default();
    let id = fields
        .get("ID")
        .map(|v| v.to_ascii_lowercase())
        .unwrap_or_else(|| "linux".to_string());
    let id_like = fields
        .get("ID_LIKE")
        .map(|v| {
            v.split_whitespace()
                .map(|part| part.to_ascii_lowercase())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let system_name = if pretty_name.is_empty() {
        format!("Linux ({id})")
    } else {
        pretty_name
    };
    (system_name, id, id_like)
}

fn parse_os_release_fields(contents: &str) -> std::collections::BTreeMap<String, String> {
    let mut fields = std::collections::BTreeMap::new();
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some((key, value)) = trimmed.split_once('=') else {
            continue;
        };
        let parsed_value = parse_os_release_value(value.trim());
        fields.insert(key.trim().to_string(), parsed_value);
    }
    fields
}

fn parse_os_release_value(raw: &str) -> String {
    let mut value = raw.trim().to_string();
    if value.starts_with('"') && value.ends_with('"') && value.len() >= 2 {
        value = value[1..value.len() - 1].to_string();
    }
    value
        .replace("\\\"", "\"")
        .replace("\\\\", "\\")
        .replace("\\n", "\n")
}

pub(crate) fn resolve_default_tui_target_for(current_dir: &Path) -> Result<DefaultTuiTarget> {
    let manifests = discover_project_manifests(current_dir)?;
    let initial_project = if manifests.len() == 1 {
        manifests.into_iter().next()
    } else {
        None
    };
    Ok(DefaultTuiTarget {
        workspace_root: current_dir.to_path_buf(),
        initial_project,
    })
}

fn run_default_tui(_debug: bool) -> Result<()> {
    let current_dir = std::env::current_dir().context("failed to read current directory")?;
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        anyhow::bail!(
            "interactive petiglyph TUI requires a terminal in {}",
            current_dir.display()
        );
    }
    let target = resolve_default_tui_target_for(&current_dir)?;
    tui_workspace(
        target.workspace_root,
        target.initial_project,
        None,
        None,
        None,
        None,
    )
}

fn run_cli(cli: Cli) -> std::result::Result<(), CliRunError> {
    match cli.command {
        None => run_default_tui(cli.debug).map_err(CliRunError::Plain),
        Some(CliCommand::Create { name, no_launch }) => {
            create_project(&name, no_launch).map_err(CliRunError::Plain)
        }
        Some(CliCommand::List { json }) => {
            run_automation_command("list", json, list_command, print_list_result)
        }
        Some(CliCommand::Delete { manifest, json }) => run_automation_command(
            "delete",
            json,
            || delete_command(manifest_path_from_option(manifest)?),
            print_delete_result,
        ),
        Some(CliCommand::SetThreshold {
            image_name,
            threshold,
            manifest,
            json,
        }) => run_automation_command(
            "set-threshold",
            json,
            || set_threshold_command(manifest_path_from_option(manifest)?, &image_name, threshold),
            print_set_threshold_result,
        ),
        Some(CliCommand::ClearThreshold {
            image_name,
            manifest,
            json,
        }) => run_automation_command(
            "clear-threshold",
            json,
            || clear_threshold_command(manifest_path_from_option(manifest)?, &image_name),
            print_clear_threshold_result,
        ),
        Some(CliCommand::Glyph { command }) => run_glyph_command(command),
        Some(CliCommand::Grid { command }) => run_grid_command(command),
        Some(CliCommand::Composition { command }) => run_composition_command(command),
        Some(CliCommand::Animation { command }) => run_animation_command(command),
        Some(CliCommand::Tui {
            manifest,
            input_dir,
            threshold,
            glyph_size,
            codepoint_start,
        }) => {
            let current_dir = std::env::current_dir()
                .context("failed to read current directory")
                .map_err(CliRunError::Plain)?;
            if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
                return Err(CliRunError::Plain(anyhow::anyhow!(
                    "interactive petiglyph TUI requires a terminal in {}",
                    current_dir.display()
                )));
            }
            match manifest {
                Some(path) => tui(path, input_dir, threshold, glyph_size, codepoint_start)
                    .map_err(CliRunError::Plain),
                None => {
                    let target =
                        resolve_default_tui_target_for(&current_dir).map_err(CliRunError::Plain)?;
                    if target.initial_project.is_none()
                        && (input_dir.is_some()
                            || threshold.is_some()
                            || glyph_size.is_some()
                            || codepoint_start.is_some())
                    {
                        return Err(CliRunError::Plain(anyhow::anyhow!(
                            "--input-dir/--threshold/--glyph-size/--codepoint-start require a concrete project; choose a project in Welcome first or pass --manifest"
                        )));
                    }
                    tui_workspace(
                        target.workspace_root,
                        target.initial_project,
                        input_dir,
                        threshold,
                        glyph_size,
                        codepoint_start,
                    )
                    .map_err(CliRunError::Plain)
                }
            }
        }
        Some(CliCommand::Build {
            manifest,
            input_dir,
            out_dir,
            threshold,
            glyph_size,
            codepoint_start,
            json,
            force_remap,
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
                    force_remap,
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
            force_remap,
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
                    force_remap,
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
            force_remap,
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
                    force_remap,
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
        Some(CliCommand::Uninstall) => {
            let colors = CliColors::new();
            eprintln!(
                "petiglyph: {}uninstall is ambiguous.{}",
                colors.bold_red, colors.reset
            );
            eprintln!("Did you mean `uninstall-font` or `nuke-everything`?");
            eprintln!();
            eprintln!(
                "  {}uninstall-font{}  - Removes only the font variants generated by your active project.",
                colors.bold, colors.reset
            );
            eprintln!(
                "  {}nuke-everything{}   - Deletes all petiglyph traces on this machine (all fonts, all projects, all metadata). NUKED AND LOST.",
                colors.bold, colors.reset
            );
            std::process::exit(1);
        }
        Some(CliCommand::NukeEverything { json }) => run_automation_command(
            "nuke-everything",
            json,
            uninstall_tool_command,
            print_uninstall_tool_result,
        ),
        Some(CliCommand::Doctor {
            manifest,
            repair,
            json,
        }) => run_automation_command(
            "doctor",
            json,
            || doctor_command(manifest, repair),
            print_doctor_result,
        ),
    }
}

fn run_glyph_command(command: GlyphCommand) -> std::result::Result<(), CliRunError> {
    match command {
        GlyphCommand::Create {
            manifest,
            input,
            threshold,
            invert,
            grayscale_enabled,
            grayscale_brightness,
            grayscale_contrast,
            grayscale_gamma_percent,
            json,
        } => run_automation_command(
            "glyph.create",
            json,
            || {
                create_glyphs_command(
                    manifest_path_from_option(manifest)?,
                    input,
                    threshold,
                    invert,
                    grayscale_options(
                        grayscale_enabled,
                        grayscale_brightness,
                        grayscale_contrast,
                        grayscale_gamma_percent,
                    )?,
                )
            },
            print_imported_sources_result,
        ),
        GlyphCommand::SetThreshold {
            image_name,
            threshold,
            manifest,
            json,
        } => run_automation_command(
            "glyph.set-threshold",
            json,
            || set_threshold_command(manifest_path_from_option(manifest)?, &image_name, threshold),
            print_set_threshold_result,
        ),
        GlyphCommand::ClearThreshold {
            image_name,
            manifest,
            json,
        } => run_automation_command(
            "glyph.clear-threshold",
            json,
            || clear_threshold_command(manifest_path_from_option(manifest)?, &image_name),
            print_clear_threshold_result,
        ),
        GlyphCommand::SetInvert {
            image_name,
            invert,
            manifest,
            json,
        } => run_automation_command(
            "glyph.set-invert",
            json,
            || set_invert_command(manifest_path_from_option(manifest)?, &image_name, invert),
            print_set_invert_result,
        ),
    }
}

fn run_grid_command(command: GridCommand) -> std::result::Result<(), CliRunError> {
    match command {
        GridCommand::Create {
            manifest,
            input,
            rows,
            cols,
            horizontal_bleed,
            vertical_bleed,
            threshold,
            invert,
            grayscale_enabled,
            grayscale_brightness,
            grayscale_contrast,
            grayscale_gamma_percent,
            json,
        } => run_automation_command(
            "grid.create",
            json,
            || {
                create_grid_command(
                    manifest_path_from_option(manifest)?,
                    input,
                    rows,
                    cols,
                    horizontal_bleed.into(),
                    vertical_bleed.into(),
                    threshold,
                    invert,
                    grayscale_options(
                        grayscale_enabled,
                        grayscale_brightness,
                        grayscale_contrast,
                        grayscale_gamma_percent,
                    )?,
                )
            },
            print_imported_sources_result,
        ),
    }
}

fn run_composition_command(command: CompositionCommand) -> std::result::Result<(), CliRunError> {
    match command {
        CompositionCommand::Set {
            source_key,
            manifest,
            rows,
            cols,
            horizontal_bleed,
            vertical_bleed,
            json,
        } => run_automation_command(
            "composition.set",
            json,
            || {
                set_composition_command(
                    manifest_path_from_option(manifest)?,
                    source_key,
                    rows,
                    cols,
                    horizontal_bleed.into(),
                    vertical_bleed.into(),
                )
            },
            print_composition_result,
        ),
        CompositionCommand::Clear {
            source_key,
            manifest,
            json,
        } => run_automation_command(
            "composition.clear",
            json,
            || clear_composition_command(manifest_path_from_option(manifest)?, source_key),
            print_composition_result,
        ),
    }
}

fn run_animation_command(command: AnimationCommand) -> std::result::Result<(), CliRunError> {
    match command {
        AnimationCommand::CreateStandard {
            manifest,
            input,
            name,
            fps,
            threshold,
            invert,
            grayscale_enabled,
            grayscale_brightness,
            grayscale_contrast,
            grayscale_gamma_percent,
            json,
        } => run_automation_command(
            "animation.create-standard",
            json,
            || {
                create_animation_command(
                    manifest_path_from_option(manifest)?,
                    input,
                    name,
                    AnimationType::Standard,
                    fps,
                    None,
                    None,
                    None,
                    None,
                    threshold,
                    invert,
                    grayscale_options(
                        grayscale_enabled,
                        grayscale_brightness,
                        grayscale_contrast,
                        grayscale_gamma_percent,
                    )?,
                )
            },
            print_animation_mutation_result,
        ),
        AnimationCommand::CreateGrid {
            manifest,
            input,
            name,
            fps,
            rows,
            cols,
            horizontal_bleed,
            vertical_bleed,
            threshold,
            invert,
            grayscale_enabled,
            grayscale_brightness,
            grayscale_contrast,
            grayscale_gamma_percent,
            json,
        } => run_automation_command(
            "animation.create-grid",
            json,
            || {
                create_animation_command(
                    manifest_path_from_option(manifest)?,
                    input,
                    name,
                    AnimationType::Grid,
                    fps,
                    Some(rows),
                    Some(cols),
                    Some(horizontal_bleed.into()),
                    Some(vertical_bleed.into()),
                    threshold,
                    invert,
                    grayscale_options(
                        grayscale_enabled,
                        grayscale_brightness,
                        grayscale_contrast,
                        grayscale_gamma_percent,
                    )?,
                )
            },
            print_animation_mutation_result,
        ),
        AnimationCommand::SetFps {
            name,
            fps,
            manifest,
            json,
        } => run_automation_command(
            "animation.set-fps",
            json,
            || animation_set_fps_command(manifest_path_from_option(manifest)?, name, fps),
            print_animation_mutation_result,
        ),
        AnimationCommand::Delete {
            name,
            manifest,
            json,
        } => run_automation_command(
            "animation.delete",
            json,
            || animation_delete_command(manifest_path_from_option(manifest)?, name),
            print_animation_mutation_result,
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

struct CliColors {
    bold_green: &'static str,
    bold_red: &'static str,
    yellow: &'static str,
    red: &'static str,
    green: &'static str,
    cyan: &'static str,
    bold: &'static str,
    reset: &'static str,
}

impl CliColors {
    fn new() -> Self {
        if io::stdout().is_terminal() {
            Self {
                bold_green: "\x1b[1;32m",
                bold_red: "\x1b[1;31m",
                yellow: "\x1b[33m",
                red: "\x1b[31m",
                green: "\x1b[32m",
                cyan: "\x1b[36m",
                bold: "\x1b[1m",
                reset: "\x1b[0m",
            }
        } else {
            Self {
                bold_green: "",
                bold_red: "",
                yellow: "",
                red: "",
                green: "",
                cyan: "",
                bold: "",
                reset: "",
            }
        }
    }
}

fn print_list_result(data: &ListCommandData) {
    let colors = CliColors::new();
    println!(
        "{}workspace:{}\n  {}",
        colors.cyan, colors.reset, data.workspace_dir
    );
    println!();
    println!("{}projects:{}", colors.cyan, colors.reset);
    if data.projects.is_empty() {
        println!("  (none found)");
    } else {
        for project in &data.projects {
            println!(
                "  - {}{}{} ({})",
                colors.bold, project.font_name, colors.reset, project.manifest_path
            );
        }
    }
    println!();
    println!("{}installed fonts:{}", colors.cyan, colors.reset);
    if data.installed_fonts.is_empty() {
        println!("  (none found)");
    } else {
        for font in &data.installed_fonts {
            println!(
                "  - {}{}{} ({})",
                colors.bold, font.file_name, colors.reset, font.path
            );
        }
    }
    if let Some(summary) = &data.pua_usage {
        println!();
        println!("{}supplementary pua usage:{}", colors.cyan, colors.reset);
        println!(
            "  petiglyph: {}/{} codepoints",
            format_count_k(summary.petiglyph_occupied),
            format_count_k(summary.supplementary_pua_total)
        );
        println!(
            "  external:  {} codepoints",
            format_count_k(summary.external_occupied)
        );
        println!(
            "  available: {} codepoints",
            format_count_k(summary.available)
        );
        if summary.petiglyph_occupied >= 10_000 {
            println!(
                "  note:      petiglyph managed usage is above 10k ({})",
                format_count_k(summary.petiglyph_occupied)
            );
        }
    }
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

fn print_delete_result(data: &DeleteCommandData) {
    let colors = CliColors::new();
    println!(
        "petiglyph: {}project deleted{}",
        colors.bold_red, colors.reset
    );
    println!("  manifest:  {}", data.manifest);
    println!("  directory: {}", data.deleted_dir);
}

fn print_set_threshold_result(data: &SetThresholdCommandData) {
    let colors = CliColors::new();
    println!(
        "petiglyph: {}threshold updated{}",
        colors.bold_green, colors.reset
    );
    println!("  manifest:  {}", data.manifest);
    println!(
        "  image:     {}{}{}",
        colors.bold, data.image_name, colors.reset
    );
    println!(
        "  threshold: {}{}{}",
        colors.bold, data.threshold, colors.reset
    );
}

fn print_clear_threshold_result(data: &ClearThresholdCommandData) {
    let colors = CliColors::new();
    if data.was_present {
        println!(
            "petiglyph: {}threshold cleared{}",
            colors.bold_green, colors.reset
        );
    } else {
        println!(
            "petiglyph: {}no threshold override found to clear{}",
            colors.yellow, colors.reset
        );
    }
    println!("  manifest:  {}", data.manifest);
    println!(
        "  image:     {}{}{}",
        colors.bold, data.image_name, colors.reset
    );
}

fn print_build_result(data: &BuildCommandData) {
    let colors = CliColors::new();
    println!(
        "petiglyph: {}build complete{}",
        colors.bold_green, colors.reset
    );
    println!(
        "  font:                {}{}{}",
        colors.bold, data.font_name, colors.reset
    );
    println!(
        "  glyphs:              {}{}{}",
        colors.bold, data.glyph_count, colors.reset
    );
    println!("  threshold:           {}", data.threshold);
    println!("  threshold-overrides: {}", data.threshold_overrides);
    println!("  glyph-size:          {}", data.glyph_size);
    println!("  codepoint-start:     {}", data.codepoint_start);
    println!("  manifest:            {}", data.manifest);
    println!("  input-dir:           {}", data.input_dir);
    println!("  out-dir:             {}", data.out_dir);
    println!("  bdf:                 {}", data.bdf);
    println!("  ttf:                 {}", data.ttf);
    println!("  map:                 {}", data.map);
    println!("  sample:              {}", data.sample);
    println!("  previews:            {}", data.previews);
}

fn print_sample_result(data: &SampleCommandData) {
    let colors = CliColors::new();
    println!("{}petiglyph sample{}", colors.bold_green, colors.reset);
    println!(
        "  font:                {}{}{}",
        colors.bold, data.build.font_name, colors.reset
    );
    println!("  glyphs:              {}", data.build.glyph_count);
    println!("  threshold:           {}", data.build.threshold);
    println!("  threshold-overrides: {}", data.build.threshold_overrides);
    println!("  glyph-size:          {}", data.build.glyph_size);
    println!("  ttf:                 {}", data.build.ttf);
    println!("  installed:           {}", data.installed_ttf);
    println!("  sample:              {}", data.build.sample);
    if let Some(coverage) = &data.coverage {
        let cov = coverage
            .checked_codepoints
            .saturating_sub(coverage.missing_codepoints);
        let cov_color = if coverage.missing_codepoints > 0 {
            colors.yellow
        } else {
            colors.green
        };
        println!(
            "  coverage:            {}{}/{}{} codepoints resolved to managed petiglyph fonts",
            cov_color, cov, coverage.checked_codepoints, colors.reset
        );
        if coverage.missing_codepoints > 0 {
            println!(
                "  {}warning:{}             {} sample glyph(s) may render as '?'",
                colors.yellow, colors.reset, coverage.missing_codepoints
            );
        }
    }
    for hint in sample_terminal_rendering_hints(&data.sample_string) {
        println!(
            "  {}hint:{}                {}",
            colors.yellow, colors.reset, hint
        );
    }
    println!();
    println!("{}", data.sample_string);
}

pub(crate) fn sample_terminal_rendering_hints(sample: &str) -> Vec<String> {
    let mut hints = Vec::new();
    let has_private_use = sample
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .map(|ch| ch as u32)
        .any(is_private_use_codepoint);
    let has_multiline_grid = sample.contains('\n');

    if has_private_use {
        hints.push(
            "sample uses Private Use codepoints (East Asian Ambiguous width by Unicode); keep ambiguous width as single-cell for stable alignment".to_string(),
        );
        let term_program = std::env::var("TERM_PROGRAM").unwrap_or_default();
        let term = std::env::var("TERM").unwrap_or_default();
        if term_program.eq_ignore_ascii_case("WezTerm") {
            hints.push(
                "WezTerm: keep `treat_east_asian_ambiguous_width_as_wide = false`".to_string(),
            );
        } else if term_program.eq_ignore_ascii_case("iTerm.app")
            || term_program.eq_ignore_ascii_case("iTerm2")
        {
            hints.push(
                "iTerm2: disable “Ambiguous characters are double-width” in Profiles > Text"
                    .to_string(),
            );
        } else if term.contains("kitty") {
            hints.push(
                "kitty: if any symbol looks clipped/scaled, force one-cell width with `narrow_symbols U+F0000-U+10FFFD 1`".to_string(),
            );
        }
    }

    if has_multiline_grid {
        hints.push(
            "composite rows assume tightly stacked terminal cells; avoid custom line-height/cell-height tweaks while verifying seams"
                .to_string(),
        );
    }

    hints
}

pub(crate) fn is_private_use_codepoint(codepoint: u32) -> bool {
    (0xE000..=0xF8FF).contains(&codepoint)
        || (0xF0000..=0xFFFFD).contains(&codepoint)
        || (0x100000..=0x10FFFD).contains(&codepoint)
}

fn print_install_result(data: &InstallFontCommandData) {
    let colors = CliColors::new();
    println!(
        "petiglyph: {}font installed{}",
        colors.bold_green, colors.reset
    );
    println!("  source:                 {}", data.build.ttf);
    println!("  installed:              {}", data.installed_ttf);
    println!("  install-dir:            {}", data.install_dir);
    println!(
        "  replaced-previous-ttfs: {}",
        data.replaced_previous_ttf_count
    );
}

fn print_uninstall_result(data: &UninstallFontCommandData) {
    let colors = CliColors::new();
    match data.outcome {
        UninstallOutcome::Removed => {
            println!(
                "petiglyph: {}font uninstalled{}",
                colors.bold_green, colors.reset
            );
            println!("  removed-ttfs: {}", data.removed_ttf_count);
        }
        UninstallOutcome::AlreadyAbsent => {
            println!(
                "petiglyph: {}font already absent{}",
                colors.yellow, colors.reset
            );
        }
    }
    println!("  manifest:     {}", data.manifest);
    println!("  install-dir:  {}", data.install_dir);
}

fn print_uninstall_tool_result(data: &UninstallToolCommandData) {
    let colors = CliColors::new();
    match data.outcome {
        UninstallOutcome::Removed => {
            println!(
                "petiglyph: {}tool state uninstalled{}",
                colors.bold_green, colors.reset
            );
            println!("  removed-ttfs:        {}", data.removed_ttf_count);
            println!("  removed-metadata:    {}", data.removed_metadata_count);
            println!("  removed-state-files: {}", data.removed_state_file_count);
        }
        UninstallOutcome::AlreadyAbsent => {
            println!(
                "petiglyph: {}tool state already absent{}",
                colors.yellow, colors.reset
            );
        }
    }
    println!("  install-dir:         {}", data.install_dir);
    if let Some(path) = &data.binary_path {
        println!();
        println!("petiglyph binary is at: {}", path);
        println!("  remove with: rm {}", path);
    }
}

fn print_doctor_result(data: &DoctorReport) {
    let colors = CliColors::new();

    if data.healthy {
        println!(
            "petiglyph doctor: {}healthy{}",
            colors.bold_green, colors.reset
        );
    } else {
        println!(
            "petiglyph doctor: {}issues detected{}",
            colors.bold_red, colors.reset
        );
    }

    let w_color = if data.warnings > 0 { colors.yellow } else { "" };
    let e_color = if data.errors > 0 { colors.red } else { "" };
    let r_color = if data.repaired > 0 { colors.green } else { "" };

    println!(
        "  warnings:    {}{}{}",
        w_color, data.warnings, colors.reset
    );
    println!("  errors:      {}{}{}", e_color, data.errors, colors.reset);
    println!(
        "  repaired:    {}{}{}",
        r_color, data.repaired, colors.reset
    );
    println!("  install-dir: {}", data.install_dir);
    println!("  registry:    {}", data.registry_path);
    if let Some(manifest) = &data.manifest {
        println!("  manifest:    {}", manifest);
    }
    if let Some(project_id) = &data.project_id {
        println!("  project-id:  {}", project_id);
    }
    println!();
    for finding in &data.findings {
        let sev_color = match finding.severity {
            crate::doctor::DoctorSeverity::Info => colors.cyan,
            crate::doctor::DoctorSeverity::Warning => colors.yellow,
            crate::doctor::DoctorSeverity::Error => colors.red,
        };
        let status_color = match finding.status {
            crate::doctor::DoctorStatus::Ok => colors.green,
            crate::doctor::DoctorStatus::Issue => colors.red,
            crate::doctor::DoctorStatus::Repaired => colors.green,
        };
        println!(
            "- [{}{:?}{}/{}{:?}{}] {}{}{}: {}",
            sev_color,
            finding.severity,
            colors.reset,
            status_color,
            finding.status,
            colors.reset,
            colors.bold,
            finding.code,
            colors.reset,
            finding.message
        );
    }
}

fn list_command() -> Result<ListCommandData> {
    let current_dir = std::env::current_dir().context("failed to read current directory")?;
    let manifests = discover_project_manifests(&current_dir)?;
    let mut projects = Vec::new();
    for manifest_path in manifests {
        if let Ok(manifest) = read_manifest(&manifest_path) {
            projects.push(ListProjectData {
                manifest_path: manifest_path.display().to_string(),
                font_name: manifest.font_name,
            });
        }
    }

    let mut installed_fonts = Vec::new();
    if let Ok(install_dir) = crate::install::managed_install_dir()
        && install_dir.is_dir()
    {
        if let Ok(entries) = std::fs::read_dir(&install_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let is_ttf = path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .map(|ext| ext.eq_ignore_ascii_case("ttf"))
                    .unwrap_or(false);
                if path.is_file() && is_ttf {
                    let file_name = path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("unknown.ttf")
                        .to_string();
                    installed_fonts.push(ListInstalledFontData {
                        file_name,
                        path: path.display().to_string(),
                    });
                }
            }
        }
        installed_fonts.sort_by(|a, b| a.file_name.cmp(&b.file_name));
    }

    let pua_usage = supplementary_pua_usage_summary().ok();

    Ok(ListCommandData {
        workspace_dir: current_dir.display().to_string(),
        projects,
        installed_fonts,
        pua_usage,
    })
}

fn delete_command(manifest_path: PathBuf) -> Result<DeleteCommandData> {
    let project_dir = delete_project_for_manifest(&manifest_path)?;
    Ok(DeleteCommandData {
        manifest: manifest_path.display().to_string(),
        deleted_dir: project_dir.display().to_string(),
    })
}

fn set_threshold_command(
    manifest_path: PathBuf,
    image_name: &str,
    threshold: u8,
) -> Result<SetThresholdCommandData> {
    let mut manifest = read_manifest(&manifest_path)?;
    manifest
        .threshold_overrides
        .insert(image_name.to_string(), threshold);
    write_manifest(&manifest_path, &manifest)?;
    Ok(SetThresholdCommandData {
        manifest: manifest_path.display().to_string(),
        image_name: image_name.to_string(),
        threshold,
    })
}

fn clear_threshold_command(
    manifest_path: PathBuf,
    image_name: &str,
) -> Result<ClearThresholdCommandData> {
    let mut manifest = read_manifest(&manifest_path)?;
    let was_present = manifest.threshold_overrides.remove(image_name).is_some();
    if was_present {
        write_manifest(&manifest_path, &manifest)?;
    }
    Ok(ClearThresholdCommandData {
        manifest: manifest_path.display().to_string(),
        image_name: image_name.to_string(),
        was_present,
    })
}

fn grayscale_options(
    enabled: bool,
    brightness: i16,
    contrast: i16,
    gamma_percent: u16,
) -> Result<crate::animation_media::AnimationImportProcessingOptions> {
    if !(-80..=80).contains(&brightness) {
        anyhow::bail!("grayscale brightness must be in -80..=80");
    }
    if !(-80..=80).contains(&contrast) {
        anyhow::bail!("grayscale contrast must be in -80..=80");
    }
    if !(50..=200).contains(&gamma_percent) {
        anyhow::bail!("grayscale gamma percent must be in 50..=200");
    }
    Ok(crate::animation_media::AnimationImportProcessingOptions {
        grayscale_enabled: enabled,
        grayscale: crate::animation_media::AnimationGrayscaleOptions {
            brightness,
            contrast,
            gamma_percent,
        },
    })
}

fn create_glyphs_command(
    manifest_path: PathBuf,
    input: Vec<PathBuf>,
    threshold: u8,
    invert: InvertValue,
    processing: crate::animation_media::AnimationImportProcessingOptions,
) -> Result<ImportedSourcesCommandData> {
    validate_threshold(threshold)?;
    let mut manifest = read_manifest(&manifest_path)?;
    let input_dir = manifest_input_dir(&manifest_path, &manifest);
    let imported = import_static_sources(&input_dir, input, processing)?;
    if imported.is_empty() {
        anyhow::bail!("no importable images were added");
    }
    for source in &imported {
        manifest
            .threshold_overrides
            .insert(source.clone(), threshold);
        if matches!(invert, InvertValue::On) {
            manifest.invert_overrides.insert(source.clone(), true);
        } else {
            manifest.invert_overrides.remove(source);
        }
    }
    write_manifest(&manifest_path, &manifest)?;
    Ok(ImportedSourcesCommandData {
        manifest: manifest_path.display().to_string(),
        imported_sources: imported,
    })
}

#[allow(clippy::too_many_arguments)]
fn create_grid_command(
    manifest_path: PathBuf,
    input: Vec<PathBuf>,
    rows: usize,
    cols: usize,
    horizontal_bleed: BleedLevel,
    vertical_bleed: BleedLevel,
    threshold: u8,
    invert: InvertValue,
    processing: crate::animation_media::AnimationImportProcessingOptions,
) -> Result<ImportedSourcesCommandData> {
    validate_threshold(threshold)?;
    validate_grid(rows, cols)?;
    let mut manifest = read_manifest(&manifest_path)?;
    let input_dir = manifest_input_dir(&manifest_path, &manifest);
    let imported = import_static_sources(&input_dir, input, processing)?;
    if imported.len() != 1 {
        anyhow::bail!("grid create requires exactly one imported source");
    }
    let source = imported[0].clone();
    manifest
        .threshold_overrides
        .insert(source.clone(), threshold);
    if matches!(invert, InvertValue::On) {
        manifest.invert_overrides.insert(source.clone(), true);
    } else {
        manifest.invert_overrides.remove(&source);
    }
    manifest.compositions.insert(
        source,
        CompositionDef {
            rows,
            cols,
            horizontal_bleed,
            vertical_bleed,
        },
    );
    write_manifest(&manifest_path, &manifest)?;
    Ok(ImportedSourcesCommandData {
        manifest: manifest_path.display().to_string(),
        imported_sources: imported,
    })
}

fn set_invert_command(
    manifest_path: PathBuf,
    image_name: &str,
    invert: InvertValue,
) -> Result<SetInvertCommandData> {
    let mut manifest = read_manifest(&manifest_path)?;
    let invert_bool = matches!(invert, InvertValue::On);
    if invert_bool {
        manifest
            .invert_overrides
            .insert(image_name.to_string(), true);
    } else {
        manifest.invert_overrides.remove(image_name);
    }
    write_manifest(&manifest_path, &manifest)?;
    Ok(SetInvertCommandData {
        manifest: manifest_path.display().to_string(),
        image_name: image_name.to_string(),
        invert: invert_bool,
    })
}

fn set_composition_command(
    manifest_path: PathBuf,
    source_key: String,
    rows: usize,
    cols: usize,
    horizontal_bleed: BleedLevel,
    vertical_bleed: BleedLevel,
) -> Result<CompositionCommandData> {
    validate_grid(rows, cols)?;
    let mut manifest = read_manifest(&manifest_path)?;
    manifest.compositions.insert(
        source_key.clone(),
        CompositionDef {
            rows,
            cols,
            horizontal_bleed,
            vertical_bleed,
        },
    );
    write_manifest(&manifest_path, &manifest)?;
    Ok(CompositionCommandData {
        manifest: manifest_path.display().to_string(),
        source_key,
        rows: Some(rows),
        cols: Some(cols),
    })
}

fn clear_composition_command(
    manifest_path: PathBuf,
    source_key: String,
) -> Result<CompositionCommandData> {
    let mut manifest = read_manifest(&manifest_path)?;
    manifest.compositions.remove(&source_key);
    write_manifest(&manifest_path, &manifest)?;
    Ok(CompositionCommandData {
        manifest: manifest_path.display().to_string(),
        source_key,
        rows: None,
        cols: None,
    })
}

#[allow(clippy::too_many_arguments)]
fn create_animation_command(
    manifest_path: PathBuf,
    input: Vec<PathBuf>,
    name: Option<String>,
    animation_type: AnimationType,
    fps: u8,
    rows: Option<usize>,
    cols: Option<usize>,
    horizontal_bleed: Option<BleedLevel>,
    vertical_bleed: Option<BleedLevel>,
    threshold: u8,
    invert: InvertValue,
    processing: crate::animation_media::AnimationImportProcessingOptions,
) -> Result<AnimationMutationCommandData> {
    validate_threshold(threshold)?;
    validate_fps(fps)?;
    let mut manifest = read_manifest(&manifest_path)?;
    let input_dir = manifest_input_dir(&manifest_path, &manifest);
    let payload = input
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let media = crate::animation_media::import_animation_media_to_input(
        &input_dir,
        &payload,
        crate::animation_media::ExistingImportPolicy::ReuseIdentical,
        processing,
    )?;
    let mut frames = media.imported_source_keys;
    if frames.is_empty() {
        anyhow::bail!("animation import produced no frames");
    }
    frames.sort_by_key(|k| natural_sort_key(k));
    frames.dedup();

    for source in &frames {
        manifest
            .threshold_overrides
            .insert(source.clone(), threshold);
        if matches!(invert, InvertValue::On) {
            manifest.invert_overrides.insert(source.clone(), true);
        } else {
            manifest.invert_overrides.remove(source);
        }
    }

    let animation_name = unique_animation_name(&manifest, name.as_deref(), &frames);
    if manifest.animations.iter().any(|a| a.name == animation_name) {
        anyhow::bail!("animation `{animation_name}` already exists");
    }

    let (final_frames, grid_rows, grid_cols, grid_hb, grid_vb) =
        if animation_type == AnimationType::Grid {
            let rows = rows.ok_or_else(|| anyhow::anyhow!("grid animation requires --rows"))?;
            let cols = cols.ok_or_else(|| anyhow::anyhow!("grid animation requires --cols"))?;
            validate_grid(rows, cols)?;
            let hb = horizontal_bleed.unwrap_or(BleedLevel::Weak);
            let vb = vertical_bleed.unwrap_or(BleedLevel::Off);
            let desired = CompositionDef {
                rows,
                cols,
                horizontal_bleed: hb,
                vertical_bleed: vb,
            };
            let mut resolved = Vec::with_capacity(frames.len());
            for frame in &frames {
                if let Some(existing) = manifest.compositions.get(frame) {
                    if existing != &desired {
                        let dup = duplicate_source_key_for_grid_conflict(&input_dir, frame)?;
                        manifest.compositions.insert(dup.clone(), desired.clone());
                        resolved.push(dup);
                        continue;
                    }
                    resolved.push(frame.clone());
                } else {
                    manifest.compositions.insert(frame.clone(), desired.clone());
                    resolved.push(frame.clone());
                }
            }
            (resolved, Some(rows), Some(cols), Some(hb), Some(vb))
        } else {
            (frames, None, None, None, None)
        };

    manifest.animations.push(AnimationDef {
        name: animation_name.clone(),
        animation_type,
        fps,
        frames: final_frames.clone(),
        rows: grid_rows,
        cols: grid_cols,
        horizontal_bleed: grid_hb,
        vertical_bleed: grid_vb,
        grayscale_processing: Some(processing),
    });
    write_manifest(&manifest_path, &manifest)?;
    Ok(AnimationMutationCommandData {
        manifest: manifest_path.display().to_string(),
        name: animation_name,
        fps: Some(fps),
        frame_count: Some(final_frames.len()),
    })
}

fn animation_set_fps_command(
    manifest_path: PathBuf,
    name: String,
    fps: u8,
) -> Result<AnimationMutationCommandData> {
    validate_fps(fps)?;
    let mut manifest = read_manifest(&manifest_path)?;
    let Some(anim) = manifest.animations.iter_mut().find(|a| a.name == name) else {
        anyhow::bail!("animation not found: {name}");
    };
    anim.fps = fps;
    let frame_count = anim.frames.len();
    write_manifest(&manifest_path, &manifest)?;
    Ok(AnimationMutationCommandData {
        manifest: manifest_path.display().to_string(),
        name,
        fps: Some(fps),
        frame_count: Some(frame_count),
    })
}

fn animation_delete_command(
    manifest_path: PathBuf,
    name: String,
) -> Result<AnimationMutationCommandData> {
    let mut manifest = read_manifest(&manifest_path)?;
    let before = manifest.animations.len();
    manifest.animations.retain(|a| a.name != name);
    if before == manifest.animations.len() {
        anyhow::bail!("animation not found: {name}");
    }
    write_manifest(&manifest_path, &manifest)?;
    Ok(AnimationMutationCommandData {
        manifest: manifest_path.display().to_string(),
        name,
        fps: None,
        frame_count: None,
    })
}

fn validate_threshold(threshold: u8) -> Result<()> {
    let _ = threshold;
    Ok(())
}

fn validate_fps(fps: u8) -> Result<()> {
    if !(1..=30).contains(&fps) {
        anyhow::bail!("fps must be in 1..=30");
    }
    Ok(())
}

fn validate_grid(rows: usize, cols: usize) -> Result<()> {
    if rows == 0 || cols == 0 {
        anyhow::bail!("rows and cols must be > 0");
    }
    Ok(())
}

fn manifest_input_dir(manifest_path: &Path, manifest: &Manifest) -> PathBuf {
    manifest_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(&manifest.input_dir)
}

fn import_static_sources(
    input_dir: &Path,
    inputs: Vec<PathBuf>,
    processing: crate::animation_media::AnimationImportProcessingOptions,
) -> Result<Vec<String>> {
    fs::create_dir_all(input_dir)
        .with_context(|| format!("failed to create {}", input_dir.display()))?;
    let mut out = Vec::new();
    for src in inputs {
        if !src.is_file() {
            anyhow::bail!("input is not a file: {}", src.display());
        }
        if !is_supported_source(&src) {
            anyhow::bail!("unsupported image type: {}", src.display());
        }
        let file_name = src
            .file_name()
            .ok_or_else(|| anyhow::anyhow!("input has no file name: {}", src.display()))?;
        let mut dest = input_dir.join(file_name);
        if dest.exists() {
            dest = next_available_import_destination(input_dir, file_name);
        }
        fs::copy(&src, &dest).with_context(|| {
            format!("failed to import {} into {}", src.display(), dest.display())
        })?;
        if processing.grayscale_enabled && should_apply_static_import_grayscale(&dest) {
            let _ = crate::animation_media::apply_grayscale_processing_to_image_file(
                &dest,
                processing.grayscale,
            );
        }
        out.push(source_key_from_input_path(input_dir, &dest));
    }
    Ok(out)
}

fn next_available_import_destination(input_dir: &Path, file_name: &std::ffi::OsStr) -> PathBuf {
    let candidate = input_dir.join(file_name);
    if !candidate.exists() {
        return candidate;
    }
    let path = Path::new(file_name);
    let stem = path.file_stem().and_then(|v| v.to_str()).unwrap_or("glyph");
    let ext = path.extension().and_then(|v| v.to_str()).unwrap_or("");
    for idx in 2.. {
        let renamed = if ext.is_empty() {
            format!("{stem}-{idx}")
        } else {
            format!("{stem}-{idx}.{ext}")
        };
        let cand = input_dir.join(renamed);
        if !cand.exists() {
            return cand;
        }
    }
    candidate
}

fn should_apply_static_import_grayscale(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|ext| ext.to_str()).map(|e| e.to_ascii_lowercase()),
        Some(ext) if matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "webp" | "avif" | "bmp")
    )
}

fn source_key_from_input_path(input_dir: &Path, source_path: &Path) -> String {
    source_path
        .strip_prefix(input_dir)
        .unwrap_or(source_path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn duplicate_source_key_for_grid_conflict(input_dir: &Path, source_key: &str) -> Result<String> {
    let source_path = input_dir.join(source_key);
    let file_name = source_path
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("invalid source file path for {source_key}"))?;
    let duplicate_path = next_incremental_duplicate_destination(input_dir, Path::new(file_name))?;
    fs::copy(&source_path, &duplicate_path).with_context(|| {
        format!(
            "failed to duplicate source {} for grid conflict resolution",
            source_path.display()
        )
    })?;
    Ok(source_key_from_input_path(input_dir, &duplicate_path))
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
    for entry in fs::read_dir(input_dir)? {
        let path = entry?.path();
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
    while let Some((head, tail)) = current.rsplit_once('-') {
        if tail.is_empty() || !tail.chars().all(|ch| ch.is_ascii_digit()) {
            break;
        }
        current = head;
    }
    if current.is_empty() { stem } else { current }
}

fn unique_animation_name(manifest: &Manifest, provided: Option<&str>, frames: &[String]) -> String {
    let base = provided
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(slugify)
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| {
            let first = frames
                .first()
                .map(|f| slugify(f.trim_end_matches(".png")))
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| "animation".to_string());
            format!("{first}_anim")
        });
    let existing = manifest
        .animations
        .iter()
        .map(|a| a.name.clone())
        .collect::<BTreeSet<_>>();
    if !existing.contains(&base) {
        return base;
    }
    for idx in 2.. {
        let candidate = format!("{base}_{idx}");
        if !existing.contains(&candidate) {
            return candidate;
        }
    }
    base
}

fn natural_sort_key(value: &str) -> (String, usize) {
    let mut key = String::new();
    let mut num_buf = String::new();
    for ch in value.chars() {
        if ch.is_ascii_digit() {
            num_buf.push(ch);
            continue;
        }
        if !num_buf.is_empty() {
            key.push_str(&format!("{:010}", num_buf.parse::<u32>().unwrap_or(0)));
            num_buf.clear();
        }
        key.push(ch.to_ascii_lowercase());
    }
    if !num_buf.is_empty() {
        key.push_str(&format!("{:010}", num_buf.parse::<u32>().unwrap_or(0)));
    }
    (key, value.len())
}

fn print_imported_sources_result(data: &ImportedSourcesCommandData) {
    println!(
        "petiglyph: imported {} source(s)",
        data.imported_sources.len()
    );
    println!("  manifest: {}", data.manifest);
    for source in &data.imported_sources {
        println!("  - {}", source);
    }
}

fn print_set_invert_result(data: &SetInvertCommandData) {
    println!(
        "petiglyph: invert override {}",
        if data.invert { "on" } else { "off" }
    );
    println!("  manifest: {}", data.manifest);
    println!("  image:    {}", data.image_name);
}

fn print_composition_result(data: &CompositionCommandData) {
    println!("petiglyph: composition updated");
    println!("  manifest: {}", data.manifest);
    println!("  source:   {}", data.source_key);
    if let (Some(rows), Some(cols)) = (data.rows, data.cols) {
        println!("  grid:     {}x{}", rows, cols);
    }
}

fn print_animation_mutation_result(data: &AnimationMutationCommandData) {
    println!("petiglyph: animation updated");
    println!("  manifest: {}", data.manifest);
    println!("  name:     {}", data.name);
    if let Some(fps) = data.fps {
        println!("  fps:      {}", fps);
    }
    if let Some(frame_count) = data.frame_count {
        println!("  frames:   {}", frame_count);
    }
}

fn build_font(
    manifest_path: PathBuf,
    input_override: Option<PathBuf>,
    out_override: Option<PathBuf>,
    threshold_override: Option<u8>,
    glyph_size_override: Option<u32>,
    codepoint_start_override: Option<String>,
    force_remap: bool,
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
        anyhow::bail!("glyph_size must be > 0");
    }

    let summary = build_outputs_with_options(&config, BuildOptions { force_remap })?;
    Ok(build_command_data(&manifest_path, &config, &summary))
}

fn sample_command(
    manifest_path: PathBuf,
    input_override: Option<PathBuf>,
    out_override: Option<PathBuf>,
    threshold_override: Option<u8>,
    glyph_size_override: Option<u32>,
    codepoint_start_override: Option<String>,
    force_remap: bool,
) -> Result<SampleCommandData> {
    let mut config = load_runtime_config(
        &manifest_path,
        input_override,
        out_override,
        threshold_override,
        glyph_size_override,
        codepoint_start_override,
    )?;
    config.font_name =
        effective_font_name(&manifest_path, &config.font_name, DEFAULT_INSTALL_NAME_MODE)?;

    let summary = build_outputs_with_options(&config, BuildOptions { force_remap })?;
    let install_result = install_built_font(
        &manifest_path,
        &config.font_name,
        &config.project_id,
        &summary.ttf_path,
        summary.glyph_count,
    )?;
    let sample = std::fs::read_to_string(&summary.sample_path)
        .with_context(|| format!("failed to read {}", summary.sample_path.display()))?;
    let sample_string = sample.trim_end().to_string();
    let coverage = diagnose_sample_coverage(&sample_string).ok().flatten();
    Ok(SampleCommandData {
        build: build_command_data(&manifest_path, &config, &summary),
        sample_string,
        installed_ttf: install_result.install_path.display().to_string(),
        coverage,
    })
}

fn install_font_command(
    manifest_path: PathBuf,
    input_override: Option<PathBuf>,
    out_override: Option<PathBuf>,
    threshold_override: Option<u8>,
    glyph_size_override: Option<u32>,
    codepoint_start_override: Option<String>,
    force_remap: bool,
) -> Result<InstallFontCommandData> {
    let mut config = load_runtime_config(
        &manifest_path,
        input_override,
        out_override,
        threshold_override,
        glyph_size_override,
        codepoint_start_override,
    )?;
    config.font_name =
        effective_font_name(&manifest_path, &config.font_name, DEFAULT_INSTALL_NAME_MODE)?;

    let summary = build_outputs_with_options(&config, BuildOptions { force_remap })?;
    let install_result = install_built_font(
        &manifest_path,
        &config.font_name,
        &config.project_id,
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

fn uninstall_tool_command() -> Result<UninstallToolCommandData> {
    let uninstall = uninstall_tool_state()?;
    let binary_path = std::env::current_exe()
        .ok()
        .and_then(|p| p.to_str().map(String::from));
    Ok(UninstallToolCommandData {
        platform: uninstall.platform,
        install_dir: uninstall.install_dir.display().to_string(),
        outcome: uninstall.outcome,
        removed_ttf_count: uninstall.removed_ttf_count,
        removed_metadata_count: uninstall.removed_metadata_count,
        removed_state_file_count: uninstall.removed_state_file_count,
        binary_path,
    })
}

fn doctor_command(manifest: Option<PathBuf>, repair: bool) -> Result<DoctorReport> {
    doctor(repair, manifest)
}
