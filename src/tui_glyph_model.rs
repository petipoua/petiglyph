fn sort_source_keys_for_animation_frames(keys: &mut [String]) {
    keys.sort_by(|left, right| {
        natural_source_key_cmp(left, right).then_with(|| {
            source_display_name(left)
                .to_ascii_lowercase()
                .cmp(&source_display_name(right).to_ascii_lowercase())
        })
    });
}

fn natural_source_key_cmp(left: &str, right: &str) -> std::cmp::Ordering {
    natural_ascii_cmp(
        &source_display_name(left).to_ascii_lowercase(),
        &source_display_name(right).to_ascii_lowercase(),
    )
}

fn natural_ascii_cmp(left: &str, right: &str) -> std::cmp::Ordering {
    let left = left.as_bytes();
    let right = right.as_bytes();
    let mut left_idx = 0usize;
    let mut right_idx = 0usize;

    while left_idx < left.len() && right_idx < right.len() {
        if left[left_idx].is_ascii_digit() && right[right_idx].is_ascii_digit() {
            let left_start = left_idx;
            let right_start = right_idx;
            while left_idx < left.len() && left[left_idx].is_ascii_digit() {
                left_idx += 1;
            }
            while right_idx < right.len() && right[right_idx].is_ascii_digit() {
                right_idx += 1;
            }

            let left_digits = trim_leading_ascii_zeroes(&left[left_start..left_idx]);
            let right_digits = trim_leading_ascii_zeroes(&right[right_start..right_idx]);
            let numeric_cmp = left_digits
                .len()
                .cmp(&right_digits.len())
                .then_with(|| left_digits.cmp(right_digits));
            if numeric_cmp != std::cmp::Ordering::Equal {
                return numeric_cmp;
            }

            let width_cmp = (left_idx - left_start).cmp(&(right_idx - right_start));
            if width_cmp != std::cmp::Ordering::Equal {
                return width_cmp;
            }
            continue;
        }

        let cmp = left[left_idx].cmp(&right[right_idx]);
        if cmp != std::cmp::Ordering::Equal {
            return cmp;
        }
        left_idx += 1;
        right_idx += 1;
    }

    left.len().cmp(&right.len())
}

fn trim_leading_ascii_zeroes(digits: &[u8]) -> &[u8] {
    let mut idx = 0usize;
    while idx + 1 < digits.len() && digits[idx] == b'0' {
        idx += 1;
    }
    &digits[idx..]
}

fn animation_frame_parent_source(source_key: &str) -> String {
    parse_compose_tile_key(source_key)
        .map(|(parent, _, _, _, _)| parent.to_string())
        .unwrap_or_else(|| source_key.to_string())
}

fn animation_frame_parent_sources(config: &RuntimeConfig) -> BTreeSet<String> {
    config
        .animations
        .iter()
        .flat_map(|animation| animation.frames.iter())
        .map(|frame| animation_frame_parent_source(frame))
        .collect()
}

fn glyph_matches_animation_row_frame(
    glyph: &InteractiveGlyph,
    animation: &AnimationDef,
    frame_source_key: &str,
) -> bool {
    if animation.animation_type == AnimationType::Standard
        && !frame_source_key.contains("#compose:")
    {
        return glyph.glyph.source_key == frame_source_key
            && glyph.glyph.composition_tile.is_none();
    }
    glyph_matches_animation_frame_source(glyph, frame_source_key)
}

fn animation_frame_source_for_preview(
    selected_row: Option<&VisibleGlyphRow>,
    animation: &AnimationDef,
    preview: Option<&AnimationPreview>,
) -> Option<String> {
    if let Some(VisibleGlyphRow::AnimationFrame { source_key, .. }) = selected_row {
        return Some(source_key.clone());
    }

    let preview = preview?;
    if preview.animation_name != animation.name {
        return None;
    }
    animation
        .frames
        .get(
            preview
                .frame_index
                .min(animation.frames.len().saturating_sub(1)),
        )
        .cloned()
}

fn inactive_runtime_config(workspace_root: &Path) -> RuntimeConfig {
    RuntimeConfig {
        project_dir: workspace_root.to_path_buf(),
        project_id: "inactive-workspace".to_string(),
        input_dir: workspace_root.join("images"),
        out_dir: workspace_root.join("build"),
        font_name: "No active project".to_string(),
        glyph_size: 64,
        base_threshold: 64,
        threshold_overrides: Default::default(),
        invert_overrides: Default::default(),
        compositions: Default::default(),
        animations: Vec::new(),
        codepoint_start: 0x10_0000,
    }
}

pub(crate) fn switch_notice_visible(started_at: Instant, now: Instant) -> bool {
    now.duration_since(started_at) < Duration::from_millis(SWITCH_NOTICE_MS)
}

fn load_project_switch_task(
    manifest_path: PathBuf,
    launch_overrides: TuiLaunchOverrides,
) -> Result<ProjectSwitchTaskOutput> {
    let config = load_runtime_config(
        &manifest_path,
        launch_overrides.input_dir,
        None,
        launch_overrides.threshold,
        launch_overrides.glyph_size,
        launch_overrides.codepoint_start,
    )?;
    let loaded = load_interactive_glyphs_from_config(&config)?;
    let (last_build, last_sample) = cached_build_state(&config);
    let installed_font_path =
        cached_installed_font_path(&manifest_path, &config.font_name, &config.project_id);

    Ok(ProjectSwitchTaskOutput {
        manifest_path,
        config,
        loaded,
        last_build,
        last_sample,
        installed_font_path,
    })
}

fn build_and_install(
    manifest_path: PathBuf,
    launch_overrides: TuiLaunchOverrides,
) -> Result<InstallTaskOutput> {
    let config = load_runtime_config(
        &manifest_path,
        launch_overrides.input_dir,
        None,
        launch_overrides.threshold,
        launch_overrides.glyph_size,
        launch_overrides.codepoint_start,
    )?;
    if config.glyph_size == 0 {
        bail!("glyph_size must be > 0");
    }
    let install_font_name =
        effective_font_name(&manifest_path, &config.font_name, DEFAULT_INSTALL_NAME_MODE)?;

    let summary = build_outputs(&config)?;
    let sample = fs::read_to_string(&summary.sample_path)
        .with_context(|| format!("failed to read {}", summary.sample_path.display()))?;
    let installed = install_built_font(
        &manifest_path,
        &install_font_name,
        &config.project_id,
        &summary.ttf_path,
        summary.glyph_count,
    )?;
    let sample = sample.trim_end().to_string();
    let sample = if sample.is_empty() {
        None
    } else {
        Some(sample)
    };

    Ok(InstallTaskOutput::Install {
        summary: Box::new(summary),
        sample,
        installed_path: installed.install_path,
        first_install_on_machine: installed.first_install_on_machine,
    })
}

fn uninstall_installed_font_task(
    installed_ttf: PathBuf,
    file_name: String,
) -> Result<InstallTaskOutput> {
    let result = uninstall_installed_font_file(&installed_ttf)?;
    let status_message = match result.outcome {
        crate::install::UninstallOutcome::Removed => format!("uninstalled {file_name}"),
        crate::install::UninstallOutcome::AlreadyAbsent => {
            format!("font already absent: {file_name}")
        }
    };
    Ok(InstallTaskOutput::Uninstall { status_message })
}

fn cached_build_state(config: &RuntimeConfig) -> (Option<BuildSummary>, Option<String>) {
    let ttf_path = expected_ttf_path(config);
    let bdf_path = expected_bdf_path(config);
    if !ttf_path.is_file() || !bdf_path.is_file() {
        return (None, None);
    }

    let mapping_path = config.out_dir.join("glyph-map.json");
    let sample_path = config.out_dir.join("glyph-sample.txt");
    let previews_dir = config.out_dir.join("previews");

    let glyph_count = fs::read_to_string(&mapping_path)
        .ok()
        .and_then(|raw| serde_json::from_str::<Vec<MappingEntry>>(&raw).ok())
        .map_or(0, |entries| entries.len());

    let sample = fs::read_to_string(&sample_path)
        .ok()
        .map(|raw| raw.trim_end().to_string())
        .filter(|value| !value.is_empty());

    (
        Some(BuildSummary {
            glyph_count,
            bdf_path,
            ttf_path,
            mapping_path,
            sample_path,
            previews_dir,
        }),
        sample,
    )
}

fn cached_installed_font_path(
    manifest_path: &Path,
    font_name: &str,
    project_id: &str,
) -> Option<PathBuf> {
    resolve_installed_font_path_with(manifest_path, font_name, Some(project_id), |path| {
        path.is_file()
    })
}

pub(crate) fn resolve_installed_font_path_with<F>(
    manifest_path: &Path,
    font_name: &str,
    project_id: Option<&str>,
    mut is_installed: F,
) -> Option<PathBuf>
where
    F: FnMut(&Path) -> bool,
{
    let mut candidates = Vec::new();
    if let Ok(paths) =
        installed_ttf_candidates_for_manifest_font(manifest_path, font_name, project_id)
    {
        for path in paths {
            if !candidates.contains(&path) {
                candidates.push(path);
            }
        }
    }
    if let Ok(path) =
        expected_install_ttf_path_for_mode(manifest_path, font_name, DEFAULT_INSTALL_NAME_MODE)
        && !candidates.contains(&path)
    {
        candidates.push(path);
    }
    if let Ok(path) =
        expected_install_ttf_path_for_mode(manifest_path, font_name, FontInstallNameMode::Plain)
        && !candidates.contains(&path)
    {
        candidates.push(path);
    }

    candidates.into_iter().find(|path| is_installed(path))
}

pub(crate) fn persist_threshold_override(
    manifest_path: &Path,
    source_key: &str,
    threshold: Option<u8>,
) -> Result<()> {
    let mut manifest = read_manifest(manifest_path)?;
    match threshold {
        Some(value) => {
            manifest
                .threshold_overrides
                .insert(source_key.to_string(), value);
        }
        None => {
            manifest.threshold_overrides.remove(source_key);
        }
    }
    write_manifest(manifest_path, &manifest)
}

pub(crate) fn persist_invert_override(
    manifest_path: &Path,
    source_key: &str,
    invert: bool,
) -> Result<()> {
    let mut manifest = read_manifest(manifest_path)?;
    if invert {
        manifest
            .invert_overrides
            .insert(source_key.to_string(), true);
    } else {
        manifest.invert_overrides.remove(source_key);
    }
    write_manifest(manifest_path, &manifest)
}

fn persist_composition_definition(
    manifest_path: &Path,
    source_key: &str,
    composition: Option<CompositionDef>,
) -> Result<()> {
    let mut manifest = read_manifest(manifest_path)?;
    match composition {
        Some(def) => {
            manifest.compositions.insert(source_key.to_string(), def);
        }
        None => {
            manifest.compositions.remove(source_key);
        }
    }
    write_manifest(manifest_path, &manifest)
}

fn persist_animation_definition(manifest_path: &Path, animation: AnimationDef) -> Result<()> {
    let mut manifest = read_manifest(manifest_path)?;
    manifest.animations.push(animation);
    write_manifest(manifest_path, &manifest)
}

fn remove_animation_definition(manifest_path: &Path, animation_name: &str) -> Result<bool> {
    let mut manifest = read_manifest(manifest_path)?;
    let original_len = manifest.animations.len();
    manifest.animations.retain(|a| a.name != animation_name);
    let removed = manifest.animations.len() != original_len;
    if removed {
        write_manifest(manifest_path, &manifest)?;
    }
    Ok(removed)
}

fn persist_animation_fps(manifest_path: &Path, animation_name: &str, fps: u8) -> Result<bool> {
    let mut manifest = read_manifest(manifest_path)?;
    let Some(animation) = manifest
        .animations
        .iter_mut()
        .find(|animation| animation.name == animation_name)
    else {
        return Ok(false);
    };
    animation.fps = fps.clamp(1, 30);
    write_manifest(manifest_path, &manifest)?;
    Ok(true)
}

fn default_animation_name_from_frames(config: &RuntimeConfig, frames: &[String]) -> String {
    let base = frames
        .first()
        .map(|frame| animation_name_base_from_frame(frame))
        .filter(|base| !base.is_empty())
        .unwrap_or_else(|| "animation".to_string());
    let stem = format!("{base}_anim");
    let existing = config
        .animations
        .iter()
        .map(|a| a.name.as_str())
        .collect::<BTreeSet<_>>();
    if !existing.contains(stem.as_str()) {
        return stem;
    }
    for idx in 1..=9999 {
        let candidate = format!("{stem}_{idx}");
        if !existing.contains(candidate.as_str()) {
            return candidate;
        }
    }
    stem
}

fn animation_name_base_from_frame(frame: &str) -> String {
    let display_name = source_display_name(frame);
    let stem = Path::new(&display_name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(display_name.as_str());
    let without_separators = stem.replace(['-', '_'], "");
    let trimmed_digits = without_separators.trim_end_matches(|c: char| c.is_ascii_digit());
    let slug = slugify(trimmed_digits);
    if slug.is_empty() {
        "animation".to_string()
    } else {
        slug
    }
}

fn selected_source_parent_key(app: &App) -> Option<String> {
    let row = app.selected_visible_row()?;
    match row {
        VisibleGlyphRow::AnimationFrame { source_key, .. } => Some(source_key),
        VisibleGlyphRow::AnimationParent { .. } => None,
        VisibleGlyphRow::Single { glyph_idx } => app
            .glyphs
            .get(glyph_idx)
            .map(|g| g.glyph.source_parent_key.clone()),
        VisibleGlyphRow::CompositionParent { source_key, .. }
        | VisibleGlyphRow::CompositionChild { source_key, .. } => Some(source_key),
    }
}

fn selected_animation_index(app: &App) -> Option<usize> {
    match app.selected_visible_row()? {
        VisibleGlyphRow::AnimationParent { animation_idx }
        | VisibleGlyphRow::AnimationFrame { animation_idx, .. } => Some(animation_idx),
        _ => None,
    }
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
    let Some(file_name) = source_path.file_name() else {
        bail!("invalid source file path for {source_key}");
    };
    let duplicate_path = next_incremental_duplicate_destination(input_dir, Path::new(file_name))?;
    fs::copy(&source_path, &duplicate_path).with_context(|| {
        format!(
            "failed to duplicate source {} for grid conflict resolution",
            source_path.display()
        )
    })?;
    Ok(source_key_from_input_path(input_dir, &duplicate_path))
}

fn duplicate_selected_parent_source_for_grid(app: &mut App, source_key: &str) -> Result<String> {
    let Some(source_path) = app
        .glyphs
        .iter()
        .find(|g| g.glyph.source_parent_key == source_key)
        .map(|g| g.glyph.source_path.clone())
    else {
        anyhow::bail!("unable to locate source path for {source_key}");
    };
    let Some(file_name) = source_path.file_name() else {
        anyhow::bail!("invalid source file path for {source_key}");
    };

    let duplicate_path =
        next_incremental_duplicate_destination(&app.config.input_dir, Path::new(file_name))?;
    fs::copy(&source_path, &duplicate_path).with_context(|| {
        format!(
            "failed to duplicate source {} -> {}",
            source_path.display(),
            duplicate_path.display()
        )
    })?;
    Ok(source_key_from_input_path(
        &app.config.input_dir,
        &duplicate_path,
    ))
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
    for entry in fs::read_dir(input_dir)
        .with_context(|| format!("failed to scan {}", input_dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
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

fn apply_default_composition_to_selected(app: &mut App) -> Result<()> {
    if app.active_project.is_none() {
        app.status = Some(
            "create a project in Home or relaunch with --manifest before composing".to_string(),
        );
        return Ok(());
    }

    let Some(source_key) = selected_source_parent_key(app) else {
        app.status = Some("no glyph selected".to_string());
        return Ok(());
    };
    if app.config.compositions.contains_key(&source_key) {
        app.status = Some(format!(
            "composition already exists for {source_key}; press C to remove it first"
        ));
        return Ok(());
    }

    persist_composition_definition(
        &app.manifest_path,
        &source_key,
        Some(CompositionDef {
            rows: 2,
            cols: 2,
            horizontal_bleed: BleedLevel::Weak,
            vertical_bleed: BleedLevel::Off,
        }),
    )?;
    app.reload_glyphs()?;
    app.expanded_compositions.insert(source_key.clone());
    app.clamp_glyph_selection();
    app.status = Some(format!(
        "created composition for {source_key}: 2x2 (edit [compositions] in petiglyph.toml for custom sizes)"
    ));
    Ok(())
}

fn clear_selected_composition(app: &mut App) -> Result<()> {
    if app.active_project.is_none() {
        app.status = Some(
            "create a project in Home or relaunch with --manifest before composing".to_string(),
        );
        return Ok(());
    }
    let Some(source_key) = selected_source_parent_key(app) else {
        app.status = Some("no glyph selected".to_string());
        return Ok(());
    };
    if !app.config.compositions.contains_key(&source_key) {
        app.status = Some(format!("no composition configured for {source_key}"));
        return Ok(());
    }

    persist_composition_definition(&app.manifest_path, &source_key, None)?;
    app.expanded_compositions.remove(&source_key);
    app.reload_glyphs()?;
    app.status = Some(format!("removed composition for {source_key}"));
    Ok(())
}

fn selected_visible_glyph_index(app: &App) -> Option<usize> {
    match app.selected_visible_row()? {
        VisibleGlyphRow::AnimationFrame { glyph_idx, .. } => glyph_idx,
        VisibleGlyphRow::AnimationParent { .. } => None,
        VisibleGlyphRow::Single { glyph_idx }
        | VisibleGlyphRow::CompositionChild { glyph_idx, .. } => Some(glyph_idx),
        VisibleGlyphRow::CompositionParent { .. } => None,
    }
}

fn selected_threshold_sources(app: &App) -> Option<Vec<String>> {
    match app.selected_visible_row()? {
        VisibleGlyphRow::AnimationParent { animation_idx } => app
            .config
            .animations
            .get(animation_idx)
            .map(animation_threshold_parent_sources),
        VisibleGlyphRow::AnimationFrame { source_key, .. } => {
            Some(vec![animation_frame_parent_source(&source_key)])
        }
        VisibleGlyphRow::CompositionChild { .. } => None,
        _ => selected_source_parent_key(app).map(|source| vec![source]),
    }
}

fn animation_threshold_parent_sources(animation: &AnimationDef) -> Vec<String> {
    animation
        .frames
        .iter()
        .map(|frame| animation_frame_parent_source(frame))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>()
}

fn animation_has_non_uniform_frame_thresholds(app: &App, animation: &AnimationDef) -> bool {
    let mut values = animation_threshold_parent_sources(animation)
        .into_iter()
        .filter_map(|source_key| {
            app.glyphs
                .iter()
                .find(|g| g.glyph.source_parent_key == source_key)
                .map(|g| g.working_threshold)
        });
    let Some(first) = values.next() else {
        return false;
    };
    values.any(|value| value != first)
}

fn animation_uniform_frame_threshold(app: &App, animation: &AnimationDef) -> Option<u8> {
    let mut values = animation_threshold_parent_sources(animation)
        .into_iter()
        .map(|source_key| {
            app.glyphs
                .iter()
                .find(|g| g.glyph.source_parent_key == source_key)
                .map(|g| g.working_threshold)
                .or_else(|| {
                    app.config
                        .threshold_overrides
                        .get(&source_key)
                        .copied()
                        .or(Some(app.config.base_threshold))
                })
        })
        .collect::<Vec<_>>();
    if values.is_empty() {
        return None;
    }
    let first = values.remove(0)?;
    if values.into_iter().all(|v| v == Some(first)) {
        Some(first)
    } else {
        None
    }
}

fn animation_threshold_summary_label(app: &App, animation: &AnimationDef) -> String {
    animation_uniform_frame_threshold(app, animation)
        .map(|value| value.to_string())
        .unwrap_or_else(|| "var. threshold".to_string())
}

fn grayscale_summary_from_processing(
    processing: Option<animation_media::AnimationImportProcessingOptions>,
) -> String {
    match processing {
        Some(processing) if !processing.grayscale_enabled => "gray OFF".to_string(),
        Some(processing) => format!(
            "gray ON B{:+} C{:+} G{:.2}",
            processing.grayscale.brightness,
            processing.grayscale.contrast,
            processing.grayscale.gamma_percent as f32 / 100.0
        ),
        None => "gray n/a".to_string(),
    }
}

fn animation_grayscale_summary_label(animation: &AnimationDef) -> String {
    grayscale_summary_from_processing(animation.grayscale_processing)
}

fn installed_animation_threshold_summary_label(
    uniform_threshold: Option<u8>,
    variable: bool,
) -> String {
    if variable {
        "var. threshold".to_string()
    } else {
        uniform_threshold
            .map(|value| value.to_string())
            .unwrap_or_else(|| "n/a".to_string())
    }
}

fn selected_invert_sources(app: &App) -> Option<Vec<String>> {
    selected_threshold_sources(app)
}

fn selected_row_supports_invert(app: &App) -> bool {
    selected_invert_sources(app).is_some()
}

fn selected_row_invert_value(app: &App) -> Option<bool> {
    let sources = selected_invert_sources(app)?;
    let first = sources.first()?;
    app.glyphs
        .iter()
        .find(|glyph| glyph.glyph.source_parent_key == *first)
        .map(|glyph| glyph.working_invert)
        .or(Some(false))
}

fn animation_has_non_uniform_frame_invert(app: &App, animation: &AnimationDef) -> bool {
    let mut values = animation_threshold_parent_sources(animation)
        .into_iter()
        .filter_map(|source_key| {
            app.glyphs
                .iter()
                .find(|g| g.glyph.source_parent_key == source_key)
                .map(|g| g.working_invert)
        });
    let Some(first) = values.next() else {
        return false;
    };
    values.any(|value| value != first)
}

fn selected_glyph(app: &App) -> Option<&InteractiveGlyph> {
    let idx = selected_visible_glyph_index(app)?;
    app.glyphs.get(idx)
}

fn set_selected_threshold(app: &mut App, threshold: u8) {
    if app.active_project.is_none() {
        app.status = Some(
            "create a project in Home or relaunch with --manifest before tuning glyphs".to_string(),
        );
        return;
    }

    let Some(sources) = selected_threshold_sources(app) else {
        app.status = Some("no glyph selected".to_string());
        return;
    };
    let threshold_override = if threshold == app.config.base_threshold {
        None
    } else {
        Some(threshold)
    };
    for source_key in &sources {
        if let Err(err) =
            persist_threshold_override(&app.manifest_path, source_key, threshold_override)
        {
            app.status = Some(format!("failed to save override for {source_key}: {err}"));
            return;
        }
    }
    let source_set = sources.iter().cloned().collect::<BTreeSet<_>>();
    for glyph in &mut app.glyphs {
        if source_set.contains(&glyph.glyph.source_parent_key) {
            glyph.working_threshold = threshold;
            glyph.saved_threshold = threshold_override;
        }
    }
    for source_key in sources {
        match threshold_override {
            Some(value) => {
                app.config.threshold_overrides.insert(source_key, value);
            }
            None => {
                app.config.threshold_overrides.remove(&source_key);
            }
        }
    }
    app.status = Some(match threshold_override {
        Some(value) => format!("saved threshold override: {value}"),
        None => format!(
            "cleared threshold override(s): now using base threshold {}",
            app.config.base_threshold
        ),
    });
}

fn remove_selected_threshold_override(app: &mut App) {
    if app.active_project.is_none() {
        app.status = Some(
            "create a project in Home or relaunch with --manifest before tuning glyphs".to_string(),
        );
        return;
    }

    let Some(sources) = selected_threshold_sources(app) else {
        app.status = Some("no glyph selected".to_string());
        return;
    };
    for source_key in &sources {
        if let Err(err) = persist_threshold_override(&app.manifest_path, source_key, None) {
            app.status = Some(format!("failed to remove override for {source_key}: {err}"));
            return;
        }
    }
    let base_threshold = app.config.base_threshold;
    let source_set = sources.iter().cloned().collect::<BTreeSet<_>>();
    for glyph in &mut app.glyphs {
        if source_set.contains(&glyph.glyph.source_parent_key) {
            glyph.saved_threshold = None;
            glyph.working_threshold = base_threshold;
        }
    }
    for source_key in sources {
        app.config.threshold_overrides.remove(&source_key);
    }
    app.status = Some(format!(
        "removed threshold override(s): now using base threshold {}",
        base_threshold
    ));
}

fn set_selected_invert(app: &mut App, invert: bool) {
    if app.active_project.is_none() {
        app.status = Some(
            "create a project in Home or relaunch with --manifest before tuning glyphs".to_string(),
        );
        return;
    }

    let Some(sources) = selected_invert_sources(app) else {
        app.status = Some("no glyph selected".to_string());
        return;
    };

    for source_key in &sources {
        if let Err(err) = persist_invert_override(&app.manifest_path, source_key, invert) {
            app.status = Some(format!(
                "failed to save invert override for {source_key}: {err}"
            ));
            return;
        }
    }

    let source_set = sources.iter().cloned().collect::<BTreeSet<_>>();
    for glyph in &mut app.glyphs {
        if source_set.contains(&glyph.glyph.source_parent_key) {
            glyph.saved_invert = invert;
            glyph.working_invert = invert;
        }
    }

    for source_key in sources {
        if invert {
            app.config.invert_overrides.insert(source_key, true);
        } else {
            app.config.invert_overrides.remove(&source_key);
        }
    }

    app.status = Some(if invert {
        "saved invert override: on".to_string()
    } else {
        "cleared invert override(s): normal colors".to_string()
    });
}

fn toggle_selected_invert(app: &mut App) {
    let Some(current) = selected_row_invert_value(app) else {
        app.status = Some("no glyph selected".to_string());
        return;
    };
    set_selected_invert(app, !current);
}
