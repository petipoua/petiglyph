use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::project::{
    format_codepoint, load_runtime_config, parse_codepoint, read_manifest, slugify,
};

const INSTALL_METADATA_PREFIX: &str = ".petiglyph-install-";
const INSTALL_METADATA_SUFFIX: &str = ".json";
const INSTALL_LOCK_FILE_NAME: &str = ".petiglyph-install.lock";
const UNICODE_REGISTRY_FILE_NAME: &str = ".unicode-registry.json";
const UNICODE_REGISTRY_LOCK_FILE_NAME: &str = ".unicode-registry.lock";
const UNICODE_REGISTRY_VERSION: u32 = 1;
const SUPPLEMENTARY_PUA_START: u32 = 0xF0000;
const SUPPLEMENTARY_PUA_END: u32 = 0x10_FFFF;
const MIN_PROJECT_RANGE_SIZE: u32 = 1;
const FILE_LOCK_TIMEOUT: Duration = Duration::from_secs(15);
const FILE_LOCK_STALE_AFTER: Duration = Duration::from_secs(120);
const FILE_LOCK_RETRY_DELAY: Duration = Duration::from_millis(50);
const CLI_VERSION: &str = env!("CARGO_PKG_VERSION");
const FONTCONFIG_PETIGLYPH_ALIAS_FILE_NAME: &str = "99-petiglyph.conf";

#[derive(Debug)]
struct FileLockGuard {
    path: PathBuf,
}

impl Drop for FileLockGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

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

pub(crate) const DEFAULT_INSTALL_NAME_MODE: FontInstallNameMode =
    FontInstallNameMode::ProjectPrefixed;

#[derive(Debug, Serialize, Deserialize)]
struct InstalledFontMetadata {
    manifest_path: String,
    font_name: String,
    installed_ttf: String,
    version: String,
    #[serde(default)]
    project_id: Option<String>,
    #[serde(default)]
    install_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct UnicodeRegistryRange {
    project_id: String,
    range_start: String,
    range_end: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct UnicodeRegistryFile {
    version: u32,
    #[serde(default)]
    assignments: Vec<UnicodeRegistryRange>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct UnicodeRangeReservation {
    pub(crate) range_start: u32,
    pub(crate) range_end: u32,
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

#[derive(Debug, Clone, Serialize)]
pub(crate) struct SampleCoverageProbe {
    pub(crate) codepoint: String,
    pub(crate) matched_font_file: String,
    pub(crate) matched_in_managed_install_dir: bool,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct SampleCoverageDiagnosis {
    pub(crate) checked_codepoints: usize,
    pub(crate) missing_codepoints: usize,
    pub(crate) probes: Vec<SampleCoverageProbe>,
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

fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn is_valid_unicode_scalar(codepoint: u32) -> bool {
    codepoint <= 0x10_FFFF && !(0xD800..=0xDFFF).contains(&codepoint)
}

fn acquire_file_lock(lock_path: &Path, label: &str) -> Result<FileLockGuard> {
    let started = SystemTime::now();

    loop {
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(lock_path)
        {
            Ok(mut file) => {
                let _ = writeln!(
                    file,
                    "pid={} created_unix_ms={}",
                    std::process::id(),
                    now_millis()
                );
                return Ok(FileLockGuard {
                    path: lock_path.to_path_buf(),
                });
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                let stale = fs::metadata(lock_path)
                    .and_then(|metadata| metadata.modified())
                    .ok()
                    .and_then(|modified| modified.elapsed().ok())
                    .is_some_and(|elapsed| elapsed > FILE_LOCK_STALE_AFTER);

                if stale {
                    let _ = fs::remove_file(lock_path);
                    continue;
                }

                if started.elapsed().unwrap_or_default() >= FILE_LOCK_TIMEOUT {
                    bail!(
                        "timed out waiting for {} lock: {}",
                        label,
                        lock_path.display()
                    );
                }

                thread::sleep(FILE_LOCK_RETRY_DELAY);
            }
            Err(err) => {
                return Err(err).with_context(|| {
                    format!("failed to create {} lock {}", label, lock_path.display())
                });
            }
        }
    }
}

fn write_atomic_string(path: &Path, data: &str) -> Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let temp_path = parent.join(format!(
        ".{}.tmp-{}-{}",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("write"),
        std::process::id(),
        now_millis()
    ));
    fs::write(&temp_path, data)
        .with_context(|| format!("failed to write {}", temp_path.display()))?;
    fs::rename(&temp_path, path)
        .with_context(|| format!("failed to replace {}", path.display()))?;
    Ok(())
}

fn write_atomic_bytes(path: &Path, data: &[u8]) -> Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let temp_path = parent.join(format!(
        ".{}.tmp-{}-{}",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("write"),
        std::process::id(),
        now_millis()
    ));
    fs::write(&temp_path, data)
        .with_context(|| format!("failed to write {}", temp_path.display()))?;
    fs::rename(&temp_path, path)
        .with_context(|| format!("failed to replace {}", path.display()))?;
    Ok(())
}

fn install_key(project_id: &str, font_name: &str) -> String {
    let mut project = slugify(project_id);
    if project.is_empty() {
        project = "project".to_string();
    }
    if project.len() > 12 {
        project.truncate(12);
    }

    let mut font = slugify(font_name);
    if font.is_empty() {
        font = "petiglyph".to_string();
    }
    let font_hash = fnv1a64(font_name.as_bytes()) as u32;

    format!("{font}_{project}_{font_hash:08x}")
}

fn version_slug() -> String {
    let mut out = String::new();
    let mut prev_sep = false;
    for ch in CLI_VERSION.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_sep = false;
            continue;
        }
        if !prev_sep {
            out.push('_');
            prev_sep = true;
        }
    }
    out.trim_matches('_').to_string()
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn immutable_ttf_file_name(project_id: &str, font_name: &str, ttf_bytes: &[u8]) -> String {
    let key = install_key(project_id, font_name);
    let version = version_slug();
    let hash = fnv1a64(ttf_bytes);
    format!("{key}.v{version}.{hash:016x}.ttf")
}

fn parse_registry_range(range: &UnicodeRegistryRange) -> Result<(u32, u32)> {
    let start = parse_codepoint(&range.range_start)
        .with_context(|| format!("invalid registry range_start for {}", range.project_id))?;
    let end = parse_codepoint(&range.range_end)
        .with_context(|| format!("invalid registry range_end for {}", range.project_id))?;
    if !is_valid_unicode_scalar(start) || !is_valid_unicode_scalar(end) || start > end {
        bail!(
            "invalid Unicode registry range for {}: {}..{}",
            range.project_id,
            range.range_start,
            range.range_end
        );
    }
    if start < SUPPLEMENTARY_PUA_START || end > SUPPLEMENTARY_PUA_END {
        bail!(
            "registry range for {} is outside supplementary private use area: {}..{}",
            range.project_id,
            range.range_start,
            range.range_end
        );
    }
    Ok((start, end))
}

fn load_unicode_registry(path: &Path) -> Result<UnicodeRegistryFile> {
    if !path.exists() {
        return Ok(UnicodeRegistryFile {
            version: UNICODE_REGISTRY_VERSION,
            assignments: Vec::new(),
        });
    }

    let raw =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    let registry: UnicodeRegistryFile = serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    if registry.version != UNICODE_REGISTRY_VERSION {
        bail!(
            "unsupported Unicode registry version in {}: expected {}, got {}",
            path.display(),
            UNICODE_REGISTRY_VERSION,
            registry.version
        );
    }

    let mut seen = BTreeSet::new();
    let mut ranges = Vec::with_capacity(registry.assignments.len());
    for assignment in &registry.assignments {
        if assignment.project_id.trim().is_empty() {
            bail!(
                "Unicode registry contains an empty project_id in {}",
                path.display()
            );
        }
        if !seen.insert(assignment.project_id.clone()) {
            bail!(
                "Unicode registry contains duplicate project_id {} in {}",
                assignment.project_id,
                path.display()
            );
        }
        ranges.push((
            assignment.project_id.clone(),
            parse_registry_range(assignment)?,
        ));
    }

    ranges.sort_by_key(|(_, (start, _))| *start);
    for pair in ranges.windows(2) {
        let (_, (_, left_end)) = &pair[0];
        let (_, (right_start, _)) = &pair[1];
        if right_start <= left_end {
            bail!(
                "Unicode registry contains overlapping ranges in {}",
                path.display()
            );
        }
    }

    Ok(registry)
}

fn save_unicode_registry(path: &Path, registry: &UnicodeRegistryFile) -> Result<()> {
    let raw =
        serde_json::to_string_pretty(registry).context("failed to serialize Unicode registry")?;
    write_atomic_string(path, &raw)
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

fn install_lock_path(font_root: &Path) -> PathBuf {
    font_root.join(INSTALL_LOCK_FILE_NAME)
}

pub(crate) fn install_dir_for_manifest(_manifest_path: &Path) -> Result<PathBuf> {
    let font_root = user_font_root()?;
    Ok(install_dir_for_project(&font_root))
}

pub(crate) fn managed_install_dir() -> Result<PathBuf> {
    let font_root = user_font_root()?;
    Ok(install_dir_for_project(&font_root))
}

fn user_fontconfig_conf_dir() -> Result<PathBuf> {
    let home = home_dir()?;
    Ok(home.join(".config/fontconfig/conf.d"))
}

fn petiglyph_fontconfig_alias_path() -> Result<PathBuf> {
    Ok(user_fontconfig_conf_dir()?.join(FONTCONFIG_PETIGLYPH_ALIAS_FILE_NAME))
}

fn xml_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(ch),
        }
    }
    out
}

fn range_overlaps(a_start: u32, a_end: u32, b_start: u32, b_end: u32) -> bool {
    a_start <= b_end && b_start <= a_end
}

fn can_use_range(
    start: u32,
    end: u32,
    project_id: &str,
    registry: &UnicodeRegistryFile,
) -> Result<bool> {
    for assignment in &registry.assignments {
        if assignment.project_id == project_id {
            continue;
        }
        let (other_start, other_end) = parse_registry_range(assignment)?;
        if range_overlaps(start, end, other_start, other_end) {
            return Ok(false);
        }
    }
    Ok(true)
}

fn range_len(start: u32, end: u32) -> u32 {
    end - start + 1
}

fn clamp_to_pua_start(value: u32) -> u32 {
    value
        .max(SUPPLEMENTARY_PUA_START)
        .min(SUPPLEMENTARY_PUA_END)
}

fn compute_range_span(
    requested_codepoints: usize,
    range_start: u32,
    locked_codepoints: &BTreeSet<u32>,
) -> Result<u32> {
    let requested = u32::try_from(requested_codepoints)
        .context("glyph count is too large for Unicode range allocation")?;
    let requested = requested.max(1);
    let mut span = requested.max(MIN_PROJECT_RANGE_SIZE);
    if let Some(max_locked) = locked_codepoints.iter().max().copied()
        && max_locked >= range_start
    {
        span = span.max(max_locked - range_start + 1);
    }
    Ok(span)
}

fn find_first_fit(
    start_hint: u32,
    span: u32,
    project_id: &str,
    registry: &UnicodeRegistryFile,
) -> Result<Option<(u32, u32)>> {
    let latest_start = SUPPLEMENTARY_PUA_END
        .checked_sub(span.saturating_sub(1))
        .ok_or_else(|| anyhow::anyhow!("Unicode range span is too large for supplementary PUA"))?;

    let mut occupied = Vec::new();
    for assignment in &registry.assignments {
        if assignment.project_id == project_id {
            continue;
        }
        occupied.push(parse_registry_range(assignment)?);
    }
    occupied.sort_by_key(|(start, _)| *start);

    let try_from = |initial: u32, upper_bound: u32| -> Option<(u32, u32)> {
        if initial > upper_bound {
            return None;
        }
        let mut cursor = initial;
        while cursor <= upper_bound {
            let end = cursor + span - 1;
            let mut bumped = false;
            for (other_start, other_end) in &occupied {
                if *other_end < cursor || *other_start > end {
                    continue;
                }
                cursor = other_end.saturating_add(1);
                bumped = true;
                break;
            }
            if !bumped {
                return Some((cursor, end));
            }
        }
        None
    };

    let clamped_hint = clamp_to_pua_start(start_hint);
    if let Some(range) = try_from(clamped_hint, latest_start) {
        return Ok(Some(range));
    }
    if let Some(range) = try_from(SUPPLEMENTARY_PUA_START, clamped_hint.saturating_sub(1)) {
        return Ok(Some(range));
    }
    Ok(None)
}

fn find_first_fit_containing_locked(
    preferred_start: u32,
    span: u32,
    project_id: &str,
    registry: &UnicodeRegistryFile,
    locked_codepoints: &BTreeSet<u32>,
) -> Result<Option<(u32, u32)>> {
    if locked_codepoints.is_empty() {
        return find_first_fit(preferred_start, span, project_id, registry);
    }

    let Some(min_locked) = locked_codepoints.iter().min().copied() else {
        return find_first_fit(preferred_start, span, project_id, registry);
    };
    let Some(max_locked) = locked_codepoints.iter().max().copied() else {
        return find_first_fit(preferred_start, span, project_id, registry);
    };

    if max_locked < min_locked {
        return Ok(None);
    }

    let locked_span = max_locked - min_locked + 1;
    if locked_span > span {
        bail!(
            "locked codepoint span U+{:04X}..U+{:04X} cannot fit requested range span {}",
            min_locked,
            max_locked,
            span
        );
    }

    let latest_start = SUPPLEMENTARY_PUA_END
        .checked_sub(span.saturating_sub(1))
        .ok_or_else(|| anyhow::anyhow!("Unicode range span is too large for supplementary PUA"))?;
    let min_start = max_locked
        .saturating_sub(span.saturating_sub(1))
        .max(SUPPLEMENTARY_PUA_START);
    let max_start = min_locked.min(latest_start);
    if min_start > max_start {
        return Ok(None);
    }

    let preferred = preferred_start.clamp(min_start, max_start);
    let mut starts = Vec::with_capacity((max_start - min_start + 1) as usize);
    starts.push(preferred);
    for delta in 1..=(max_start - min_start) {
        if let Some(left) = preferred.checked_sub(delta)
            && left >= min_start
        {
            starts.push(left);
        }
        if let Some(right) = preferred.checked_add(delta)
            && right <= max_start
        {
            starts.push(right);
        }
    }

    for start in starts {
        let end = start + span - 1;
        if can_use_range(start, end, project_id, registry)? {
            return Ok(Some((start, end)));
        }
    }

    Ok(None)
}

pub(crate) fn reserve_project_unicode_range(
    registry_root_override: Option<&Path>,
    project_id: &str,
    requested_start: u32,
    required_codepoints: usize,
    locked_codepoints: &BTreeSet<u32>,
) -> Result<UnicodeRangeReservation> {
    if project_id.trim().is_empty() {
        bail!("project_id cannot be empty when reserving a Unicode range");
    }
    if !is_valid_unicode_scalar(requested_start) {
        bail!(
            "requested codepoint_start is not a valid Unicode scalar: U+{:04X}",
            requested_start
        );
    }
    if required_codepoints == 0 {
        bail!("cannot reserve a Unicode range for zero glyphs");
    }

    for codepoint in locked_codepoints {
        if !is_valid_unicode_scalar(*codepoint) {
            bail!("glyph lock contains an invalid Unicode scalar: U+{codepoint:04X}");
        }
        if *codepoint < SUPPLEMENTARY_PUA_START || *codepoint > SUPPLEMENTARY_PUA_END {
            bail!(
                "glyph lock codepoint U+{codepoint:04X} is outside supplementary private use area U+F0000..U+10FFFF"
            );
        }
    }

    let font_root = match registry_root_override {
        Some(path) => path.to_path_buf(),
        None => user_font_root()?,
    };
    let install_dir = install_dir_for_project(&font_root);
    fs::create_dir_all(&install_dir)
        .with_context(|| format!("failed to create {}", install_dir.display()))?;

    let lock_path = install_dir.join(UNICODE_REGISTRY_LOCK_FILE_NAME);
    let _guard = acquire_file_lock(&lock_path, "Unicode registry")?;

    let registry_path = install_dir.join(UNICODE_REGISTRY_FILE_NAME);
    let mut registry = load_unicode_registry(&registry_path)?;

    let mut project_assignment_idx = None;
    for (idx, assignment) in registry.assignments.iter().enumerate() {
        if assignment.project_id == project_id {
            project_assignment_idx = Some(idx);
            break;
        }
        let (other_start, other_end) = parse_registry_range(assignment)?;
        if locked_codepoints
            .iter()
            .any(|cp| *cp >= other_start && *cp <= other_end)
        {
            bail!(
                "glyph lock/codepoint conflict: project {} uses codepoints owned by {} ({}..{})",
                project_id,
                assignment.project_id,
                assignment.range_start,
                assignment.range_end
            );
        }
    }

    let (range_start, range_end, changed) = if let Some(idx) = project_assignment_idx {
        let assignment = &registry.assignments[idx];
        let (start, mut end) = parse_registry_range(assignment)?;

        if locked_codepoints.iter().any(|cp| *cp < start || *cp > end) {
            bail!(
                "glyph lock/codepoint conflict: project {} has codepoints outside its owned registry range {}..{}",
                project_id,
                assignment.range_start,
                assignment.range_end
            );
        }

        let required_span = compute_range_span(required_codepoints, start, locked_codepoints)?;
        let current_span = range_len(start, end);

        if required_span > current_span {
            let expanded_end = start
                .checked_add(required_span - 1)
                .ok_or_else(|| anyhow::anyhow!("Unicode range expansion overflow"))?;
            if expanded_end <= SUPPLEMENTARY_PUA_END
                && can_use_range(start, expanded_end, project_id, &registry)?
            {
                end = expanded_end;
                (start, end, true)
            } else {
                let Some((relocated_start, relocated_end)) = find_first_fit_containing_locked(
                    start,
                    required_span,
                    project_id,
                    &registry,
                    locked_codepoints,
                )?
                else {
                    bail!(
                        "Unicode range conflict: cannot reserve {} codepoints for project {} without colliding with another project",
                        required_span,
                        project_id
                    );
                };
                (relocated_start, relocated_end, true)
            }
        } else {
            (start, end, false)
        }
    } else {
        let start_hint = locked_codepoints
            .iter()
            .min()
            .copied()
            .unwrap_or(requested_start);
        let start_hint = clamp_to_pua_start(start_hint);
        let required_span = compute_range_span(required_codepoints, start_hint, locked_codepoints)?;

        let Some((start, end)) = find_first_fit(start_hint, required_span, project_id, &registry)?
        else {
            bail!(
                "Unicode range allocation failed: no disjoint supplementary private use range available for project {}",
                project_id
            );
        };
        registry.assignments.push(UnicodeRegistryRange {
            project_id: project_id.to_string(),
            range_start: format_codepoint(start),
            range_end: format_codepoint(end),
        });
        (start, end, true)
    };

    if changed {
        for assignment in &mut registry.assignments {
            if assignment.project_id == project_id {
                assignment.range_start = format_codepoint(range_start);
                assignment.range_end = format_codepoint(range_end);
            }
        }
        save_unicode_registry(&registry_path, &registry)?;
    }

    Ok(UnicodeRangeReservation {
        range_start,
        range_end,
    })
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

pub(crate) fn installed_ttf_candidates_for_manifest_font(
    manifest_path: &Path,
    font_name: &str,
) -> Result<Vec<PathBuf>> {
    let install_dir = install_dir_for_manifest(manifest_path)?;
    if !install_dir.exists() {
        return Ok(Vec::new());
    }

    let manifest_canonical = manifest_path.canonicalize().ok();
    let default_name = effective_font_name(manifest_path, font_name, DEFAULT_INSTALL_NAME_MODE)?;
    let plain_name = effective_font_name(manifest_path, font_name, FontInstallNameMode::Plain)?;
    let mut names = BTreeSet::new();
    names.insert(default_name);
    names.insert(plain_name);

    let mut out = Vec::new();
    for entry in fs::read_dir(&install_dir)
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

        let Ok(raw) = fs::read_to_string(&path) else {
            continue;
        };
        let Ok(metadata) = serde_json::from_str::<InstalledFontMetadata>(&raw) else {
            continue;
        };

        let manifest_matches = manifest_canonical
            .as_deref()
            .zip(Path::new(&metadata.manifest_path).canonicalize().ok())
            .is_some_and(|(left, right)| left == right)
            || metadata.manifest_path == manifest_path.display().to_string();
        if !manifest_matches {
            continue;
        }
        if !names.contains(&metadata.font_name) {
            continue;
        }
        out.push(PathBuf::from(metadata.installed_ttf));
    }

    Ok(out)
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

fn metadata_file_name_for_project(project_id: &str, font_name: &str) -> String {
    let key = install_key(project_id, font_name);
    format!("{INSTALL_METADATA_PREFIX}{key}{INSTALL_METADATA_SUFFIX}")
}

fn metadata_path_for_font(install_dir: &Path, font_name: &str) -> PathBuf {
    install_dir.join(metadata_file_name(font_name))
}

fn metadata_path_for_project_font(
    install_dir: &Path,
    project_id: &str,
    font_name: &str,
) -> PathBuf {
    install_dir.join(metadata_file_name_for_project(project_id, font_name))
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

fn read_managed_font_families(install_dir: &Path) -> Result<Vec<String>> {
    if !install_dir.exists() {
        return Ok(Vec::new());
    }

    let mut by_install_path: BTreeMap<String, String> = BTreeMap::new();
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
        let Ok(raw) = fs::read_to_string(&path) else {
            continue;
        };
        let Ok(metadata) = serde_json::from_str::<InstalledFontMetadata>(&raw) else {
            continue;
        };
        if metadata.font_name.trim().is_empty() || metadata.installed_ttf.trim().is_empty() {
            continue;
        }
        by_install_path.insert(metadata.installed_ttf, metadata.font_name);
    }

    let mut out = Vec::with_capacity(by_install_path.len());
    for (_, family) in by_install_path {
        out.push(family);
    }
    Ok(out)
}

fn update_fontconfig_petiglyph_alias(install_dir: &Path, platform: FontPlatform) -> Result<()> {
    if !matches!(platform, FontPlatform::Linux) {
        return Ok(());
    }

    let alias_path = petiglyph_fontconfig_alias_path()?;
    let families = read_managed_font_families(install_dir)?;

    if families.is_empty() {
        if alias_path.is_file() {
            fs::remove_file(&alias_path)
                .with_context(|| format!("failed to remove {}", alias_path.display()))?;
        }
        return Ok(());
    }

    let conf_dir = alias_path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(conf_dir)
        .with_context(|| format!("failed to create {}", conf_dir.display()))?;

    let mut xml = String::from(
        "<?xml version=\"1.0\"?>\n<!DOCTYPE fontconfig SYSTEM \"fonts.dtd\">\n<fontconfig>\n",
    );
    for alias in ["Petiglyph", "petiglyph"] {
        xml.push_str("  <alias binding=\"same\">\n");
        xml.push_str(&format!("    <family>{}</family>\n", xml_escape(alias)));
        xml.push_str("    <prefer>\n");
        for family in &families {
            xml.push_str(&format!("      <family>{}</family>\n", xml_escape(family)));
        }
        xml.push_str("    </prefer>\n");
        xml.push_str("  </alias>\n");
    }
    xml.push_str("</fontconfig>\n");

    write_atomic_string(&alias_path, &xml)
        .with_context(|| format!("failed to write {}", alias_path.display()))
}

fn parse_sample_codepoints(sample: &str) -> Vec<u32> {
    sample
        .chars()
        .filter_map(|ch| {
            let cp = ch as u32;
            (!ch.is_whitespace()).then_some(cp)
        })
        .collect()
}

fn fc_match_file_for_codepoint(codepoint: u32) -> Result<String> {
    let pattern = format!(":charset={codepoint:x}");
    let output = ProcessCommand::new("fc-match")
        .arg(pattern)
        .arg("-f")
        .arg("%{file}\n")
        .output()
        .context("failed to run fc-match for sample coverage probe")?;
    if !output.status.success() {
        bail!("fc-match failed while probing sample coverage");
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub(crate) fn diagnose_sample_coverage(sample: &str) -> Result<Option<SampleCoverageDiagnosis>> {
    let platform = current_platform()?;
    if !matches!(platform, FontPlatform::Linux) {
        return Ok(None);
    }

    let install_dir = managed_install_dir()?;
    let install_dir_display = install_dir.display().to_string();
    let mut probes = Vec::new();
    let mut missing = 0usize;
    let codepoints = parse_sample_codepoints(sample);

    for codepoint in codepoints {
        let matched_file = fc_match_file_for_codepoint(codepoint)?;
        let matched_in_managed = matched_file.starts_with(&install_dir_display);
        if !matched_in_managed {
            missing += 1;
        }
        probes.push(SampleCoverageProbe {
            codepoint: format_codepoint(codepoint),
            matched_font_file: matched_file,
            matched_in_managed_install_dir: matched_in_managed,
        });
    }

    Ok(Some(SampleCoverageDiagnosis {
        checked_codepoints: probes.len(),
        missing_codepoints: missing,
        probes,
    }))
}

pub(crate) fn install_built_font(
    manifest_path: &Path,
    font_name: &str,
    project_id: &str,
    built_ttf: &Path,
    glyph_count: usize,
) -> Result<FontInstallResult> {
    if glyph_count == 0 {
        bail!("cannot install a font with zero glyphs");
    }
    if project_id.trim().is_empty() {
        bail!("project_id cannot be empty during install");
    }

    let platform = current_platform()?;
    let font_root = user_font_root()?;
    let install_dir = install_dir_for_project(&font_root);
    fs::create_dir_all(&install_dir)
        .with_context(|| format!("failed to create {}", install_dir.display()))?;
    let _guard = acquire_file_lock(&install_lock_path(&font_root), "font install metadata")?;

    let built_bytes =
        fs::read(built_ttf).with_context(|| format!("failed to read {}", built_ttf.display()))?;
    let install_path =
        install_dir.join(immutable_ttf_file_name(project_id, font_name, &built_bytes));
    if install_path.exists() {
        if !install_path.is_file() {
            bail!(
                "blocked install for {}: expected file path but found non-file",
                install_path.display()
            );
        }
        let existing = fs::read(&install_path)
            .with_context(|| format!("failed to read {}", install_path.display()))?;
        if existing != built_bytes {
            bail!(
                "hash collision while installing immutable artifact at {}",
                install_path.display()
            );
        }
    } else {
        write_atomic_bytes(&install_path, &built_bytes)
            .with_context(|| format!("failed to write {}", install_path.display()))?;
    }

    let manifest_canonical = manifest_path
        .canonicalize()
        .with_context(|| format!("failed to resolve {}", manifest_path.display()))?;

    let metadata_path = metadata_path_for_project_font(&install_dir, project_id, font_name);
    let previous_installed_ttf = if metadata_path.is_file() {
        let raw = fs::read_to_string(&metadata_path)
            .with_context(|| format!("failed to read {}", metadata_path.display()))?;
        let metadata: InstalledFontMetadata = serde_json::from_str(&raw)
            .with_context(|| format!("failed to parse {}", metadata_path.display()))?;
        Some(PathBuf::from(metadata.installed_ttf))
    } else {
        None
    };

    let install_identity = install_key(project_id, font_name);
    let metadata = InstalledFontMetadata {
        manifest_path: manifest_canonical.display().to_string(),
        font_name: font_name.to_string(),
        installed_ttf: install_path.display().to_string(),
        version: CLI_VERSION.to_string(),
        project_id: Some(project_id.to_string()),
        install_key: Some(install_identity),
    };
    let metadata_json =
        serde_json::to_string_pretty(&metadata).context("failed to serialize install metadata")?;
    write_atomic_string(&metadata_path, &metadata_json)
        .with_context(|| format!("failed to write {}", metadata_path.display()))?;

    let mut replaced_previous_ttf_count = 0usize;
    if let Some(previous) = previous_installed_ttf {
        if previous != install_path && previous.is_file() {
            fs::remove_file(&previous)
                .with_context(|| format!("failed to remove {}", previous.display()))?;
            replaced_previous_ttf_count += 1;
        }
    }

    let legacy_path = install_dir.join(font_file_name(font_name));
    if legacy_path != install_path && legacy_path.is_file() {
        fs::remove_file(&legacy_path)
            .with_context(|| format!("failed to remove {}", legacy_path.display()))?;
        replaced_previous_ttf_count += 1;
    }

    let legacy_metadata = metadata_path_for_font(&install_dir, font_name);
    if legacy_metadata != metadata_path && legacy_metadata.is_file() {
        fs::remove_file(&legacy_metadata)
            .with_context(|| format!("failed to remove {}", legacy_metadata.display()))?;
    }

    update_fontconfig_petiglyph_alias(&install_dir, platform)?;
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
    let _guard = acquire_file_lock(&install_lock_path(&font_root), "font install metadata")?;
    let install_dir = install_dir_for_project(&font_root);
    let manifest = read_manifest(manifest_path)?;
    let config = load_runtime_config(manifest_path, None, None, None, None, None)?;
    let manifest_canonical = manifest_path
        .canonicalize()
        .with_context(|| format!("failed to resolve {}", manifest_path.display()))?;

    if !install_dir.exists() {
        update_fontconfig_petiglyph_alias(&install_dir, platform)?;
        return Ok(FontUninstallResult {
            platform,
            install_dir,
            outcome: UninstallOutcome::AlreadyAbsent,
            removed_ttf_count: 0,
        });
    }

    let default_name = effective_font_name(
        manifest_path,
        &manifest.font_name,
        DEFAULT_INSTALL_NAME_MODE,
    )?;

    // Keep removing the legacy plain install candidate if it differs, to avoid stale leftovers
    // after the install naming policy switched to project-scoped mode.
    let plain = effective_font_name(
        manifest_path,
        &manifest.font_name,
        FontInstallNameMode::Plain,
    )?;

    let mut names = BTreeSet::new();
    names.insert(default_name);
    names.insert(plain);

    let mut ttf_paths_to_remove = BTreeSet::new();
    let mut metadata_paths_to_remove = BTreeSet::new();
    let metadata_entries: Vec<_> = fs::read_dir(&install_dir)
        .with_context(|| format!("failed to read {}", install_dir.display()))?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .collect();

    for metadata_path in metadata_entries {
        let file_name = metadata_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        let is_metadata = file_name.starts_with(INSTALL_METADATA_PREFIX)
            && file_name.ends_with(INSTALL_METADATA_SUFFIX);
        if !is_metadata {
            continue;
        }

        let Ok(raw) = fs::read_to_string(&metadata_path) else {
            continue;
        };
        let Ok(metadata) = serde_json::from_str::<InstalledFontMetadata>(&raw) else {
            continue;
        };

        let manifest_matches = Path::new(&metadata.manifest_path)
            .canonicalize()
            .ok()
            .as_deref()
            .is_some_and(|path| path == manifest_canonical.as_path())
            || metadata.manifest_path == manifest_canonical.display().to_string();
        let font_matches = names.contains(&metadata.font_name);
        let project_matches = metadata.project_id.as_deref() == Some(config.project_id.as_str());

        if manifest_matches || (font_matches && project_matches) {
            metadata_paths_to_remove.insert(metadata_path.clone());
            ttf_paths_to_remove.insert(PathBuf::from(metadata.installed_ttf));
        }
    }

    for name in &names {
        ttf_paths_to_remove.insert(install_dir.join(font_file_name(name)));
        metadata_paths_to_remove.insert(metadata_path_for_font(&install_dir, name));
        metadata_paths_to_remove.insert(metadata_path_for_project_font(
            &install_dir,
            &config.project_id,
            name,
        ));
    }

    let mut removed_ttf_count = 0usize;
    for ttf_path in &ttf_paths_to_remove {
        if ttf_path.is_file() {
            fs::remove_file(ttf_path)
                .with_context(|| format!("failed to remove {}", ttf_path.display()))?;
            removed_ttf_count += 1;
        } else if ttf_path.exists() {
            bail!(
                "blocked uninstall for {}: expected file but found non-file at {}",
                install_dir.display(),
                ttf_path.display()
            );
        }
    }

    for metadata_path in &metadata_paths_to_remove {
        if metadata_path.is_file() {
            fs::remove_file(metadata_path)
                .with_context(|| format!("failed to remove {}", metadata_path.display()))?;
        } else if metadata_path.exists() {
            bail!(
                "blocked uninstall for {}: expected file but found non-file at {}",
                install_dir.display(),
                metadata_path.display()
            );
        }
    }

    update_fontconfig_petiglyph_alias(&install_dir, platform)?;
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
    let _guard = acquire_file_lock(&install_lock_path(&font_root), "font install metadata")?;
    let install_dir = install_dir_for_project(&font_root);

    if !install_dir.exists() {
        update_fontconfig_petiglyph_alias(&install_dir, platform)?;
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

    update_fontconfig_petiglyph_alias(&install_dir, platform)?;
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
