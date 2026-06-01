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
use walkdir::WalkDir;

use crate::project::{
    AnimationType, format_codepoint, load_runtime_config, parse_codepoint, read_manifest, slugify,
};

const INSTALL_METADATA_PREFIX: &str = ".petiglyph-install-";
const INSTALL_METADATA_SUFFIX: &str = ".json";
const INSTALL_LOCK_FILE_NAME: &str = ".petiglyph-install.lock";
const UNICODE_REGISTRY_FILE_NAME: &str = ".unicode-registry.json";
const UNICODE_REGISTRY_LOCK_FILE_NAME: &str = ".unicode-registry.lock";
const UNICODE_REGISTRY_VERSION: u32 = 1;
const SUPPLEMENTARY_PUA_START: u32 = 0xF0000;
const SUPPLEMENTARY_PUA_END: u32 = 0x10_FFFF;
const TEST_EXTERNAL_FONT_SCAN_DIR_NAME: &str = ".external-fonts";
const EXTERNAL_PUA_CACHE_FILE_NAME: &str = ".external-pua-cache.json";
const EXTERNAL_PUA_CACHE_VERSION: u32 = 1;
const MIN_PROJECT_RANGE_SIZE: u32 = 1;
const FILE_LOCK_TIMEOUT: Duration = Duration::from_secs(15);
const FILE_LOCK_STALE_AFTER: Duration = Duration::from_secs(120);
const FILE_LOCK_RETRY_DELAY: Duration = Duration::from_millis(50);
const CLI_VERSION: &str = env!("CARGO_PKG_VERSION");
const FONTCONFIG_PETIGLYPH_ALIAS_FILE_NAME: &str = "99-petiglyph.conf";
const INSTALL_ARTIFACT_TAG_LENGTHS: [usize; 7] = [0, 2, 4, 6, 8, 10, 12];
const FIRST_INSTALL_STATE_FILE_NAME: &str = "first-install.json";
const LEGACY_FIRST_INSTALL_STATE_FILE_NAME_V2: &str = ".petiglyph-first-install-state.json";
const LEGACY_FIRST_INSTALL_STATE_FILE_NAME: &str = ".petiglyph-machine-state.json";
const FIRST_INSTALL_STATE_VERSION: u32 = 1;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
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
    #[serde(default)]
    animation_snapshots: Vec<InstalledAnimationSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct InstalledAnimationSnapshot {
    name: String,
    #[serde(rename = "type")]
    animation_type: AnimationType,
    fps: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    grayscale_processing: Option<crate::animation_media::AnimationImportProcessingOptions>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    uniform_threshold: Option<u8>,
    #[serde(default, skip_serializing_if = "is_false")]
    variable_threshold: bool,
    frame_blocks: Vec<String>,
}

fn is_false(value: &bool) -> bool {
    !*value
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

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ExternalPuaCacheEntry {
    path: String,
    size_bytes: u64,
    modified_unix_ms: u128,
    #[serde(default)]
    codepoints: Vec<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ExternalPuaCacheFile {
    version: u32,
    #[serde(default)]
    entries: Vec<ExternalPuaCacheEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FirstInstallStateFile {
    version: u32,
    first_install_recorded_unix_ms: u128,
    recorded_by_cli_version: String,
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
    pub(crate) first_install_on_machine: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct FontUninstallResult {
    pub(crate) platform: FontPlatform,
    pub(crate) install_dir: PathBuf,
    pub(crate) outcome: UninstallOutcome,
    pub(crate) removed_ttf_count: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct ToolUninstallResult {
    pub(crate) platform: FontPlatform,
    pub(crate) install_dir: PathBuf,
    pub(crate) outcome: UninstallOutcome,
    pub(crate) removed_ttf_count: usize,
    pub(crate) removed_metadata_count: usize,
    pub(crate) removed_state_file_count: usize,
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

#[derive(Debug, Clone, Serialize)]
pub(crate) struct PuaUsageSummary {
    pub(crate) supplementary_pua_total: usize,
    pub(crate) external_occupied: usize,
    pub(crate) petiglyph_occupied: usize,
    pub(crate) available: usize,
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

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn install_artifact_base(font_name: &str) -> String {
    let slug = slugify(font_name);
    if slug.is_empty() {
        "petiglyph".to_string()
    } else {
        slug
    }
}

fn immutable_ttf_file_name(font_name: &str, artifact_hash: u64, tag_len: usize) -> String {
    let base = install_artifact_base(font_name);
    if tag_len == 0 {
        return format!("{base}.ttf");
    }
    let full_hash = format!("{artifact_hash:016x}");
    let tag_len = tag_len.min(full_hash.len());
    let short_tag = &full_hash[..tag_len];
    format!("{base}_{short_tag}.ttf")
}

fn immutable_ttf_install_path(
    install_dir: &Path,
    project_id: &str,
    font_name: &str,
    ttf_bytes: &[u8],
) -> Result<PathBuf> {
    let mut hash_input = Vec::with_capacity(project_id.len() + ttf_bytes.len() + 1);
    hash_input.extend_from_slice(project_id.as_bytes());
    hash_input.push(0);
    hash_input.extend_from_slice(ttf_bytes);
    let artifact_hash = fnv1a64(&hash_input);

    for tag_len in INSTALL_ARTIFACT_TAG_LENGTHS {
        let candidate =
            install_dir.join(immutable_ttf_file_name(font_name, artifact_hash, tag_len));
        if !candidate.exists() {
            return Ok(candidate);
        }
        if !candidate.is_file() {
            bail!(
                "blocked install for {}: expected file path but found non-file",
                candidate.display()
            );
        }
        let existing = fs::read(&candidate)
            .with_context(|| format!("failed to read {}", candidate.display()))?;
        if existing == ttf_bytes {
            return Ok(candidate);
        }
    }

    bail!(
        "hash collision while installing immutable artifact for {}",
        install_artifact_base(font_name)
    )
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

fn load_external_pua_cache(path: &Path) -> ExternalPuaCacheFile {
    if !path.exists() {
        return ExternalPuaCacheFile {
            version: EXTERNAL_PUA_CACHE_VERSION,
            entries: Vec::new(),
        };
    }

    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(_) => {
            return ExternalPuaCacheFile {
                version: EXTERNAL_PUA_CACHE_VERSION,
                entries: Vec::new(),
            };
        }
    };

    let parsed = serde_json::from_str::<ExternalPuaCacheFile>(&raw);
    let Ok(cache) = parsed else {
        return ExternalPuaCacheFile {
            version: EXTERNAL_PUA_CACHE_VERSION,
            entries: Vec::new(),
        };
    };

    if cache.version != EXTERNAL_PUA_CACHE_VERSION {
        return ExternalPuaCacheFile {
            version: EXTERNAL_PUA_CACHE_VERSION,
            entries: Vec::new(),
        };
    }

    cache
}

fn save_external_pua_cache(path: &Path, cache: &ExternalPuaCacheFile) -> Result<()> {
    let raw = serde_json::to_string_pretty(cache)
        .context("failed to serialize external supplementary PUA cache")?;
    write_atomic_string(path, &raw)
}

fn metadata_modified_unix_ms(metadata: &fs::Metadata) -> Option<u128> {
    metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis())
}

fn supplementary_pua_slot_count() -> usize {
    usize::try_from(SUPPLEMENTARY_PUA_END - SUPPLEMENTARY_PUA_START + 1)
        .expect("supplementary PUA slot count fits in usize")
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

fn first_install_state_path(install_dir: &Path) -> PathBuf {
    install_dir.join(FIRST_INSTALL_STATE_FILE_NAME)
}

fn legacy_first_install_state_path(font_root: &Path) -> PathBuf {
    font_root.join(LEGACY_FIRST_INSTALL_STATE_FILE_NAME)
}

fn legacy_first_install_state_path_v2(install_dir: &Path) -> PathBuf {
    install_dir.join(LEGACY_FIRST_INSTALL_STATE_FILE_NAME_V2)
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

pub(crate) fn supplementary_pua_usage_summary() -> Result<PuaUsageSummary> {
    let install_dir = managed_install_dir()?;
    fs::create_dir_all(&install_dir)
        .with_context(|| format!("failed to create {}", install_dir.display()))?;

    let lock_path = install_dir.join(UNICODE_REGISTRY_LOCK_FILE_NAME);
    let _guard = acquire_file_lock(&lock_path, "Unicode registry")?;

    let external = collect_external_supplementary_pua_codepoints(None, &install_dir)?;
    let petiglyph = collect_managed_supplementary_pua_codepoints(&install_dir)?;
    let total = supplementary_pua_slot_count();

    let mut combined = external.clone();
    combined.extend(petiglyph.iter().copied());
    let available = total.saturating_sub(combined.len());

    Ok(PuaUsageSummary {
        supplementary_pua_total: total,
        external_occupied: external.len(),
        petiglyph_occupied: petiglyph.len(),
        available,
    })
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

fn is_supported_font_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            matches!(
                ext.to_ascii_lowercase().as_str(),
                "ttf" | "otf" | "ttc" | "otc"
            )
        })
}

fn is_non_ownership_fallback_font(path: &Path) -> bool {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .is_some_and(|stem| stem.eq_ignore_ascii_case("lastresort"))
}

fn collect_pua_codepoints_from_face(face: &ttf_parser::Face<'_>, occupied: &mut BTreeSet<u32>) {
    let Some(cmap) = face.tables().cmap else {
        return;
    };

    for subtable in cmap.subtables {
        if !subtable.is_unicode() {
            continue;
        }
        subtable.codepoints(|codepoint| {
            if (SUPPLEMENTARY_PUA_START..=SUPPLEMENTARY_PUA_END).contains(&codepoint) {
                occupied.insert(codepoint);
            }
        });
    }
}

fn collect_pua_codepoints_from_font_data(data: &[u8]) -> BTreeSet<u32> {
    let mut occupied = BTreeSet::new();
    let face_count = ttf_parser::fonts_in_collection(data).unwrap_or(1);
    for face_index in 0..face_count {
        let Ok(face) = ttf_parser::Face::parse(data, face_index) else {
            continue;
        };
        collect_pua_codepoints_from_face(&face, &mut occupied);
    }
    occupied
}

fn system_font_scan_roots() -> Result<Vec<PathBuf>> {
    let mut roots = Vec::new();
    match current_platform()? {
        FontPlatform::Linux => {
            roots.push(PathBuf::from("/usr/share/fonts"));
            roots.push(PathBuf::from("/usr/local/share/fonts"));
            roots.push(user_font_root()?);
        }
        FontPlatform::Macos => {
            roots.push(PathBuf::from("/System/Library/Fonts"));
            roots.push(PathBuf::from("/Library/Fonts"));
            roots.push(user_font_root()?);
        }
        FontPlatform::Windows => {
            let windir = env::var_os("WINDIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("C:\\Windows"));
            roots.push(windir.join("Fonts"));
            roots.push(user_font_root()?);
        }
    }
    Ok(roots)
}

fn external_font_scan_roots(registry_root_override: Option<&Path>) -> Result<Vec<PathBuf>> {
    if let Some(root) = registry_root_override {
        return Ok(vec![root.join(TEST_EXTERNAL_FONT_SCAN_DIR_NAME)]);
    }
    system_font_scan_roots()
}

fn collect_external_supplementary_pua_codepoints(
    registry_root_override: Option<&Path>,
    managed_install_dir: &Path,
) -> Result<BTreeSet<u32>> {
    let roots = external_font_scan_roots(registry_root_override)?;
    let mut occupied = BTreeSet::new();
    let cache_path = managed_install_dir.join(EXTERNAL_PUA_CACHE_FILE_NAME);
    let mut cache = load_external_pua_cache(&cache_path);
    let mut cache_by_path = BTreeMap::new();
    for entry in cache.entries.drain(..) {
        cache_by_path.insert(entry.path.clone(), entry);
    }
    let mut seen_paths = BTreeSet::new();

    for root in roots {
        if !root.exists() {
            continue;
        }

        for entry in WalkDir::new(&root).follow_links(false) {
            let Ok(entry) = entry else {
                continue;
            };
            let path = entry.path();
            if !entry.file_type().is_file() || !is_supported_font_file(path) {
                continue;
            }
            if path.starts_with(managed_install_dir) {
                continue;
            }
            // LastResort advertises fallback coverage; it does not mean those PUA slots
            // are owned by an installed icon font.
            if is_non_ownership_fallback_font(path) {
                continue;
            }
            let path_key = path.display().to_string();
            seen_paths.insert(path_key.clone());

            let metadata = match entry.metadata() {
                Ok(metadata) => metadata,
                Err(_) => continue,
            };
            let size_bytes = metadata.len();
            let modified_unix_ms = metadata_modified_unix_ms(&metadata).unwrap_or(0);

            if let Some(cached) = cache_by_path.get(&path_key)
                && cached.size_bytes == size_bytes
                && cached.modified_unix_ms == modified_unix_ms
            {
                occupied.extend(cached.codepoints.iter().copied());
                continue;
            }

            let Ok(data) = fs::read(path) else {
                continue;
            };
            let found = collect_pua_codepoints_from_font_data(&data);
            occupied.extend(found.iter().copied());
            cache_by_path.insert(
                path_key,
                ExternalPuaCacheEntry {
                    path: path.display().to_string(),
                    size_bytes,
                    modified_unix_ms,
                    codepoints: found.into_iter().collect(),
                },
            );
        }
    }

    cache.entries = cache_by_path
        .into_values()
        .filter(|entry| seen_paths.contains(&entry.path))
        .collect();
    cache.version = EXTERNAL_PUA_CACHE_VERSION;
    let _ = save_external_pua_cache(&cache_path, &cache);

    Ok(occupied)
}

fn collect_managed_supplementary_pua_codepoints(install_dir: &Path) -> Result<BTreeSet<u32>> {
    let mut occupied = BTreeSet::new();
    if !install_dir.is_dir() {
        return Ok(occupied);
    }

    for entry in fs::read_dir(install_dir)
        .with_context(|| format!("failed to read {}", install_dir.display()))?
    {
        let entry =
            entry.with_context(|| format!("failed to read entry in {}", install_dir.display()))?;
        let path = entry.path();
        if !path.is_file() || !is_supported_font_file(&path) {
            continue;
        }
        let Ok(data) = fs::read(&path) else {
            continue;
        };
        occupied.extend(collect_pua_codepoints_from_font_data(&data));
    }

    Ok(occupied)
}

fn can_use_range(
    start: u32,
    end: u32,
    project_id: &str,
    registry: &UnicodeRegistryFile,
    external_occupied: &BTreeSet<u32>,
) -> Result<bool> {
    if external_occupied.range(start..=end).next().is_some() {
        return Ok(false);
    }

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
    value.clamp(SUPPLEMENTARY_PUA_START, SUPPLEMENTARY_PUA_END)
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
    external_occupied: &BTreeSet<u32>,
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
            if let Some(first_external) = external_occupied.range(cursor..=end).next().copied() {
                cursor = first_external.saturating_add(1);
                continue;
            }
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
    external_occupied: &BTreeSet<u32>,
) -> Result<Option<(u32, u32)>> {
    if locked_codepoints.is_empty() {
        return find_first_fit(
            preferred_start,
            span,
            project_id,
            registry,
            external_occupied,
        );
    }

    let Some(min_locked) = locked_codepoints.iter().min().copied() else {
        return find_first_fit(
            preferred_start,
            span,
            project_id,
            registry,
            external_occupied,
        );
    };
    let Some(max_locked) = locked_codepoints.iter().max().copied() else {
        return find_first_fit(
            preferred_start,
            span,
            project_id,
            registry,
            external_occupied,
        );
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
        if can_use_range(start, end, project_id, registry, external_occupied)? {
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
    let external_occupied =
        collect_external_supplementary_pua_codepoints(registry_root_override, &install_dir)?;

    let mut project_assignment_idx = None;
    if locked_codepoints
        .iter()
        .any(|cp| external_occupied.contains(cp))
    {
        bail!(
            "glyph lock/codepoint conflict: project {} uses codepoints already mapped by external installed fonts",
            project_id
        );
    }
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
                && can_use_range(
                    start,
                    expanded_end,
                    project_id,
                    &registry,
                    &external_occupied,
                )?
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
                    &external_occupied,
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

        let Some((start, end)) = find_first_fit(
            start_hint,
            required_span,
            project_id,
            &registry,
            &external_occupied,
        )?
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
            } else if slugify(trimmed) == project_slug {
                Ok(trimmed.to_string())
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
    project_id: Option<&str>,
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
        let name_matches = names.contains(&metadata.font_name);
        let id_matches = project_id.is_some_and(|pid| metadata.project_id.as_deref() == Some(pid));

        if !manifest_matches && !id_matches {
            continue;
        }
        if !name_matches {
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

fn install_dir_has_any_ttf(install_dir: &Path) -> Result<bool> {
    if !install_dir.is_dir() {
        return Ok(false);
    }

    for entry in fs::read_dir(install_dir)
        .with_context(|| format!("failed to read {}", install_dir.display()))?
    {
        let entry =
            entry.with_context(|| format!("failed to read entry in {}", install_dir.display()))?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let is_ttf = path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("ttf"));
        if is_ttf {
            return Ok(true);
        }
    }

    Ok(false)
}

fn mark_first_install_state(
    font_root: &Path,
    install_dir: &Path,
    had_existing_install_before: bool,
) -> Result<bool> {
    let legacy_state_path_v2 = legacy_first_install_state_path_v2(install_dir);
    if legacy_state_path_v2.is_file() {
        fs::remove_file(&legacy_state_path_v2)
            .with_context(|| format!("failed to remove {}", legacy_state_path_v2.display()))?;
    } else if legacy_state_path_v2.exists() {
        bail!(
            "blocked install for {}: expected file but found non-file at {}",
            install_dir.display(),
            legacy_state_path_v2.display()
        );
    }

    let legacy_state_path = legacy_first_install_state_path(font_root);
    if legacy_state_path.is_file() {
        fs::remove_file(&legacy_state_path)
            .with_context(|| format!("failed to remove {}", legacy_state_path.display()))?;
    } else if legacy_state_path.exists() {
        bail!(
            "blocked install for {}: expected file but found non-file at {}",
            install_dir.display(),
            legacy_state_path.display()
        );
    }

    let state_path = first_install_state_path(install_dir);
    if state_path.is_file() {
        return Ok(false);
    }
    if state_path.exists() {
        bail!(
            "blocked install for {}: expected file but found non-file at {}",
            install_dir.display(),
            state_path.display()
        );
    }

    let state = FirstInstallStateFile {
        version: FIRST_INSTALL_STATE_VERSION,
        first_install_recorded_unix_ms: now_millis(),
        recorded_by_cli_version: CLI_VERSION.to_string(),
    };
    let raw = serde_json::to_string_pretty(&state)
        .context("failed to serialize first install machine state")?;
    write_atomic_string(&state_path, &raw)
        .with_context(|| format!("failed to write {}", state_path.display()))?;

    Ok(!had_existing_install_before)
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
            match fs::remove_file(&alias_path) {
                Ok(()) => {}
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => {
                    return Err(err)
                        .with_context(|| format!("failed to remove {}", alias_path.display()));
                }
            }
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

fn compose_tile_source_key(
    parent_source_key: &str,
    rows: usize,
    cols: usize,
    row: usize,
    col: usize,
) -> String {
    format!("{parent_source_key}#compose:{rows}x{cols}:{row}:{col}")
}

fn emitted_composition_cols(logical_cols: usize) -> usize {
    logical_cols.checked_mul(2).unwrap_or(logical_cols)
}

fn mapped_source_block(by_source: &BTreeMap<String, String>, source_key: &str) -> Option<String> {
    by_source
        .get(source_key)
        .cloned()
        .or_else(|| {
            let (parent, rows, cols, row, col) = parse_compose_tile_key(source_key)?;
            by_source.iter().find_map(|(candidate_key, codepoint)| {
                let (
                    candidate_parent,
                    candidate_rows,
                    candidate_cols,
                    candidate_row,
                    candidate_col,
                ) = parse_compose_tile_key(candidate_key)?;
                if candidate_parent == parent
                    && candidate_rows == rows
                    && candidate_cols == cols
                    && candidate_row == row
                    && candidate_col == col
                {
                    Some(codepoint.clone())
                } else {
                    None
                }
            })
        })
        .as_deref()
        .and_then(|cp| parse_codepoint(cp).ok())
        .and_then(char::from_u32)
        .map(|c| c.to_string())
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

fn animation_frame_parent_source(source_key: &str) -> String {
    source_key
        .split_once("#compose:")
        .map_or_else(|| source_key.to_string(), |(parent, _)| parent.to_string())
}

fn animation_snapshots_from_manifest_and_mapping(
    manifest_path: &Path,
) -> Vec<InstalledAnimationSnapshot> {
    let manifest = match read_manifest(manifest_path) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let mapping_path = manifest_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(&manifest.out_dir)
        .join("glyph-map.json");
    let mapping_raw = match fs::read_to_string(mapping_path) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let mappings: Vec<crate::build::MappingEntry> = match serde_json::from_str(&mapping_raw) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let mut by_source = BTreeMap::new();
    for entry in mappings {
        by_source.insert(entry.source_file, entry.codepoint);
    }
    let base_threshold = manifest.threshold;
    let threshold_overrides = manifest.threshold_overrides.clone();

    let mut out = Vec::new();
    for animation in manifest.animations {
        let threshold_sources = animation
            .frames
            .iter()
            .map(|frame| animation_frame_parent_source(frame))
            .collect::<BTreeSet<_>>();
        let mut threshold_values = threshold_sources
            .iter()
            .map(|source| {
                threshold_overrides
                    .get(source)
                    .copied()
                    .unwrap_or(base_threshold)
            })
            .collect::<Vec<_>>();
        let first_threshold = threshold_values.first().copied();
        let variable_threshold = match first_threshold {
            Some(first) => threshold_values.drain(1..).any(|value| value != first),
            None => false,
        };
        let uniform_threshold = if variable_threshold {
            None
        } else {
            first_threshold
        };
        let mut frame_blocks = Vec::new();
        for frame in &animation.frames {
            let block = match animation.animation_type {
                AnimationType::Standard => mapped_source_block(&by_source, frame)
                    .unwrap_or_else(|| format!("[missing:{frame}]")),
                AnimationType::Grid => {
                    let rows = animation.rows.unwrap_or(1);
                    let cols = emitted_composition_cols(animation.cols.unwrap_or(1));
                    let mut lines = Vec::new();
                    for row in 0..rows {
                        let mut line = String::new();
                        for col in 0..cols {
                            let key = compose_tile_source_key(frame, rows, cols, row, col);
                            let ch = by_source
                                .get(&key)
                                .and_then(|cp| parse_codepoint(cp).ok())
                                .and_then(char::from_u32)
                                .unwrap_or(' ');
                            line.push(ch);
                        }
                        lines.push(line);
                    }
                    lines.join("\n")
                }
            };
            frame_blocks.push(block);
        }
        out.push(InstalledAnimationSnapshot {
            name: animation.name,
            animation_type: animation.animation_type,
            fps: animation.fps,
            grayscale_processing: animation.grayscale_processing,
            uniform_threshold,
            variable_threshold,
            frame_blocks,
        });
    }
    out
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
    let had_existing_install_before = install_dir_has_any_ttf(&install_dir)?;

    let built_bytes =
        fs::read(built_ttf).with_context(|| format!("failed to read {}", built_ttf.display()))?;
    let install_path =
        immutable_ttf_install_path(&install_dir, project_id, font_name, &built_bytes)?;
    if !install_path.exists() {
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
        animation_snapshots: animation_snapshots_from_manifest_and_mapping(manifest_path),
    };
    let metadata_json =
        serde_json::to_string_pretty(&metadata).context("failed to serialize install metadata")?;
    write_atomic_string(&metadata_path, &metadata_json)
        .with_context(|| format!("failed to write {}", metadata_path.display()))?;

    let mut replaced_previous_ttf_count = 0usize;
    if let Some(previous) = previous_installed_ttf
        && previous != install_path
        && previous.is_file()
    {
        fs::remove_file(&previous)
            .with_context(|| format!("failed to remove {}", previous.display()))?;
        replaced_previous_ttf_count += 1;
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
    let first_install_on_machine =
        mark_first_install_state(&font_root, &install_dir, had_existing_install_before)?;
    Ok(FontInstallResult {
        platform,
        install_dir,
        install_path,
        replaced_previous_ttf_count,
        first_install_on_machine,
    })
}

pub(crate) fn uninstall_project_font(manifest_path: &Path) -> Result<FontUninstallResult> {
    let platform = current_platform()?;
    let font_root = user_font_root()?;
    fs::create_dir_all(&font_root)
        .with_context(|| format!("failed to create {}", font_root.display()))?;
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
    fs::create_dir_all(&font_root)
        .with_context(|| format!("failed to create {}", font_root.display()))?;
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

fn uninstall_tool_state_for_font_root(font_root: &Path) -> Result<ToolUninstallResult> {
    let platform = current_platform()?;
    fs::create_dir_all(font_root)
        .with_context(|| format!("failed to create {}", font_root.display()))?;
    let _guard = acquire_file_lock(&install_lock_path(font_root), "font install metadata")?;
    let install_dir = install_dir_for_project(font_root);

    if !install_dir.exists() {
        update_fontconfig_petiglyph_alias(&install_dir, platform)?;
        return Ok(ToolUninstallResult {
            platform,
            install_dir,
            outcome: UninstallOutcome::AlreadyAbsent,
            removed_ttf_count: 0,
            removed_metadata_count: 0,
            removed_state_file_count: 0,
        });
    }

    let entries: Vec<_> = fs::read_dir(&install_dir)
        .with_context(|| format!("failed to read {}", install_dir.display()))?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .collect();

    let mut removed_ttf_count = 0usize;
    let mut removed_metadata_count = 0usize;
    let mut removed_state_file_count = 0usize;

    for path in entries {
        if !path.is_file() {
            bail!(
                "blocked tool uninstall for {}: expected file but found non-file at {}",
                install_dir.display(),
                path.display()
            );
        }

        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        let is_ttf = path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("ttf"));
        let is_metadata = file_name.starts_with(INSTALL_METADATA_PREFIX)
            && file_name.ends_with(INSTALL_METADATA_SUFFIX);
        let is_state = matches!(
            file_name,
            UNICODE_REGISTRY_FILE_NAME
                | UNICODE_REGISTRY_LOCK_FILE_NAME
                | EXTERNAL_PUA_CACHE_FILE_NAME
                | FIRST_INSTALL_STATE_FILE_NAME
                | LEGACY_FIRST_INSTALL_STATE_FILE_NAME_V2
        );

        if !(is_ttf || is_metadata || is_state) {
            bail!(
                "blocked tool uninstall for {}: unexpected file {}",
                install_dir.display(),
                path.display()
            );
        }

        fs::remove_file(&path).with_context(|| format!("failed to remove {}", path.display()))?;
        if is_ttf {
            removed_ttf_count += 1;
        } else if is_metadata {
            removed_metadata_count += 1;
        } else {
            removed_state_file_count += 1;
        }
    }

    update_fontconfig_petiglyph_alias(&install_dir, platform)?;
    if removed_ttf_count > 0 {
        refresh_font_cache(font_root, platform)?;
    }

    if fs::read_dir(&install_dir)
        .with_context(|| format!("failed to read {}", install_dir.display()))?
        .next()
        .is_none()
    {
        fs::remove_dir(&install_dir)
            .with_context(|| format!("failed to remove {}", install_dir.display()))?;
    }

    let removed_any =
        removed_ttf_count > 0 || removed_metadata_count > 0 || removed_state_file_count > 0;
    let outcome = if removed_any {
        UninstallOutcome::Removed
    } else {
        UninstallOutcome::AlreadyAbsent
    };

    Ok(ToolUninstallResult {
        platform,
        install_dir,
        outcome,
        removed_ttf_count,
        removed_metadata_count,
        removed_state_file_count,
    })
}

pub(crate) fn uninstall_tool_state() -> Result<ToolUninstallResult> {
    let font_root = user_font_root()?;
    uninstall_tool_state_for_font_root(&font_root)
}

#[cfg(test)]
mod tests {
    use super::{
        animation_snapshots_from_manifest_and_mapping, first_install_state_path,
        immutable_ttf_file_name, immutable_ttf_install_path, install_dir_for_project,
        install_dir_has_any_ttf, mark_first_install_state, uninstall_tool_state_for_font_root,
    };
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn make_temp_dir(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time is valid")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("petiglyph-install-{name}-{nonce}"));
        fs::create_dir_all(&dir).expect("temp dir is created");
        dir
    }

    #[test]
    fn standard_animation_snapshot_resolves_composed_frame_keys() {
        let project_dir = make_temp_dir("standard-animation-composed-frames");
        let manifest_path = project_dir.join("petiglyph.toml");
        fs::create_dir_all(project_dir.join("build")).expect("build dir is created");
        fs::write(
            &manifest_path,
            r#"input_dir = "icons"
out_dir = "build"
font_name = "Demo"
glyph_size = 32
threshold = 128
codepoint_start = "U+100000"

[[animations]]
name = "demo"
type = "standard"
fps = 6
frames = [
  "strip.png#compose:1x2:0:0",
  "strip.png#compose:1x2:0:1",
]
"#,
        )
        .expect("manifest is written");
        fs::write(
            project_dir.join("build").join("glyph-map.json"),
            r#"[
  { "glyph_name": "strip_r1_c1", "source_file": "strip.png#compose:1x2:0:0", "codepoint": "U+100000" },
  { "glyph_name": "strip_r1_c2", "source_file": "strip.png#compose:1x2:0:1", "codepoint": "U+100001" }
]"#,
        )
        .expect("glyph map is written");

        let snapshots = animation_snapshots_from_manifest_and_mapping(&manifest_path);

        assert_eq!(snapshots.len(), 1);
        assert_eq!(
            snapshots[0].frame_blocks,
            vec![
                char::from_u32(0x100000).unwrap().to_string(),
                char::from_u32(0x100001).unwrap().to_string(),
            ]
        );

        fs::remove_dir_all(project_dir).expect("temp dir is removed");
    }

    #[test]
    fn standard_animation_snapshot_requires_matching_grid_dims() {
        let project_dir = make_temp_dir("standard-animation-compose-dims");
        let manifest_path = project_dir.join("petiglyph.toml");
        fs::create_dir_all(project_dir.join("build")).expect("build dir is created");
        fs::write(
            &manifest_path,
            r#"input_dir = "icons"
out_dir = "build"
font_name = "Demo"
glyph_size = 32
threshold = 128
codepoint_start = "U+100000"

[[animations]]
name = "demo"
type = "standard"
fps = 6
frames = [
  "strip.png#compose:1x2:0:0",
  "strip.png#compose:1x2:0:1",
]
"#,
        )
        .expect("manifest is written");
        fs::write(
            project_dir.join("build").join("glyph-map.json"),
            r#"[
  { "glyph_name": "strip_r1_c1", "source_file": "strip.png#compose:1x4:0:0", "codepoint": "U+100000" },
  { "glyph_name": "strip_r1_c2", "source_file": "strip.png#compose:1x4:0:1", "codepoint": "U+100001" }
]"#,
        )
        .expect("glyph map is written");

        let snapshots = animation_snapshots_from_manifest_and_mapping(&manifest_path);

        assert_eq!(snapshots.len(), 1);
        // Grid dimensions must match; 1x2 frames do not match 1x4 tiles
        assert_eq!(
            snapshots[0].frame_blocks,
            vec![
                "[missing:strip.png#compose:1x2:0:0]".to_string(),
                "[missing:strip.png#compose:1x2:0:1]".to_string(),
            ]
        );

        fs::remove_dir_all(project_dir).expect("temp dir is removed");
    }

    #[test]
    fn grid_animation_snapshot_uses_emitted_composition_columns() {
        let project_dir = make_temp_dir("grid-animation-emitted-cols");
        let manifest_path = project_dir.join("petiglyph.toml");
        fs::create_dir_all(project_dir.join("build")).expect("build dir is created");
        fs::write(
            &manifest_path,
            r#"input_dir = "icons"
out_dir = "build"
font_name = "Demo"
glyph_size = 32
threshold = 128
codepoint_start = "U+100000"

[[animations]]
name = "walk"
type = "grid"
fps = 6
frames = ["strip.png"]
rows = 2
cols = 2
horizontal_bleed = "weak"
vertical_bleed = "off"
"#,
        )
        .expect("manifest is written");
        fs::write(
            project_dir.join("build").join("glyph-map.json"),
            r#"[
  { "glyph_name": "strip_r1_c1", "source_file": "strip.png#compose:2x4:0:0", "codepoint": "U+100000" },
  { "glyph_name": "strip_r1_c2", "source_file": "strip.png#compose:2x4:0:1", "codepoint": "U+100001" },
  { "glyph_name": "strip_r1_c3", "source_file": "strip.png#compose:2x4:0:2", "codepoint": "U+100002" },
  { "glyph_name": "strip_r1_c4", "source_file": "strip.png#compose:2x4:0:3", "codepoint": "U+100003" },
  { "glyph_name": "strip_r2_c1", "source_file": "strip.png#compose:2x4:1:0", "codepoint": "U+100004" },
  { "glyph_name": "strip_r2_c2", "source_file": "strip.png#compose:2x4:1:1", "codepoint": "U+100005" },
  { "glyph_name": "strip_r2_c3", "source_file": "strip.png#compose:2x4:1:2", "codepoint": "U+100006" },
  { "glyph_name": "strip_r2_c4", "source_file": "strip.png#compose:2x4:1:3", "codepoint": "U+100007" }
]"#,
        )
        .expect("glyph map is written");

        let snapshots = animation_snapshots_from_manifest_and_mapping(&manifest_path);

        assert_eq!(snapshots.len(), 1);
        assert_eq!(
            snapshots[0].frame_blocks,
            vec![format!(
                "{}{}{}{}\n{}{}{}{}",
                char::from_u32(0x100000).unwrap(),
                char::from_u32(0x100001).unwrap(),
                char::from_u32(0x100002).unwrap(),
                char::from_u32(0x100003).unwrap(),
                char::from_u32(0x100004).unwrap(),
                char::from_u32(0x100005).unwrap(),
                char::from_u32(0x100006).unwrap(),
                char::from_u32(0x100007).unwrap()
            )]
        );

        fs::remove_dir_all(project_dir).expect("temp dir is removed");
    }

    #[test]
    fn immutable_ttf_file_name_uses_plain_name_for_zero_suffix_length() {
        let name = immutable_ttf_file_name("Test Font", 0x1234_5678_9abc_def0, 0);
        assert_eq!(name, "test_font.ttf");
    }

    #[test]
    fn external_scan_ignores_lastresort_fallback_fonts() {
        assert!(super::is_non_ownership_fallback_font(&PathBuf::from(
            "/System/Library/Fonts/LastResort.otf"
        )));
        assert!(!super::is_non_ownership_fallback_font(&PathBuf::from(
            "/Library/Fonts/UsefulIconFont.otf"
        )));
    }

    #[test]
    fn immutable_ttf_install_path_escalates_suffix_and_caps_at_twelve_hex() {
        let install_dir = make_temp_dir("suffix-escalation");
        let project_id = "project-1";
        let font_name = "Test Font";
        let ttf_bytes = b"new-bytes";

        let plain_name = "test_font.ttf";
        fs::write(install_dir.join(plain_name), b"existing")
            .expect("conflicting plain artifact is written");
        let first_candidate =
            immutable_ttf_install_path(&install_dir, project_id, font_name, ttf_bytes)
                .expect("next candidate should resolve");
        let first_name = first_candidate
            .file_name()
            .and_then(|name| name.to_str())
            .expect("candidate name");
        assert!(
            first_name.starts_with("test_font_")
                && first_name.ends_with(".ttf")
                && first_name.len() == "test_font_aa.ttf".len(),
            "expected 2-hex fallback after plain-name conflict, got {first_name}"
        );

        let full_hash = format!(
            "{:016x}",
            super::fnv1a64(&[project_id.as_bytes(), &[0], ttf_bytes,].concat())
        );
        for tag_len in [0usize, 2, 4, 6, 8, 10, 12] {
            let name = if tag_len == 0 {
                "test_font.ttf".to_string()
            } else {
                format!("test_font_{}.ttf", &full_hash[..tag_len])
            };
            fs::write(install_dir.join(name), b"different-bytes")
                .expect("conflicting candidate is written");
        }

        let err = immutable_ttf_install_path(&install_dir, project_id, font_name, ttf_bytes)
            .expect_err("should fail after exhausting up to 12 hex");
        let message = err.to_string();
        assert!(
            message.contains("hash collision"),
            "expected collision error after exhausting 12 hex, got {message}"
        );

        fs::remove_dir_all(install_dir).expect("temp dir is removed");
    }

    #[test]
    fn first_install_state_marks_first_then_skips_afterward() {
        let font_root = make_temp_dir("first-install-state");
        let install_dir = install_dir_for_project(&font_root);
        fs::create_dir_all(&install_dir).expect("install dir is created");

        let had_existing = install_dir_has_any_ttf(&install_dir).expect("ttf scan should work");
        assert!(!had_existing, "fresh install dir should have no ttf files");

        let first = mark_first_install_state(&font_root, &install_dir, had_existing)
            .expect("first mark should succeed");
        assert!(first, "first marker write should report first install");
        assert!(
            first_install_state_path(&install_dir).is_file(),
            "first-install marker file should be written"
        );

        let second = mark_first_install_state(&font_root, &install_dir, false)
            .expect("second mark should also succeed");
        assert!(
            !second,
            "subsequent marker checks should not report first install"
        );

        fs::remove_dir_all(font_root).expect("temp dir is removed");
    }

    #[test]
    fn first_install_state_backfills_without_first_flag_when_ttf_exists() {
        let font_root = make_temp_dir("first-install-backfill");
        let install_dir = install_dir_for_project(&font_root);
        fs::create_dir_all(&install_dir).expect("install dir is created");
        fs::write(install_dir.join("already_here.ttf"), b"ttf").expect("existing ttf is written");

        let had_existing = install_dir_has_any_ttf(&install_dir).expect("ttf scan should work");
        assert!(had_existing, "existing ttf should be detected");

        let first = mark_first_install_state(&font_root, &install_dir, had_existing)
            .expect("marker write should succeed");
        assert!(
            !first,
            "backfill marker with pre-existing install should not report first install"
        );

        fs::remove_dir_all(font_root).expect("temp dir is removed");
    }

    #[test]
    fn first_install_state_removes_legacy_root_marker() {
        let font_root = make_temp_dir("first-install-legacy-cleanup");
        let install_dir = install_dir_for_project(&font_root);
        fs::create_dir_all(&install_dir).expect("install dir is created");
        let legacy = super::legacy_first_install_state_path(&font_root);
        fs::write(&legacy, b"legacy").expect("legacy marker is written");

        let first = mark_first_install_state(&font_root, &install_dir, false)
            .expect("marker write should succeed");
        assert!(first, "fresh marker should still report first install");
        assert!(
            !legacy.exists(),
            "legacy marker at font root should be removed during migration"
        );

        fs::remove_dir_all(font_root).expect("temp dir is removed");
    }

    #[test]
    fn uninstall_tool_state_removes_all_managed_files() {
        let font_root = make_temp_dir("tool-uninstall-removes-all");
        let install_dir = install_dir_for_project(&font_root);
        fs::create_dir_all(&install_dir).expect("install dir is created");

        fs::write(install_dir.join("demo_font.ttf"), b"ttf").expect("ttf is written");
        fs::write(install_dir.join(".petiglyph-install-demo.json"), b"{}")
            .expect("metadata is written");
        fs::write(
            install_dir.join(super::UNICODE_REGISTRY_FILE_NAME),
            b"{\"version\":1,\"assignments\":[]}",
        )
        .expect("registry is written");
        fs::write(
            install_dir.join(super::FIRST_INSTALL_STATE_FILE_NAME),
            b"{\"version\":1}",
        )
        .expect("first-install is written");

        let result =
            uninstall_tool_state_for_font_root(&font_root).expect("tool uninstall should succeed");

        assert_eq!(result.outcome, super::UninstallOutcome::Removed);
        assert_eq!(result.removed_ttf_count, 1);
        assert_eq!(result.removed_metadata_count, 1);
        assert_eq!(result.removed_state_file_count, 2);
        assert!(
            !install_dir.exists(),
            "install dir should be removed when tool uninstall deletes all state"
        );

        fs::remove_dir_all(font_root).expect("temp dir is removed");
    }

    #[test]
    fn uninstall_tool_state_is_idempotent_when_absent() {
        let font_root = make_temp_dir("tool-uninstall-idempotent");
        let result =
            uninstall_tool_state_for_font_root(&font_root).expect("tool uninstall should succeed");
        assert_eq!(result.outcome, super::UninstallOutcome::AlreadyAbsent);
        assert_eq!(result.removed_ttf_count, 0);
        assert_eq!(result.removed_metadata_count, 0);
        assert_eq!(result.removed_state_file_count, 0);

        fs::remove_dir_all(font_root).expect("temp dir is removed");
    }

    #[test]
    fn uninstall_tool_state_rejects_unexpected_files() {
        let font_root = make_temp_dir("tool-uninstall-unexpected-file");
        let install_dir = install_dir_for_project(&font_root);
        fs::create_dir_all(&install_dir).expect("install dir is created");
        fs::write(install_dir.join("notes.txt"), b"unexpected")
            .expect("unexpected file is written");

        let error = uninstall_tool_state_for_font_root(&font_root)
            .expect_err("unexpected files should block tool uninstall");
        let message = error.to_string();
        assert!(
            message.contains("unexpected file"),
            "expected strict-file validation error, got: {message}"
        );

        fs::remove_dir_all(font_root).expect("temp dir is removed");
    }
}
