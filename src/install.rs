use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;

use crate::project::slugify;

const INSTALL_METADATA_FILE: &str = ".petiglyph-install.json";
const CLI_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FontPlatform {
    Linux,
    Macos,
    Windows,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum UninstallOutcome {
    Removed,
    AlreadyAbsent,
}

#[derive(Debug, Serialize, Deserialize)]
struct InstalledFontMetadata {
    manifest_path: String,
    font_name: String,
    installed_ttf: String,
    version: String,
}

#[derive(Debug, Clone)]
pub(crate) struct FontInstallResult {
    pub(crate) platform: FontPlatform,
    pub(crate) install_dir: PathBuf,
    pub(crate) install_path: PathBuf,
    pub(crate) replaced_previous_ttf_count: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct FontUninstallResult {
    pub(crate) platform: FontPlatform,
    pub(crate) install_dir: PathBuf,
    pub(crate) outcome: UninstallOutcome,
    pub(crate) removed_ttf_count: usize,
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

pub(crate) fn install_dir_for_manifest(manifest_path: &Path) -> Result<PathBuf> {
    let font_root = user_font_root()?;
    install_dir_for_project(manifest_path, &font_root)
}

pub(crate) fn expected_install_ttf_path(manifest_path: &Path, font_name: &str) -> Result<PathBuf> {
    let install_dir = install_dir_for_manifest(manifest_path)?;
    Ok(install_dir.join(font_file_name(font_name)))
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

pub(crate) fn install_built_font(
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

pub(crate) fn uninstall_project_font(manifest_path: &Path) -> Result<FontUninstallResult> {
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
