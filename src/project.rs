use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Manifest {
    pub(crate) input_dir: String,
    pub(crate) out_dir: String,
    pub(crate) font_name: String,
    pub(crate) glyph_size: u32,
    pub(crate) threshold: u8,
    pub(crate) codepoint_start: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) threshold_overrides: BTreeMap<String, u8>,
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
pub(crate) struct RuntimeConfig {
    pub(crate) input_dir: PathBuf,
    pub(crate) out_dir: PathBuf,
    pub(crate) font_name: String,
    pub(crate) glyph_size: u32,
    pub(crate) base_threshold: u8,
    pub(crate) threshold_overrides: BTreeMap<String, u8>,
    pub(crate) codepoint_start: u32,
}

pub(crate) fn format_codepoint(codepoint: u32) -> String {
    format!("U+{:04X}", codepoint)
}

pub(crate) fn manifest_path_from_option(manifest: Option<PathBuf>) -> Result<PathBuf> {
    match manifest {
        Some(path) => Ok(path),
        None => {
            let current_dir = env::current_dir().context("failed to read current directory")?;
            auto_detect_manifest_path(&current_dir)
        }
    }
}

pub(crate) fn auto_detect_manifest_path(current_dir: &Path) -> Result<PathBuf> {
    let manifest_path = current_dir.join("petiglyph.toml");
    if manifest_path.is_file() {
        return Ok(manifest_path);
    }

    let mut nested_manifests = Vec::new();
    for entry in fs::read_dir(current_dir).with_context(|| {
        format!(
            "failed while searching for petiglyph.toml in {}",
            current_dir.display()
        )
    })? {
        let entry = entry.with_context(|| {
            format!(
                "failed while searching for petiglyph.toml in {}",
                current_dir.display()
            )
        })?;
        let file_type = entry.file_type().with_context(|| {
            format!(
                "failed while searching for petiglyph.toml in {}",
                current_dir.display()
            )
        })?;
        if !file_type.is_dir() {
            continue;
        }

        let candidate = entry.path().join("petiglyph.toml");
        if candidate.is_file() {
            nested_manifests.push(candidate);
        }
    }

    match nested_manifests.len() {
        1 => Ok(nested_manifests.remove(0)),
        0 => bail!(
            "no petiglyph project detected in {} (run `petiglyph create <name>` or pass --manifest)",
            current_dir.display()
        ),
        _ => bail!(
            "multiple petiglyph projects detected in {} (pass --manifest to choose one)",
            current_dir.display()
        ),
    }
}

pub(crate) fn create_project(project_name: &str, no_launch: bool) -> Result<()> {
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
        crate::tui::tui(manifest_path, None, None, None, None)
    } else {
        println!("non-interactive shell detected; skipping automatic TUI launch");
        Ok(())
    }
}

pub(crate) fn read_manifest(manifest_path: &Path) -> Result<Manifest> {
    let data = fs::read_to_string(manifest_path)
        .with_context(|| format!("failed to read {}", manifest_path.display()))?;
    toml::from_str(&data).with_context(|| format!("failed to parse {}", manifest_path.display()))
}

pub(crate) fn write_manifest(manifest_path: &Path, manifest: &Manifest) -> Result<()> {
    let data = toml::to_string_pretty(manifest).context("failed to serialize manifest")?;
    fs::write(manifest_path, data)
        .with_context(|| format!("failed to write {}", manifest_path.display()))
}

pub(crate) fn load_runtime_config(
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

pub(crate) fn parse_codepoint(value: &str) -> Result<u32> {
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

pub(crate) fn slugify(input: &str) -> String {
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
