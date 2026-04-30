use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use serde::Serialize;
use std::io::{self, IsTerminal};
use std::path::{Path, PathBuf};

use crate::artifact_warning::incompatible_artifact_warning;
use crate::build::{BuildOptions, BuildSummary, build_outputs_with_options};
use crate::doctor::{DoctorReport, doctor};
use crate::install::{
    DEFAULT_INSTALL_NAME_MODE, FontPlatform, UninstallOutcome, diagnose_sample_coverage,
    effective_font_name, install_built_font, uninstall_project_font,
};
use crate::project::{
    RuntimeConfig, create_project, discover_project_manifests, format_codepoint,
    load_runtime_config, manifest_path_from_option,
};
use crate::tui::{tui, tui_workspace};

const CLI_VERSION: &str = env!("CARGO_PKG_VERSION");

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

fn run_default_tui() -> Result<()> {
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
        None => run_default_tui().map_err(CliRunError::Plain),
        Some(CliCommand::Create { name, no_launch }) => {
            create_project(&name, no_launch).map_err(CliRunError::Plain)
        }
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
    println!("installed: {}", data.installed_ttf);
    println!("sample: {}", data.build.sample);
    if let Some(coverage) = &data.coverage {
        println!(
            "coverage: {}/{} codepoints resolved to managed petiglyph fonts",
            coverage
                .checked_codepoints
                .saturating_sub(coverage.missing_codepoints),
            coverage.checked_codepoints
        );
        if coverage.missing_codepoints > 0 {
            println!(
                "warning: {} sample glyph(s) may render as '?'",
                coverage.missing_codepoints
            );
        }
    }
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

fn print_doctor_result(data: &DoctorReport) {
    if data.healthy {
        println!("petiglyph doctor: healthy");
    } else {
        println!("petiglyph doctor: issues detected");
    }
    println!("  install-dir: {}", data.install_dir);
    println!("  registry: {}", data.registry_path);
    if let Some(manifest) = &data.manifest {
        println!("  manifest: {}", manifest);
    }
    if let Some(project_id) = &data.project_id {
        println!("  project-id: {}", project_id);
    }
    println!("  warnings: {}", data.warnings);
    println!("  errors: {}", data.errors);
    println!("  repaired: {}", data.repaired);
    println!();
    for finding in &data.findings {
        println!(
            "- [{:?}/{:?}] {}: {}",
            finding.severity, finding.status, finding.code, finding.message
        );
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

fn doctor_command(manifest: Option<PathBuf>, repair: bool) -> Result<DoctorReport> {
    doctor(repair, manifest)
}
