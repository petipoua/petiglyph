use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::build::collect_source_files;
use crate::install::{managed_install_dir, reserve_project_unicode_range};
use crate::project::{load_runtime_config, manifest_path_from_option, parse_codepoint};

const INSTALL_METADATA_PREFIX: &str = ".petiglyph-install-";
const INSTALL_METADATA_SUFFIX: &str = ".json";
const GLOBAL_LOCK_FILES: [&str; 2] = [".unicode-registry.lock", ".petiglyph-install.lock"];
const PROJECT_LOCK_FILE: &str = ".petiglyph-build.lock";
const STALE_LOCK_AGE: Duration = Duration::from_secs(120);

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DoctorSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DoctorStatus {
    Ok,
    Issue,
    Repaired,
}

#[derive(Debug, Serialize)]
pub(crate) struct DoctorFinding {
    pub(crate) code: String,
    pub(crate) severity: DoctorSeverity,
    pub(crate) status: DoctorStatus,
    pub(crate) message: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct DoctorReport {
    pub(crate) manifest: Option<String>,
    pub(crate) project_id: Option<String>,
    pub(crate) install_dir: String,
    pub(crate) registry_path: String,
    pub(crate) repair: bool,
    pub(crate) healthy: bool,
    pub(crate) warnings: usize,
    pub(crate) errors: usize,
    pub(crate) repaired: usize,
    pub(crate) findings: Vec<DoctorFinding>,
}

#[derive(Debug, Deserialize)]
struct RegistryRaw {
    version: Option<u32>,
    #[serde(default)]
    assignments: Vec<RegistryAssignmentRaw>,
}

#[derive(Debug, Deserialize)]
struct RegistryAssignmentRaw {
    project_id: String,
    range_start: String,
    range_end: String,
}

#[derive(Debug, Deserialize)]
struct GlyphLockRaw {
    version: Option<u32>,
    project_id: Option<String>,
    #[serde(default)]
    entries: Vec<GlyphLockEntryRaw>,
}

#[derive(Debug, Deserialize)]
struct GlyphLockEntryRaw {
    source_file: String,
    codepoint: String,
    #[serde(default = "default_lock_entry_active")]
    active: bool,
}

fn default_lock_entry_active() -> bool {
    true
}

#[derive(Debug, Deserialize)]
struct InstallMetadataRaw {
    installed_ttf: String,
}

#[derive(Debug, Clone, Copy)]
struct ParsedRange {
    start: u32,
    end: u32,
}

struct RegistrySnapshot {
    assignment_count: usize,
    ranges: BTreeMap<String, ParsedRange>,
}

impl RegistrySnapshot {
    fn parse(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self {
                assignment_count: 0,
                ranges: BTreeMap::new(),
            });
        }

        let raw = fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let decoded: RegistryRaw = serde_json::from_str(&raw)
            .with_context(|| format!("failed to parse {}", path.display()))?;
        if decoded.version != Some(1) {
            anyhow::bail!(
                "unsupported registry version in {}: expected 1, got {:?}",
                path.display(),
                decoded.version
            );
        }

        let mut ranges = BTreeMap::new();
        for item in &decoded.assignments {
            let start = parse_codepoint(&item.range_start)
                .with_context(|| format!("invalid range_start for project {}", item.project_id))?;
            let end = parse_codepoint(&item.range_end)
                .with_context(|| format!("invalid range_end for project {}", item.project_id))?;
            if start > end {
                anyhow::bail!(
                    "invalid registry range for {}: {}..{}",
                    item.project_id,
                    item.range_start,
                    item.range_end
                );
            }
            if ranges
                .insert(item.project_id.clone(), ParsedRange { start, end })
                .is_some()
            {
                anyhow::bail!("duplicate registry project_id {}", item.project_id);
            }
        }

        let mut sorted = ranges
            .iter()
            .map(|(project, range)| (project.clone(), range.start, range.end))
            .collect::<Vec<_>>();
        sorted.sort_by_key(|(_, start, _)| *start);
        for window in sorted.windows(2) {
            let (_, _, left_end) = &window[0];
            let (_, right_start, _) = &window[1];
            if right_start <= left_end {
                anyhow::bail!("overlapping project ranges found in Unicode registry");
            }
        }

        Ok(Self {
            assignment_count: decoded.assignments.len(),
            ranges,
        })
    }
}

fn push_finding(
    findings: &mut Vec<DoctorFinding>,
    code: &str,
    severity: DoctorSeverity,
    status: DoctorStatus,
    message: impl Into<String>,
) {
    findings.push(DoctorFinding {
        code: code.to_string(),
        severity,
        status,
        message: message.into(),
    });
}

fn stale_lock(path: &Path) -> bool {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| modified.elapsed().ok())
        .is_some_and(|age| age > STALE_LOCK_AGE)
}

fn load_glyph_lock(lock_path: &Path) -> Result<GlyphLockRaw> {
    let raw = fs::read_to_string(lock_path)
        .with_context(|| format!("failed to read {}", lock_path.display()))?;
    serde_json::from_str(&raw).with_context(|| format!("failed to parse {}", lock_path.display()))
}

pub(crate) fn doctor(repair: bool, manifest_arg: Option<PathBuf>) -> Result<DoctorReport> {
    let install_dir = managed_install_dir()?;
    let registry_path = install_dir.join(".unicode-registry.json");
    let mut findings = Vec::new();
    let mut repaired = 0usize;

    if !install_dir.exists() {
        push_finding(
            &mut findings,
            "install_dir_missing",
            DoctorSeverity::Info,
            DoctorStatus::Ok,
            format!(
                "managed install directory does not exist yet: {}",
                install_dir.display()
            ),
        );
    }

    let registry = match RegistrySnapshot::parse(&registry_path) {
        Ok(snapshot) => {
            push_finding(
                &mut findings,
                "registry_readable",
                DoctorSeverity::Info,
                DoctorStatus::Ok,
                format!(
                    "Unicode registry is readable with {} assignment(s)",
                    snapshot.assignment_count
                ),
            );
            snapshot
        }
        Err(err) => {
            push_finding(
                &mut findings,
                "registry_invalid",
                DoctorSeverity::Error,
                DoctorStatus::Issue,
                format!("Unicode registry is invalid: {err:#}"),
            );
            RegistrySnapshot {
                assignment_count: 0,
                ranges: BTreeMap::new(),
            }
        }
    };

    for file_name in GLOBAL_LOCK_FILES {
        let lock_path = install_dir.join(file_name);
        if !lock_path.exists() {
            continue;
        }
        if stale_lock(&lock_path) {
            if repair {
                fs::remove_file(&lock_path).with_context(|| {
                    format!("failed to remove stale lock {}", lock_path.display())
                })?;
                repaired += 1;
                push_finding(
                    &mut findings,
                    "stale_global_lock",
                    DoctorSeverity::Warning,
                    DoctorStatus::Repaired,
                    format!("removed stale global lock file {}", lock_path.display()),
                );
            } else {
                push_finding(
                    &mut findings,
                    "stale_global_lock",
                    DoctorSeverity::Warning,
                    DoctorStatus::Issue,
                    format!(
                        "stale global lock file detected: {} (run `petiglyph doctor --repair`)",
                        lock_path.display()
                    ),
                );
            }
        }
    }

    if install_dir.exists() {
        let mut orphaned_metadata = Vec::new();
        for entry in fs::read_dir(&install_dir)
            .with_context(|| format!("failed to read {}", install_dir.display()))?
        {
            let entry = entry
                .with_context(|| format!("failed to read entry in {}", install_dir.display()))?;
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            if !(name.starts_with(INSTALL_METADATA_PREFIX)
                && name.ends_with(INSTALL_METADATA_SUFFIX))
            {
                continue;
            }

            let raw = fs::read_to_string(&path)
                .with_context(|| format!("failed to read {}", path.display()))?;
            let metadata: InstallMetadataRaw = serde_json::from_str(&raw)
                .with_context(|| format!("failed to parse {}", path.display()))?;
            if !Path::new(&metadata.installed_ttf).is_file() {
                orphaned_metadata.push(path);
            }
        }

        if orphaned_metadata.is_empty() {
            push_finding(
                &mut findings,
                "install_metadata_consistent",
                DoctorSeverity::Info,
                DoctorStatus::Ok,
                "install metadata references existing TTF files",
            );
        } else if repair {
            for path in &orphaned_metadata {
                fs::remove_file(path).with_context(|| {
                    format!("failed to remove orphan metadata {}", path.display())
                })?;
                repaired += 1;
            }
            push_finding(
                &mut findings,
                "orphan_install_metadata",
                DoctorSeverity::Warning,
                DoctorStatus::Repaired,
                format!(
                    "removed {} orphan install metadata file(s)",
                    orphaned_metadata.len()
                ),
            );
        } else {
            push_finding(
                &mut findings,
                "orphan_install_metadata",
                DoctorSeverity::Warning,
                DoctorStatus::Issue,
                format!(
                    "found {} orphan install metadata file(s) (run `petiglyph doctor --repair`)",
                    orphaned_metadata.len()
                ),
            );
        }
    }

    let mut manifest = None;
    let mut project_id = None;

    let resolved_manifest = match manifest_arg {
        Some(path) => Some(manifest_path_from_option(Some(path))?),
        None => match manifest_path_from_option(None) {
            Ok(path) => Some(path),
            Err(err) => {
                push_finding(
                    &mut findings,
                    "manifest_auto_detect",
                    DoctorSeverity::Warning,
                    DoctorStatus::Issue,
                    format!("no project context selected for project checks: {err}"),
                );
                None
            }
        },
    };

    if let Some(manifest_path) = resolved_manifest {
        manifest = Some(manifest_path.display().to_string());
        let config = load_runtime_config(&manifest_path, None, None, None, None, None)?;
        project_id = Some(config.project_id.clone());

        let project_lock = config.project_dir.join(PROJECT_LOCK_FILE);
        if project_lock.exists() && stale_lock(&project_lock) {
            if repair {
                fs::remove_file(&project_lock).with_context(|| {
                    format!("failed to remove stale lock {}", project_lock.display())
                })?;
                repaired += 1;
                push_finding(
                    &mut findings,
                    "stale_project_lock",
                    DoctorSeverity::Warning,
                    DoctorStatus::Repaired,
                    format!("removed stale project lock {}", project_lock.display()),
                );
            } else {
                push_finding(
                    &mut findings,
                    "stale_project_lock",
                    DoctorSeverity::Warning,
                    DoctorStatus::Issue,
                    format!(
                        "stale project build lock detected: {} (run `petiglyph doctor --repair`)",
                        project_lock.display()
                    ),
                );
            }
        }

        let glyph_lock_path = config.project_dir.join("petiglyph.lock");
        let mut locked_codepoints = BTreeSet::new();
        let mut active_entries = 0usize;
        let mut glyph_lock_parse_error = false;

        if glyph_lock_path.exists() {
            match load_glyph_lock(&glyph_lock_path) {
                Ok(lock) => {
                    if lock.version != Some(1) {
                        push_finding(
                            &mut findings,
                            "glyph_lock_version",
                            DoctorSeverity::Error,
                            DoctorStatus::Issue,
                            format!(
                                "unsupported glyph lock version in {}: {:?}",
                                glyph_lock_path.display(),
                                lock.version
                            ),
                        );
                    }
                    if lock.project_id.as_deref() != Some(config.project_id.as_str()) {
                        push_finding(
                            &mut findings,
                            "glyph_lock_project_id",
                            DoctorSeverity::Error,
                            DoctorStatus::Issue,
                            format!(
                                "glyph lock project_id mismatch in {} (manifest={}, lock={:?})",
                                glyph_lock_path.display(),
                                config.project_id,
                                lock.project_id
                            ),
                        );
                    }

                    let mut seen_sources = BTreeSet::new();
                    for entry in &lock.entries {
                        if !seen_sources.insert(entry.source_file.clone()) {
                            push_finding(
                                &mut findings,
                                "glyph_lock_duplicate_source",
                                DoctorSeverity::Error,
                                DoctorStatus::Issue,
                                format!(
                                    "duplicate source_file {} in {}",
                                    entry.source_file,
                                    glyph_lock_path.display()
                                ),
                            );
                        }
                        match parse_codepoint(&entry.codepoint) {
                            Ok(codepoint) => {
                                if !locked_codepoints.insert(codepoint) {
                                    push_finding(
                                        &mut findings,
                                        "glyph_lock_duplicate_codepoint",
                                        DoctorSeverity::Error,
                                        DoctorStatus::Issue,
                                        format!(
                                            "duplicate codepoint {} in {}",
                                            entry.codepoint,
                                            glyph_lock_path.display()
                                        ),
                                    );
                                }
                            }
                            Err(err) => push_finding(
                                &mut findings,
                                "glyph_lock_invalid_codepoint",
                                DoctorSeverity::Error,
                                DoctorStatus::Issue,
                                format!(
                                    "invalid codepoint {} in {}: {err:#}",
                                    entry.codepoint,
                                    glyph_lock_path.display()
                                ),
                            ),
                        }
                        if entry.active {
                            active_entries += 1;
                        }
                    }
                }
                Err(err) => {
                    glyph_lock_parse_error = true;
                    push_finding(
                        &mut findings,
                        "glyph_lock_invalid",
                        DoctorSeverity::Error,
                        DoctorStatus::Issue,
                        format!("failed to parse {}: {err:#}", glyph_lock_path.display()),
                    );
                }
            }
        } else {
            push_finding(
                &mut findings,
                "glyph_lock_missing",
                DoctorSeverity::Warning,
                DoctorStatus::Issue,
                format!(
                    "glyph lock is missing at {} (run a build to create it)",
                    glyph_lock_path.display()
                ),
            );
        }

        if let Ok(sources) = collect_source_files(&config.input_dir) {
            push_finding(
                &mut findings,
                "project_sources_visible",
                DoctorSeverity::Info,
                DoctorStatus::Ok,
                format!(
                    "project has {} source image(s) in {}",
                    sources.len(),
                    config.input_dir.display()
                ),
            );
        }

        if !glyph_lock_parse_error {
            if let Some(range) = registry.ranges.get(&config.project_id) {
                let out_of_range = locked_codepoints
                    .iter()
                    .any(|cp| *cp < range.start || *cp > range.end);
                if out_of_range {
                    push_finding(
                        &mut findings,
                        "registry_project_range_conflict",
                        DoctorSeverity::Error,
                        DoctorStatus::Issue,
                        format!(
                            "project lock has codepoints outside owned range U+{:04X}..U+{:04X}",
                            range.start, range.end
                        ),
                    );
                } else {
                    push_finding(
                        &mut findings,
                        "registry_project_range_ok",
                        DoctorSeverity::Info,
                        DoctorStatus::Ok,
                        format!(
                            "project owns Unicode range U+{:04X}..U+{:04X}",
                            range.start, range.end
                        ),
                    );
                }
            } else if repair {
                let required = active_entries.max(1);
                match reserve_project_unicode_range(
                    None,
                    &config.project_id,
                    config.codepoint_start,
                    required,
                    &locked_codepoints,
                ) {
                    Ok(range) => {
                        repaired += 1;
                        push_finding(
                            &mut findings,
                            "registry_project_assignment",
                            DoctorSeverity::Warning,
                            DoctorStatus::Repaired,
                            format!(
                                "created project Unicode assignment U+{:04X}..U+{:04X}",
                                range.range_start, range.range_end
                            ),
                        );
                    }
                    Err(err) => push_finding(
                        &mut findings,
                        "registry_project_assignment",
                        DoctorSeverity::Error,
                        DoctorStatus::Issue,
                        format!("failed to repair project registry assignment: {err:#}"),
                    ),
                }
            } else {
                push_finding(
                    &mut findings,
                    "registry_project_missing",
                    DoctorSeverity::Warning,
                    DoctorStatus::Issue,
                    "project has no Unicode range assignment in registry (run `petiglyph doctor --repair`)",
                );
            }

            for (other_project, range) in &registry.ranges {
                if other_project == &config.project_id {
                    continue;
                }
                if locked_codepoints
                    .iter()
                    .any(|cp| *cp >= range.start && *cp <= range.end)
                {
                    push_finding(
                        &mut findings,
                        "registry_cross_project_conflict",
                        DoctorSeverity::Error,
                        DoctorStatus::Issue,
                        format!(
                            "glyph lock uses codepoints owned by {} (U+{:04X}..U+{:04X})",
                            other_project, range.start, range.end
                        ),
                    );
                }
            }
        }
    }

    let warnings = findings
        .iter()
        .filter(|item| matches!(item.severity, DoctorSeverity::Warning))
        .count();
    let errors = findings
        .iter()
        .filter(|item| matches!(item.severity, DoctorSeverity::Error))
        .count();
    let healthy = errors == 0 && warnings == 0;

    Ok(DoctorReport {
        manifest,
        project_id,
        install_dir: install_dir.display().to_string(),
        registry_path: registry_path.display().to_string(),
        repair,
        healthy,
        warnings,
        errors,
        repaired,
        findings,
    })
}
