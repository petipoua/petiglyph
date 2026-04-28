use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;

use crate::project::{read_manifest, slugify};

const INSTALL_METADATA_PREFIX: &str = ".petiglyph-install-";
const INSTALL_METADATA_SUFFIX: &str = ".json";
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FontInstallNameMode {
    Plain,
    ProjectPrefixed,
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

fn project_slug_for_manifest(manifest_path: &Path) -> Result<String> {
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
    Ok(slugify(project_name))
}

fn install_dir_for_project(font_root: &Path) -> PathBuf {
    font_root.join("petiglyph")
}

pub(crate) fn install_dir_for_manifest(_manifest_path: &Path) -> Result<PathBuf> {
    let font_root = user_font_root()?;
    Ok(install_dir_for_project(&font_root))
}

pub(crate) fn effective_font_name(
    manifest_path: &Path,
    font_name: &str,
    mode: FontInstallNameMode,
) -> Result<String> {
    match mode {
        FontInstallNameMode::Plain => Ok(font_name.to_string()),
        FontInstallNameMode::ProjectPrefixed => {
            let project_slug = project_slug_for_manifest(manifest_path)?;
            let trimmed = font_name.trim();
            if trimmed.is_empty() {
                Ok(project_slug)
            } else {
                Ok(format!("{project_slug}-{trimmed}"))
            }
        }
    }
}

pub(crate) fn expected_install_ttf_path_for_mode(
    manifest_path: &Path,
    font_name: &str,
    mode: FontInstallNameMode,
) -> Result<PathBuf> {
    let install_dir = install_dir_for_manifest(manifest_path)?;
    let install_font_name = effective_font_name(manifest_path, font_name, mode)?;
    Ok(install_dir.join(font_file_name(&install_font_name)))
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

fn metadata_file_name(font_name: &str) -> String {
    let key = slugify(font_name);
    let key = if key.is_empty() {
        "petiglyph".to_string()
    } else {
        key
    };
    format!("{INSTALL_METADATA_PREFIX}{key}{INSTALL_METADATA_SUFFIX}")
}

fn metadata_path_for_font(install_dir: &Path, font_name: &str) -> PathBuf {
    install_dir.join(metadata_file_name(font_name))
}

fn metadata_matches_installed_ttf(
    metadata: &InstalledFontMetadata,
    installed_ttf: &Path,
    installed_ttf_canonical: Option<&Path>,
) -> bool {
    let metadata_path = Path::new(&metadata.installed_ttf);
    if metadata_path == installed_ttf {
        return true;
    }

    let Some(installed_ttf_canonical) = installed_ttf_canonical else {
        return false;
    };

    metadata_path
        .canonicalize()
        .ok()
        .as_deref()
        .is_some_and(|path| path == installed_ttf_canonical)
}

fn matching_metadata_paths_for_installed_ttf(
    install_dir: &Path,
    installed_ttf: &Path,
) -> Result<Vec<PathBuf>> {
    let installed_ttf_canonical = installed_ttf.canonicalize().ok();
    let mut matches = Vec::new();

    for entry in fs::read_dir(install_dir)
        .with_context(|| format!("failed to read {}", install_dir.display()))?
    {
        let entry =
            entry.with_context(|| format!("failed to read entry in {}", install_dir.display()))?;
        let path = entry.path();
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        let is_metadata = file_name.starts_with(INSTALL_METADATA_PREFIX)
            && file_name.ends_with(INSTALL_METADATA_SUFFIX);
        if !path.is_file() || !is_metadata {
            continue;
        }

        let Ok(raw) = fs::read_to_string(&path) else {
            continue;
        };
        let Ok(metadata) = serde_json::from_str::<InstalledFontMetadata>(&raw) else {
            continue;
        };
        if metadata_matches_installed_ttf(
            &metadata,
            installed_ttf,
            installed_ttf_canonical.as_deref(),
        ) {
            matches.push(path);
        }
    }

    Ok(matches)
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
    let install_dir = install_dir_for_project(&font_root);
    fs::create_dir_all(&install_dir)
        .with_context(|| format!("failed to create {}", install_dir.display()))?;

    let file_name = font_file_name(font_name);
    let install_path = install_dir.join(file_name);
    let replaced_previous_ttf_count = if install_path.exists() {
        if !install_path.is_file() {
            bail!(
                "blocked install for {}: expected file path but found non-file",
                install_path.display()
            );
        }
        1
    } else {
        0
    };
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
    let metadata_path = metadata_path_for_font(&install_dir, font_name);
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
    let install_dir = install_dir_for_project(&font_root);
    let manifest = read_manifest(manifest_path)?;

    if !install_dir.exists() {
        return Ok(FontUninstallResult {
            platform,
            install_dir,
            outcome: UninstallOutcome::AlreadyAbsent,
            removed_ttf_count: 0,
        });
    }

    let plain = effective_font_name(
        manifest_path,
        &manifest.font_name,
        FontInstallNameMode::Plain,
    )?;
    let prefixed = effective_font_name(
        manifest_path,
        &manifest.font_name,
        FontInstallNameMode::ProjectPrefixed,
    )?;
    let mut names = BTreeSet::new();
    names.insert(plain);
    names.insert(prefixed);

    let mut removed_ttf_count = 0usize;
    for name in &names {
        let ttf_path = install_dir.join(font_file_name(name));
        if ttf_path.is_file() {
            fs::remove_file(&ttf_path)
                .with_context(|| format!("failed to remove {}", ttf_path.display()))?;
            removed_ttf_count += 1;
        } else if ttf_path.exists() {
            bail!(
                "blocked uninstall for {}: expected file but found non-file at {}",
                install_dir.display(),
                ttf_path.display()
            );
        }

        let metadata_path = metadata_path_for_font(&install_dir, name);
        if metadata_path.is_file() {
            fs::remove_file(&metadata_path)
                .with_context(|| format!("failed to remove {}", metadata_path.display()))?;
        } else if metadata_path.exists() {
            bail!(
                "blocked uninstall for {}: expected file but found non-file at {}",
                install_dir.display(),
                metadata_path.display()
            );
        }
    }

    let outcome = if removed_ttf_count == 0 {
        UninstallOutcome::AlreadyAbsent
    } else {
        refresh_font_cache(&font_root, platform)?;
        UninstallOutcome::Removed
    };

    if fs::read_dir(&install_dir)
        .with_context(|| format!("failed to read {}", install_dir.display()))?
        .next()
        .is_none()
    {
        fs::remove_dir(&install_dir)
            .with_context(|| format!("failed to remove {}", install_dir.display()))?;
    }

    Ok(FontUninstallResult {
        platform,
        install_dir,
        outcome,
        removed_ttf_count,
    })
}

pub(crate) fn uninstall_installed_font_file(installed_ttf: &Path) -> Result<FontUninstallResult> {
    let platform = current_platform()?;
    let font_root = user_font_root()?;
    let install_dir = install_dir_for_project(&font_root);

    if !install_dir.exists() {
        return Ok(FontUninstallResult {
            platform,
            install_dir,
            outcome: UninstallOutcome::AlreadyAbsent,
            removed_ttf_count: 0,
        });
    }

    if installed_ttf.exists() && !installed_ttf.is_file() {
        bail!(
            "blocked uninstall for {}: expected file but found non-file at {}",
            install_dir.display(),
            installed_ttf.display()
        );
    }

    let install_dir_canonical = install_dir
        .canonicalize()
        .with_context(|| format!("failed to resolve {}", install_dir.display()))?;
    if let Ok(installed_ttf_canonical) = installed_ttf.canonicalize()
        && !installed_ttf_canonical.starts_with(&install_dir_canonical)
    {
        bail!(
            "blocked uninstall outside {}: {}",
            install_dir.display(),
            installed_ttf.display()
        );
    }

    let metadata_paths = matching_metadata_paths_for_installed_ttf(&install_dir, installed_ttf)?;
    let mut removed_ttf_count = 0usize;
    if installed_ttf.is_file() {
        fs::remove_file(installed_ttf)
            .with_context(|| format!("failed to remove {}", installed_ttf.display()))?;
        removed_ttf_count = 1;
    }

    for metadata_path in metadata_paths {
        if metadata_path.is_file() {
            fs::remove_file(&metadata_path)
                .with_context(|| format!("failed to remove {}", metadata_path.display()))?;
        } else if metadata_path.exists() {
            bail!(
                "blocked uninstall for {}: expected file but found non-file at {}",
                install_dir.display(),
                metadata_path.display()
            );
        }
    }

    let outcome = if removed_ttf_count == 0 {
        UninstallOutcome::AlreadyAbsent
    } else {
        refresh_font_cache(&font_root, platform)?;
        UninstallOutcome::Removed
    };

    if fs::read_dir(&install_dir)
        .with_context(|| format!("failed to read {}", install_dir.display()))?
        .next()
        .is_none()
    {
        fs::remove_dir(&install_dir)
            .with_context(|| format!("failed to remove {}", install_dir.display()))?;
    }

    Ok(FontUninstallResult {
        platform,
        install_dir,
        outcome,
        removed_ttf_count,
    })
}
