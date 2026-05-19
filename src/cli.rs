use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use serde::Serialize;
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::artifact_warning::incompatible_artifact_warning;
use crate::build::{BuildOptions, BuildSummary, build_outputs_with_options};
use crate::doctor::{DoctorReport, doctor};
use crate::install::{
    DEFAULT_INSTALL_NAME_MODE, FontPlatform, UninstallOutcome, diagnose_sample_coverage,
    effective_font_name, install_built_font, supplementary_pua_usage_summary,
    uninstall_project_font, uninstall_tool_state,
};
use crate::project::{
    RuntimeConfig, create_project, delete_project_for_manifest, discover_project_manifests,
    format_codepoint, load_runtime_config, manifest_path_from_option, read_manifest,
    write_manifest,
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
    /// Set a custom monochrome threshold override for a specific glyph source image.
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
    /// Clear a custom monochrome threshold override for a specific glyph source image.
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
    /// Build the font and print the sample private-use string.
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
    /// Completely nuke the petiglyph tool state for the current user (managed fonts + registry + metadata).
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
                    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
                        return Err(CliRunError::Plain(anyhow::anyhow!(
                            "interactive petiglyph TUI requires a terminal in {}",
                            current_dir.display()
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
            eprintln!(
                "Did you mean `{}` or `{}`?",
                "uninstall-font", "nuke-everything"
            );
            eprintln!();
            eprintln!(
                "  {}{}{}  - Removes only the font variants generated by your active project.",
                colors.bold, "uninstall-font", colors.reset
            );
            eprintln!(
                "  {}{}{}   - Deletes all petiglyph traces on this machine (all fonts, all projects, all metadata). NUKED AND LOST.",
                colors.bold, "nuke-everything", colors.reset
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
    if let Ok(install_dir) = crate::install::managed_install_dir() {
        if install_dir.is_dir() {
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
