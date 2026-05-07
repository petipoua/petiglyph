use anyhow::{Context, Result, bail};
use image::{Rgba, RgbaImage};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use walkdir::WalkDir;

use crate::compose::compose_tiles;
use crate::glyph_debug;
use crate::image_pipeline::{
    coverage_map_from_image, preprocess_standard_source, terminal_cell_width_for_height,
};
use crate::install::reserve_project_unicode_range;
use crate::project::{CompositionDef, RuntimeConfig, format_codepoint, parse_codepoint};

#[derive(Debug, Clone)]
pub(crate) struct PreprocessedGlyph {
    pub(crate) source_path: PathBuf,
    pub(crate) source_key: String,
    pub(crate) source_parent_key: String,
    pub(crate) glyph_name: String,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) coverage: Vec<u8>,
    pub(crate) image_fingerprint: String,
    pub(crate) composition_tile: Option<CompositionTileInfo>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CompositionTileInfo {
    pub(crate) rows: usize,
    pub(crate) cols: usize,
    pub(crate) row: usize,
    pub(crate) col: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct GlyphBitmap {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) pixels: Vec<bool>,
}

#[derive(Debug, Clone, Copy)]
struct TtfGlyphOptions {
    center_in_line_box: bool,
}

impl TtfGlyphOptions {
    fn centered() -> Self {
        Self {
            center_in_line_box: true,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct MappingEntry {
    pub(crate) glyph_name: String,
    pub(crate) source_file: String,
    pub(crate) codepoint: String,
}

#[derive(Debug, Clone)]
pub(crate) struct BuildSummary {
    pub(crate) glyph_count: usize,
    pub(crate) bdf_path: PathBuf,
    pub(crate) ttf_path: PathBuf,
    pub(crate) mapping_path: PathBuf,
    pub(crate) sample_path: PathBuf,
    pub(crate) previews_dir: PathBuf,
}

const GLYPH_LOCK_FILE_NAME: &str = "petiglyph.lock";
const GLYPH_BUILD_LOCK_FILE_NAME: &str = ".petiglyph-build.lock";
const GLYPH_LOCK_VERSION: u32 = 1;
const BUILD_LOCK_TIMEOUT: Duration = Duration::from_secs(15);
const BUILD_LOCK_STALE_AFTER: Duration = Duration::from_secs(120);
const BUILD_LOCK_RETRY_DELAY: Duration = Duration::from_millis(50);

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct BuildOptions {
    pub(crate) force_remap: bool,
}

#[derive(Debug)]
struct BuildFileLockGuard {
    path: PathBuf,
}

impl Drop for BuildFileLockGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GlyphLockEntry {
    source_file: String,
    codepoint: String,
    image_fingerprint: String,
    #[serde(default = "default_lock_entry_active")]
    active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GlyphLockFile {
    version: u32,
    project_id: String,
    codepoint_start: String,
    #[serde(default)]
    entries: Vec<GlyphLockEntry>,
}

fn default_lock_entry_active() -> bool {
    true
}

pub(crate) fn build_outputs(config: &RuntimeConfig) -> Result<BuildSummary> {
    build_outputs_with_options(config, BuildOptions::default())
}

pub(crate) fn build_outputs_with_options(
    config: &RuntimeConfig,
    options: BuildOptions,
) -> Result<BuildSummary> {
    glyph_debug::begin_session(&config.project_dir, "build");
    glyph_debug::log_step(
        "build.start",
        format!(
            "project={} input_dir={} glyph_size={}",
            config.project_id,
            config.input_dir.display(),
            config.glyph_size
        ),
    );
    let sources = collect_source_files(&config.input_dir)?;
    let glyphs = preprocess_sources_with_compositions(
        &sources,
        &config.input_dir,
        config.glyph_size,
        &config.compositions,
    )?;
    validate_codepoint_range(config.codepoint_start, glyphs.len())?;
    let assigned_codepoints = assign_codepoints_for_build(config, &glyphs, options)?;

    if config.out_dir.exists() {
        if config.out_dir.is_dir() {
            fs::remove_dir_all(&config.out_dir)
                .with_context(|| format!("failed to clear {}", config.out_dir.display()))?;
        } else {
            bail!(
                "build output path is not a directory: {}",
                config.out_dir.display()
            );
        }
    }

    let previews_dir = config.out_dir.join("previews");
    fs::create_dir_all(&previews_dir)
        .with_context(|| format!("failed to create {}", previews_dir.display()))?;

    let mut mapping = Vec::with_capacity(glyphs.len());
    let mut bdf_glyphs = Vec::with_capacity(glyphs.len());
    let mut ttf_glyph_options = Vec::with_capacity(glyphs.len());

    for (idx, glyph) in glyphs.iter().enumerate() {
        let codepoint = assigned_codepoints[idx];
        let threshold = effective_threshold(
            config.base_threshold,
            &config.threshold_overrides,
            &glyph.source_parent_key,
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
        ttf_glyph_options.push(TtfGlyphOptions {
            center_in_line_box: glyph.composition_tile.is_none(),
        });
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
    write_ttf(
        &ttf_path,
        &config.font_name,
        &config.project_id,
        config.glyph_size,
        &bdf_glyphs,
        &ttf_glyph_options,
    )?;

    let sample_path = config.out_dir.join("glyph-sample.txt");
    let sample = generate_smart_sample(&glyphs, &assigned_codepoints);
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

fn glyph_lock_path(config: &RuntimeConfig) -> PathBuf {
    config.project_dir.join(GLYPH_LOCK_FILE_NAME)
}

fn build_lock_path(config: &RuntimeConfig) -> PathBuf {
    config.project_dir.join(GLYPH_BUILD_LOCK_FILE_NAME)
}

fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn acquire_build_lock(config: &RuntimeConfig) -> Result<BuildFileLockGuard> {
    let path = build_lock_path(config);
    let started = SystemTime::now();
    loop {
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                let _ = writeln!(
                    file,
                    "pid={} created_unix_ms={}",
                    std::process::id(),
                    now_millis()
                );
                return Ok(BuildFileLockGuard { path });
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                let stale = fs::metadata(&path)
                    .and_then(|metadata| metadata.modified())
                    .ok()
                    .and_then(|modified| modified.elapsed().ok())
                    .is_some_and(|elapsed| elapsed > BUILD_LOCK_STALE_AFTER);
                if stale {
                    let _ = fs::remove_file(&path);
                    continue;
                }
                if started.elapsed().unwrap_or_default() >= BUILD_LOCK_TIMEOUT {
                    bail!("timed out waiting for build lock {}", path.display());
                }
                thread::sleep(BUILD_LOCK_RETRY_DELAY);
            }
            Err(err) => {
                return Err(err)
                    .with_context(|| format!("failed to create build lock {}", path.display()));
            }
        }
    }
}

fn load_glyph_lock(config: &RuntimeConfig) -> Result<GlyphLockFile> {
    let path = glyph_lock_path(config);
    if !path.exists() {
        return Ok(GlyphLockFile {
            version: GLYPH_LOCK_VERSION,
            project_id: config.project_id.clone(),
            codepoint_start: format_codepoint(config.codepoint_start),
            entries: Vec::new(),
        });
    }

    let raw =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    let lock: GlyphLockFile = serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse {}", path.display()))?;

    if lock.version != GLYPH_LOCK_VERSION {
        bail!(
            "unsupported glyph lock version in {}: expected {}, got {}",
            path.display(),
            GLYPH_LOCK_VERSION,
            lock.version
        );
    }

    if lock.project_id.trim().is_empty() {
        bail!("glyph lock project_id is empty in {}", path.display());
    }

    if lock.project_id != config.project_id {
        bail!(
            "glyph lock project_id mismatch in {} (manifest project_id={}, lock project_id={})",
            path.display(),
            config.project_id,
            lock.project_id
        );
    }

    for entry in &lock.entries {
        if entry.source_file.trim().is_empty() {
            bail!(
                "glyph lock contains an empty source_file in {}",
                path.display()
            );
        }
        parse_codepoint(&entry.codepoint).with_context(|| {
            format!(
                "invalid codepoint {} for {} in {}",
                entry.codepoint,
                entry.source_file,
                path.display()
            )
        })?;
    }

    Ok(lock)
}

fn save_glyph_lock(config: &RuntimeConfig, lock: &GlyphLockFile) -> Result<()> {
    let path = glyph_lock_path(config);
    let raw = serde_json::to_string_pretty(lock).context("failed to serialize glyph lock")?;
    fs::write(&path, raw).with_context(|| format!("failed to write {}", path.display()))
}

fn fingerprint_bytes(data: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in data {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("fnv1a64:{hash:016x}")
}

fn fingerprint_source(path: &Path) -> Result<String> {
    let data = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    Ok(fingerprint_bytes(&data))
}

fn next_available_codepoint(start: u32, used: &BTreeSet<u32>) -> Result<u32> {
    let mut candidate = start;
    loop {
        if candidate > 0x10_FFFF {
            bail!(
                "no available Unicode codepoint left from U+{:04X} to U+10FFFF",
                start
            );
        }

        if is_valid_unicode_scalar(candidate) && !used.contains(&candidate) {
            return Ok(candidate);
        }

        candidate = candidate
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("no available Unicode codepoint left"))?;
    }
}

fn next_available_codepoint_run(
    start: u32,
    used: &BTreeSet<u32>,
    span: usize,
    allocation_end: u32,
) -> Result<Vec<u32>> {
    if span == 0 {
        bail!("cannot allocate an empty codepoint run");
    }
    if span == 1 {
        return Ok(vec![next_available_codepoint(start, used)?]);
    }

    let span_u32 = u32::try_from(span).context("codepoint run size overflow")?;
    let mut candidate = start;
    loop {
        let last = candidate
            .checked_add(span_u32.saturating_sub(1))
            .ok_or_else(|| anyhow::anyhow!("no available Unicode codepoint run left"))?;
        if last > allocation_end {
            bail!(
                "no contiguous Unicode run of {} codepoints available from U+{:04X} to {}",
                span,
                start,
                format_codepoint(allocation_end)
            );
        }

        let mut valid = true;
        for offset in 0..span_u32 {
            let cp = candidate + offset;
            if !is_valid_unicode_scalar(cp) || used.contains(&cp) {
                valid = false;
                candidate = cp
                    .checked_add(1)
                    .ok_or_else(|| anyhow::anyhow!("no available Unicode codepoint run left"))?;
                break;
            }
        }

        if valid {
            return Ok((0..span_u32).map(|offset| candidate + offset).collect());
        }
    }
}

fn retired_source_key(source_key: &str, old_codepoint: u32) -> String {
    format!("{source_key}#retired:{old_codepoint:06X}")
}

#[derive(Debug, Clone, Copy)]
struct GlyphGroupPlan {
    start_idx: usize,
    len: usize,
    reusable: bool,
    is_composition: bool,
}

fn detect_glyph_groups(
    glyphs: &[PreprocessedGlyph],
    entries_by_source: &BTreeMap<String, GlyphLockEntry>,
) -> Result<(Vec<GlyphGroupPlan>, usize)> {
    let mut groups = Vec::new();
    let mut new_allocations = 0usize;
    let mut idx = 0usize;
    while idx < glyphs.len() {
        let glyph = &glyphs[idx];
        if let Some(tile) = &glyph.composition_tile {
            if tile.row != 0 || tile.col != 0 {
                bail!(
                    "composition tile ordering is invalid for {} (first tile must be row=0,col=0)",
                    glyph.source_parent_key
                );
            }

            let len = tile.rows.checked_mul(tile.cols).ok_or_else(|| {
                anyhow::anyhow!("composition grid too large for {}", glyph.source_parent_key)
            })?;
            if idx + len > glyphs.len() {
                bail!(
                    "composition glyph count mismatch for {}: expected {} tiles",
                    glyph.source_parent_key,
                    len
                );
            }

            for offset in 0..len {
                let expected_row = offset / tile.cols;
                let expected_col = offset % tile.cols;
                let child = &glyphs[idx + offset];
                let Some(child_tile) = &child.composition_tile else {
                    bail!(
                        "composition glyph sequence interrupted for {}",
                        glyph.source_parent_key
                    );
                };
                if child.source_parent_key != glyph.source_parent_key
                    || child_tile.rows != tile.rows
                    || child_tile.cols != tile.cols
                    || child_tile.row != expected_row
                    || child_tile.col != expected_col
                {
                    bail!(
                        "composition glyph sequence ordering mismatch for {}",
                        glyph.source_parent_key
                    );
                }
            }

            let mut all_exist = true;
            let mut cps = Vec::with_capacity(len);
            for offset in 0..len {
                let child = &glyphs[idx + offset];
                let Some(entry) = entries_by_source.get(&child.source_key) else {
                    all_exist = false;
                    break;
                };
                cps.push(parse_codepoint(&entry.codepoint)?);
            }
            let reusable = if all_exist {
                let first = cps[0];
                cps.iter()
                    .enumerate()
                    .all(|(offset, cp)| *cp == first + offset as u32)
            } else {
                false
            };

            if !reusable {
                new_allocations = new_allocations
                    .checked_add(len)
                    .ok_or_else(|| anyhow::anyhow!("glyph count is too large to allocate"))?;
            }

            groups.push(GlyphGroupPlan {
                start_idx: idx,
                len,
                reusable,
                is_composition: true,
            });
            idx += len;
            continue;
        }

        let reusable = entries_by_source.contains_key(&glyph.source_key);
        if !reusable {
            new_allocations = new_allocations
                .checked_add(1)
                .ok_or_else(|| anyhow::anyhow!("glyph count is too large to allocate"))?;
        }
        groups.push(GlyphGroupPlan {
            start_idx: idx,
            len: 1,
            reusable,
            is_composition: false,
        });
        idx += 1;
    }

    Ok((groups, new_allocations))
}

fn assign_codepoints_for_build(
    config: &RuntimeConfig,
    glyphs: &[PreprocessedGlyph],
    options: BuildOptions,
) -> Result<Vec<u32>> {
    let _build_lock = acquire_build_lock(config)?;
    let mut lock = if options.force_remap {
        GlyphLockFile {
            version: GLYPH_LOCK_VERSION,
            project_id: config.project_id.clone(),
            codepoint_start: format_codepoint(config.codepoint_start),
            entries: Vec::new(),
        }
    } else {
        load_glyph_lock(config)?
    };
    let mut entries_by_source: BTreeMap<String, GlyphLockEntry> = BTreeMap::new();
    let mut used_codepoints = BTreeSet::new();

    for entry in lock.entries.drain(..) {
        if entries_by_source
            .insert(entry.source_file.clone(), entry.clone())
            .is_some()
        {
            bail!(
                "duplicate source_file in glyph lock {}: {}",
                glyph_lock_path(config).display(),
                entry.source_file
            );
        }

        let parsed = parse_codepoint(&entry.codepoint)?;
        if !used_codepoints.insert(parsed) {
            bail!(
                "duplicate codepoint in glyph lock {}: {}",
                glyph_lock_path(config).display(),
                entry.codepoint
            );
        }
    }

    let (group_plans, new_allocations) = detect_glyph_groups(glyphs, &entries_by_source)?;

    let required_codepoints = used_codepoints
        .len()
        .checked_add(new_allocations)
        .ok_or_else(|| anyhow::anyhow!("glyph count is too large to allocate codepoints"))?;
    #[cfg(test)]
    let registry_root_override = Some(config.project_dir.as_path());
    #[cfg(not(test))]
    let registry_root_override = None;

    let range = reserve_project_unicode_range(
        registry_root_override,
        &config.project_id,
        config.codepoint_start,
        required_codepoints,
        &used_codepoints,
    )
    .map_err(|err| {
        if options.force_remap {
            err
        } else {
            err.context(
                "if this conflict is intentional, rerun with `--force-remap` to rebuild glyph mappings in a fresh owned range",
            )
        }
    })?;
    let allocation_start = range.range_start;
    let allocation_end = range.range_end;
    lock.codepoint_start = format_codepoint(allocation_start);

    let mut assigned = vec![0u32; glyphs.len()];
    let mut active_sources = BTreeSet::new();

    for plan in &group_plans {
        if plan.reusable {
            for offset in 0..plan.len {
                let idx = plan.start_idx + offset;
                let glyph = &glyphs[idx];
                active_sources.insert(glyph.source_key.clone());
                let Some(entry) = entries_by_source.get_mut(&glyph.source_key) else {
                    bail!("missing glyph lock entry for {}", glyph.source_key);
                };
                entry.active = true;
                entry.image_fingerprint = glyph.image_fingerprint.clone();
                assigned[idx] = parse_codepoint(&entry.codepoint)?;
            }
            continue;
        }

        let allocated = if plan.is_composition {
            next_available_codepoint_run(
                allocation_start,
                &used_codepoints,
                plan.len,
                allocation_end,
            )?
        } else {
            vec![next_available_codepoint(
                allocation_start,
                &used_codepoints,
            )?]
        };

        for codepoint in &allocated {
            if *codepoint > allocation_end {
                bail!(
                    "Unicode range conflict: project {} exhausted its owned range {}..{}",
                    config.project_id,
                    format_codepoint(allocation_start),
                    format_codepoint(allocation_end)
                );
            }
            used_codepoints.insert(*codepoint);
        }

        for offset in 0..plan.len {
            let idx = plan.start_idx + offset;
            let glyph = &glyphs[idx];
            active_sources.insert(glyph.source_key.clone());
            let codepoint = allocated[offset];

            let existing_old = entries_by_source
                .get(&glyph.source_key)
                .map(|entry| (entry.codepoint.clone(), entry.image_fingerprint.clone()));
            if let Some((old_codepoint_raw, old_fingerprint)) = existing_old {
                let old_codepoint = parse_codepoint(&old_codepoint_raw)?;
                if old_codepoint != codepoint {
                    let retired_key = retired_source_key(&glyph.source_key, old_codepoint);
                    entries_by_source
                        .entry(retired_key.clone())
                        .or_insert(GlyphLockEntry {
                            source_file: retired_key,
                            codepoint: format_codepoint(old_codepoint),
                            image_fingerprint: old_fingerprint,
                            active: false,
                        });
                }
            }

            entries_by_source.insert(
                glyph.source_key.clone(),
                GlyphLockEntry {
                    source_file: glyph.source_key.clone(),
                    codepoint: format_codepoint(codepoint),
                    image_fingerprint: glyph.image_fingerprint.clone(),
                    active: true,
                },
            );
            assigned[idx] = codepoint;
        }
    }

    for (source_key, entry) in &mut entries_by_source {
        if !active_sources.contains(source_key) {
            entry.active = false;
        }
    }

    lock.entries = entries_by_source.into_values().collect();
    save_glyph_lock(config, &lock)?;

    Ok(assigned)
}

fn is_valid_unicode_scalar(codepoint: u32) -> bool {
    codepoint <= 0x10_FFFF && !(0xD800..=0xDFFF).contains(&codepoint)
}

fn validate_codepoint_range(codepoint_start: u32, glyph_count: usize) -> Result<()> {
    if glyph_count == 0 {
        return Ok(());
    }

    if !is_valid_unicode_scalar(codepoint_start) {
        bail!(
            "codepoint_start is not a valid Unicode scalar value: U+{:04X}",
            codepoint_start
        );
    }

    let max_offset = u32::try_from(glyph_count - 1)
        .context("glyph count is too large to assign Unicode codepoints")?;
    let codepoint_end = codepoint_start.checked_add(max_offset).ok_or_else(|| {
        anyhow::anyhow!(
            "codepoint range overflow: start U+{:04X} with {} glyphs",
            codepoint_start,
            glyph_count
        )
    })?;

    if codepoint_end > 0x10_FFFF {
        bail!(
            "codepoint range exceeds Unicode limit: start U+{:04X}, glyph_count {}, max U+10FFFF",
            codepoint_start,
            glyph_count
        );
    }

    if codepoint_start <= 0xDFFF && codepoint_end >= 0xD800 {
        bail!(
            "codepoint range intersects UTF-16 surrogate range (U+D800..U+DFFF): start U+{:04X}, end U+{:04X}",
            codepoint_start,
            codepoint_end
        );
    }

    Ok(())
}

fn expected_font_file_stem(font_name: &str) -> String {
    let slug = slugify(font_name);
    if slug.is_empty() {
        "petiglyph".to_string()
    } else {
        slug
    }
}

pub(crate) fn expected_ttf_path(config: &RuntimeConfig) -> PathBuf {
    config.out_dir.join(format!(
        "{}.ttf",
        expected_font_file_stem(&config.font_name)
    ))
}

pub(crate) fn expected_bdf_path(config: &RuntimeConfig) -> PathBuf {
    config.out_dir.join(format!(
        "{}.bdf",
        expected_font_file_stem(&config.font_name)
    ))
}

pub(crate) fn collect_source_files(input_dir: &Path) -> Result<Vec<PathBuf>> {
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

pub(crate) fn is_supported_source(path: &Path) -> bool {
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

#[allow(dead_code)]
pub(crate) fn preprocess_sources(
    sources: &[PathBuf],
    input_dir: &Path,
    glyph_size: u32,
) -> Result<Vec<PreprocessedGlyph>> {
    preprocess_sources_with_compositions(sources, input_dir, glyph_size, &BTreeMap::new())
}

pub(crate) fn preprocess_sources_with_compositions(
    sources: &[PathBuf],
    input_dir: &Path,
    glyph_size: u32,
    compositions: &BTreeMap<String, CompositionDef>,
) -> Result<Vec<PreprocessedGlyph>> {
    let mut used_names = HashSet::new();
    let mut out = Vec::new();

    for source in sources {
        let source_key = source_manifest_key(source, input_dir);
        glyph_debug::log_step(
            "source.begin",
            format!("source={} path={}", source_key, source.display()),
        );
        if let Some(def) = compositions.get(&source_key) {
            glyph_debug::log_step(
                "source.compose",
                format!("source={} rows={} cols={}", source_key, def.rows, def.cols),
            );
            let tiles = compose_tiles(source, &source_key, def.rows, def.cols, glyph_size)?;
            let base_name = unique_glyph_name(source, &mut used_names);
            for tile in tiles {
                let tile_source_key =
                    compose_tile_source_key(&source_key, tile.rows, tile.cols, tile.row, tile.col);
                let tile_name = unique_glyph_name_for_seed(
                    &format!("{}_r{}_c{}", base_name, tile.row + 1, tile.col + 1),
                    &mut used_names,
                );
                out.push(PreprocessedGlyph {
                    source_path: source.clone(),
                    source_key: tile_source_key,
                    source_parent_key: source_key.clone(),
                    glyph_name: tile_name,
                    width: tile.width,
                    height: tile.height,
                    coverage: tile.coverage,
                    image_fingerprint: tile.fingerprint,
                    composition_tile: Some(CompositionTileInfo {
                        rows: tile.rows,
                        cols: tile.cols,
                        row: tile.row,
                        col: tile.col,
                    }),
                });
            }
            glyph_debug::log_step("source.done", format!("source={} mode=compose", source_key));
            continue;
        }

        glyph_debug::log_step("source.standard", format!("source={source_key}"));
        let glyph_width = terminal_cell_width_for_height(glyph_size);
        let glyph_height = glyph_size;
        let coverage = preprocess_standard_source(source, glyph_width, glyph_height, &source_key)?;
        let glyph_name = unique_glyph_name(source, &mut used_names);
        let image_fingerprint = fingerprint_source(source)?;
        out.push(PreprocessedGlyph {
            source_path: source.clone(),
            source_key: source_key.clone(),
            source_parent_key: source_key.clone(),
            glyph_name,
            width: glyph_width,
            height: glyph_height,
            coverage,
            image_fingerprint,
            composition_tile: None,
        });
        glyph_debug::log_step(
            "source.done",
            format!("source={} mode=standard", source_key),
        );
    }

    Ok(out)
}

fn compose_tile_source_key(
    source_key: &str,
    rows: usize,
    cols: usize,
    row: usize,
    col: usize,
) -> String {
    format!("{source_key}#compose:{rows}x{cols}:{row}:{col}")
}

#[allow(dead_code)]
pub(crate) fn coverage_map(source: &RgbaImage, glyph_size: u32) -> Result<Vec<u8>> {
    coverage_map_from_image(source, glyph_size)
}

fn threshold_bitmap(glyph: &PreprocessedGlyph, threshold: u8) -> GlyphBitmap {
    let pixels = glyph
        .coverage
        .iter()
        .map(|v| *v >= threshold)
        .collect::<Vec<bool>>();
    glyph_debug::log_step(
        "threshold",
        format!(
            "source={} glyph={} threshold={}",
            glyph.source_parent_key, glyph.glyph_name, threshold
        ),
    );
    glyph_debug::write_bitmap_png(
        "10_threshold_bitmap",
        &glyph.glyph_name,
        glyph.width,
        glyph.height,
        &pixels,
    );
    GlyphBitmap {
        width: glyph.width,
        height: glyph.height,
        pixels,
    }
}

fn write_preview_png(path: &Path, bitmap: &GlyphBitmap) -> Result<()> {
    let mut img = RgbaImage::from_pixel(bitmap.width, bitmap.height, Rgba([255, 255, 255, 0]));

    for y in 0..bitmap.height as usize {
        for x in 0..bitmap.width as usize {
            let idx = y * bitmap.width as usize + x;
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
    let metrics = font_vertical_metrics(
        u16::try_from(glyph_size).context("glyph_size is too large for BDF export")?,
    );
    let glyph_width = terminal_cell_width_for_height(glyph_size);

    out.push_str("STARTFONT 2.1\n");
    out.push_str(&format!(
        "FONT {}\n",
        bdf_font_name(font_name, glyph_width, glyph_size)
    ));
    out.push_str(&format!("SIZE {} 75 75\n", glyph_size));
    out.push_str(&format!(
        "FONTBOUNDINGBOX {} {} 0 {}\n",
        glyph_width, glyph_size, metrics.descent
    ));
    out.push_str("STARTPROPERTIES 2\n");
    out.push_str(&format!("FONT_ASCENT {}\n", metrics.ascent));
    out.push_str(&format!("FONT_DESCENT {}\n", metrics.descent_abs()));
    out.push_str("ENDPROPERTIES\n");
    out.push_str(&format!("CHARS {}\n", glyphs.len()));

    for (name, codepoint, bitmap) in glyphs {
        out.push_str(&format!("STARTCHAR {}\n", name));
        out.push_str(&format!("ENCODING {}\n", codepoint));
        out.push_str("SWIDTH 500 0\n");
        out.push_str(&format!("DWIDTH {} 0\n", bitmap.width));
        out.push_str(&format!(
            "BBX {} {} 0 {}\n",
            bitmap.width, bitmap.height, metrics.descent
        ));
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

#[derive(Debug, Clone, Copy)]
struct FontVerticalMetrics {
    ascent: i16,
    descent: i16,
}

impl FontVerticalMetrics {
    fn descent_abs(self) -> u16 {
        self.descent.unsigned_abs()
    }
}

fn font_vertical_metrics(units_per_em: u16) -> FontVerticalMetrics {
    let descent_abs = ((u32::from(units_per_em) + 2) / 5) as i16;
    FontVerticalMetrics {
        ascent: units_per_em as i16 - descent_abs,
        descent: -descent_abs,
    }
}

fn write_ttf(
    path: &Path,
    font_name: &str,
    font_identity: &str,
    glyph_size: u32,
    glyphs: &[(String, u32, GlyphBitmap)],
    glyph_options: &[TtfGlyphOptions],
) -> Result<()> {
    let bytes = build_ttf(font_name, font_identity, glyph_size, glyphs, glyph_options)?;
    fs::write(path, bytes).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

fn build_ttf(
    font_name: &str,
    font_identity: &str,
    glyph_size: u32,
    glyphs: &[(String, u32, GlyphBitmap)],
    glyph_options: &[TtfGlyphOptions],
) -> Result<Vec<u8>> {
    if glyphs.len() != glyph_options.len() {
        bail!(
            "TTF export metadata mismatch: {} glyphs but {} centering flags",
            glyphs.len(),
            glyph_options.len()
        );
    }

    let units_per_em = glyph_size
        .checked_mul(16)
        .context("glyph_size is too large for TTF export")?;
    let units_per_em =
        u16::try_from(units_per_em).context("glyph_size is too large for TTF export")?;
    let vertical_metrics = font_vertical_metrics(units_per_em);
    let glyph_width = terminal_cell_width_for_height(glyph_size);
    let cell_advance_width = u16::try_from(
        glyph_width
            .checked_mul(16)
            .context("glyph width is too large for TTF export")?,
    )
    .context("glyph width is too large for TTF export")?;

    let mut ttf_glyphs = Vec::with_capacity(glyphs.len() + 2);
    ttf_glyphs.push(bitmap_glyph_to_ttf(
        &notdef_bitmap(glyph_width, glyph_size),
        None,
        units_per_em,
        vertical_metrics,
        cell_advance_width,
        TtfGlyphOptions::centered(),
    )?);
    ttf_glyphs.push(bitmap_glyph_to_ttf(
        &GlyphBitmap {
            width: glyph_width,
            height: glyph_size,
            pixels: vec![false; (glyph_width as usize).saturating_mul(glyph_size as usize)],
        },
        Some(0x0020),
        units_per_em,
        vertical_metrics,
        cell_advance_width,
        TtfGlyphOptions::centered(),
    )?);

    for (idx, (_, codepoint, bitmap)) in glyphs.iter().enumerate() {
        if bitmap.width != glyph_width || bitmap.height != glyph_size {
            bail!(
                "TTF export expected {}x{} glyph bitmap, got {}x{}",
                glyph_width,
                glyph_size,
                bitmap.width,
                bitmap.height
            );
        }
        ttf_glyphs.push(bitmap_glyph_to_ttf(
            bitmap,
            Some(*codepoint),
            units_per_em,
            vertical_metrics,
            cell_advance_width,
            glyph_options[idx],
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
        let glyph_extent = i32::from(glyph.x_max) - i32::from(glyph.x_min);
        let right_side_bearing =
            i32::from(glyph.advance_width) - i32::from(glyph.left_side_bearing) - glyph_extent;
        min_right_side_bearing = min_right_side_bearing.min(right_side_bearing as i16);
        let extent = i32::from(glyph.left_side_bearing) + glyph_extent;
        x_max_extent = x_max_extent.max(checked_i16(extent, "xMaxExtent")?);

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
        vertical_metrics,
        advance_width_max,
        min_left_side_bearing,
        min_right_side_bearing,
        x_max_extent,
        num_glyphs,
    );
    let maxp = build_maxp_table(num_glyphs, max_points, max_contours);
    let loca_table = build_loca_table(&loca);
    let cmap = build_cmap_table(&mappings);
    let name = build_name_table(font_name, font_identity);
    let post = build_post_table();
    let os2 = build_os2_table(
        units_per_em,
        vertical_metrics,
        &mappings,
        advance_width_max,
        y_min,
        y_max,
    );

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

fn notdef_bitmap(width: u32, height: u32) -> GlyphBitmap {
    let mut pixels = vec![false; (width as usize).saturating_mul(height as usize)];
    let thickness = (height / 16).max(1);

    for y in 0..height {
        for x in 0..width {
            let border = x < thickness
                || y < thickness
                || x >= width.saturating_sub(thickness)
                || y >= height.saturating_sub(thickness);
            if border {
                let idx = y as usize * width as usize + x as usize;
                pixels[idx] = true;
            }
        }
    }

    GlyphBitmap {
        width,
        height,
        pixels,
    }
}

fn bitmap_glyph_to_ttf(
    bitmap: &GlyphBitmap,
    codepoint: Option<u32>,
    units_per_em: u16,
    vertical_metrics: FontVerticalMetrics,
    advance_width: u16,
    options: TtfGlyphOptions,
) -> Result<TtfGlyph> {
    if bitmap.width == 0 || bitmap.height == 0 {
        bail!("glyph bitmap size must be > 0 for TTF export");
    }
    let expected_len = (bitmap.width as usize).saturating_mul(bitmap.height as usize);
    if bitmap.pixels.len() != expected_len {
        bail!(
            "glyph bitmap pixel count mismatch: expected {}, got {}",
            expected_len,
            bitmap.pixels.len()
        );
    }

    let pixel_units = i16::try_from(u32::from(units_per_em) / bitmap.height)
        .context("invalid pixel scaling for TTF export")?;

    let mut points = Vec::new();
    let mut end_points = Vec::new();
    let mut x_min = i16::MAX;
    let mut y_min = i16::MAX;
    let mut x_max = i16::MIN;
    let mut y_max = i16::MIN;

    for y in 0..bitmap.height as usize {
        for x in 0..bitmap.width as usize {
            if !bitmap.pixels[y * bitmap.width as usize + x] {
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

    let x_shift = if options.center_in_line_box {
        center_shift(
            i32::from(advance_width),
            i32::from(x_min) + i32::from(x_max),
        )
    } else {
        0
    };
    let y_shift = if options.center_in_line_box {
        center_shift(
            i32::from(vertical_metrics.ascent) + i32::from(vertical_metrics.descent),
            i32::from(y_min) + i32::from(y_max),
        )
    } else {
        // Keep composition tiles in shared tile-local geometry, but align the
        // tile box to the font line box baseline.
        i32::from(vertical_metrics.descent)
    };
    if x_shift != 0 || y_shift != 0 {
        translate_points(&mut points, x_shift, y_shift)?;
        x_min = checked_i16(i32::from(x_min) + x_shift, "x_min after TTF centering")?;
        x_max = checked_i16(i32::from(x_max) + x_shift, "x_max after TTF centering")?;
        y_min = checked_i16(i32::from(y_min) + y_shift, "y_min after TTF centering")?;
        y_max = checked_i16(i32::from(y_max) + y_shift, "y_max after TTF centering")?;
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
        left_side_bearing: x_min,
        x_min,
        y_min,
        x_max,
        y_max,
        contour_count,
        point_count,
        data,
    })
}

fn center_shift(container_extent: i32, glyph_min_plus_max: i32) -> i32 {
    (container_extent - glyph_min_plus_max) / 2
}

fn translate_points(points: &mut [(i16, i16)], x_shift: i32, y_shift: i32) -> Result<()> {
    for (x, y) in points {
        *x = checked_i16(i32::from(*x) + x_shift, "x coordinate after TTF centering")?;
        *y = checked_i16(i32::from(*y) + y_shift, "y coordinate after TTF centering")?;
    }
    Ok(())
}

fn checked_i16(value: i32, context: &str) -> Result<i16> {
    i16::try_from(value).with_context(|| format!("{context} overflowed i16 range"))
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
    vertical_metrics: FontVerticalMetrics,
    advance_width_max: u16,
    min_left_side_bearing: i16,
    min_right_side_bearing: i16,
    x_max_extent: i16,
    number_of_h_metrics: u16,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(36);
    push_u32(&mut out, 0x0001_0000);
    push_i16(&mut out, vertical_metrics.ascent);
    push_i16(&mut out, vertical_metrics.descent);
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

fn build_name_table(font_name: &str, font_identity: &str) -> Vec<u8> {
    let family = font_name.trim();
    let family = if family.is_empty() {
        "Petiglyph"
    } else {
        family
    };
    let identity = font_identity.trim();
    let identity = if identity.is_empty() {
        "identity_missing"
    } else {
        identity
    };
    let postscript = postscript_name(family);
    let full_name = format!("{family} Regular");
    let unique = format!("{family};{};{identity}", env!("CARGO_PKG_VERSION"));

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

fn build_os2_table(
    units_per_em: u16,
    vertical_metrics: FontVerticalMetrics,
    mappings: &[(u32, u16)],
    advance_width: u16,
    y_min: i16,
    y_max: i16,
) -> Vec<u8> {
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
    push_u16(&mut out, 0x00C0);
    push_u16(&mut out, first_char);
    push_u16(&mut out, last_char);
    push_i16(&mut out, vertical_metrics.ascent);
    push_i16(&mut out, vertical_metrics.descent);
    push_i16(&mut out, 0);
    let win_ascent = i32::from(vertical_metrics.ascent)
        .max(i32::from(y_max))
        .max(0)
        .clamp(0, i32::from(u16::MAX)) as u16;
    let win_descent = i32::from(vertical_metrics.descent_abs())
        .max(-i32::from(y_min))
        .max(0)
        .clamp(0, i32::from(u16::MAX)) as u16;
    push_u16(&mut out, win_ascent);
    push_u16(&mut out, win_descent);
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

pub(crate) fn bitmap_to_bdf_rows(bitmap: &GlyphBitmap) -> String {
    let width = bitmap.width as usize;
    let height = bitmap.height as usize;
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
    unique_glyph_name_for_seed(&stem, used)
}

fn unique_glyph_name_for_seed(seed: &str, used: &mut HashSet<String>) -> String {
    let stem = slugify(seed);
    let stem = if stem.is_empty() {
        "glyph".to_string()
    } else {
        stem
    };
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

fn bdf_font_name(font_name: &str, glyph_width: u32, glyph_height: u32) -> String {
    let slug = slugify(font_name);
    let glyph_width = glyph_width.max(1);
    let glyph_height = glyph_height.max(1);
    let point_size = glyph_height.saturating_mul(10);
    let average_width = glyph_width.saturating_mul(10);
    if slug.is_empty() {
        format!(
            "-misc-petiglyph-medium-r-normal--{glyph_height}-{point_size}-75-75-c-{average_width}-iso10646-1"
        )
    } else {
        format!(
            "-misc-{slug}-medium-r-normal--{glyph_height}-{point_size}-75-75-c-{average_width}-iso10646-1"
        )
    }
}

pub(crate) fn glyph_sample_string_for_codepoints(codepoints: &[u32]) -> String {
    let mut out = String::new();
    for codepoint in codepoints {
        if let Some(ch) = char::from_u32(*codepoint) {
            out.push(ch);
            out.push(' ');
        }
    }
    out.trim_end().to_string()
}

fn generate_smart_sample(glyphs: &[PreprocessedGlyph], codepoints: &[u32]) -> String {
    if glyphs.len() != codepoints.len() {
        return glyph_sample_string_for_codepoints(codepoints);
    }

    let mut single_line = String::new();
    let mut blocks = Vec::new();
    let mut idx = 0usize;
    while idx < glyphs.len() {
        let glyph = &glyphs[idx];
        if let Some(tile) = &glyph.composition_tile {
            let span = tile.rows.saturating_mul(tile.cols);
            if span == 0 || idx + span > glyphs.len() {
                if let Some(ch) = char::from_u32(codepoints[idx]) {
                    if !single_line.is_empty() {
                        single_line.push(' ');
                    }
                    single_line.push(ch);
                }
                idx += 1;
                continue;
            }

            if !single_line.is_empty() {
                blocks.push(single_line.clone());
                single_line.clear();
            }

            let mut rows = vec![String::new(); tile.rows];
            for offset in 0..span {
                let child = &glyphs[idx + offset];
                let Some(child_tile) = &child.composition_tile else {
                    continue;
                };
                if child_tile.rows != tile.rows
                    || child_tile.cols != tile.cols
                    || child.source_parent_key != glyph.source_parent_key
                {
                    continue;
                }
                if let Some(ch) = char::from_u32(codepoints[idx + offset]) {
                    rows[child_tile.row].push(ch);
                }
            }
            blocks.push(rows.join("\n"));
            idx += span;
            continue;
        }

        if let Some(ch) = char::from_u32(codepoints[idx]) {
            if !single_line.is_empty() {
                single_line.push(' ');
            }
            single_line.push(ch);
        }
        idx += 1;
    }

    if !single_line.is_empty() {
        blocks.push(single_line);
    }

    blocks.join("\n\n")
}

#[allow(dead_code)]
pub(crate) fn glyph_sample_string(codepoint_start: u32, glyph_count: usize) -> String {
    let mut codepoints = Vec::with_capacity(glyph_count);
    for idx in 0..glyph_count {
        codepoints.push(codepoint_start + idx as u32);
    }
    glyph_sample_string_for_codepoints(&codepoints)
}
