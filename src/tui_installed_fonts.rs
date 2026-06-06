fn scan_projects_in_folder(folder: &Path) -> Result<Vec<WelcomeProject>> {
    discover_project_manifests(folder)?
        .into_iter()
        .map(|manifest_path| match read_manifest(&manifest_path) {
            Ok(manifest) => Ok(WelcomeProject {
                manifest_path,
                font_name: manifest.font_name,
                manifest_warning: None,
            }),
            Err(_) => {
                let fallback_name = manifest_path
                    .parent()
                    .and_then(|path| path.file_name())
                    .and_then(|name| name.to_str())
                    .map(|name| name.replace(['-', '_'], " "))
                    .filter(|name| !name.trim().is_empty())
                    .unwrap_or_else(|| "Unknown project".to_string());
                Ok(WelcomeProject {
                    manifest_path,
                    font_name: fallback_name,
                    manifest_warning: Some("malformed manifest".to_string()),
                })
            }
        })
        .collect()
}

pub(crate) fn scan_installed_petiglyph_fonts(cwd: &Path) -> Result<Vec<InstalledFontSample>> {
    let manifest_probe = cwd.join("petiglyph.toml");
    let install_dir = install_dir_for_manifest(&manifest_probe)?;
    if !install_dir.is_dir() {
        return Ok(Vec::new());
    }

    let mut ttf_paths = Vec::new();
    for metadata_path in crate::install::metadata_paths_in_install_dir(&install_dir)? {
        let raw = match fs::read_to_string(&metadata_path) {
            Ok(raw) => raw,
            Err(_) => continue,
        };
        let metadata = match serde_json::from_str::<crate::install::InstalledFontMetadata>(&raw) {
            Ok(metadata) => metadata,
            Err(_) => continue,
        };
        let path = PathBuf::from(metadata.installed_ttf);
        if path.is_file() {
            ttf_paths.push(path);
        }
    }
    ttf_paths.sort();

    let mut samples = Vec::new();
    for path in ttf_paths {
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("unknown.ttf")
            .to_string();

        let metadata_sample = sample_from_installed_font_metadata(&install_dir, &path).ok();
        let (raw_blocks, animation_rows, animation_previews, animation_exports) =
            match metadata_sample {
                Some(InstalledFontMetadataSample::Matched(payload)) => payload,
                Some(InstalledFontMetadataSample::MissingSampleForMatchedMetadata) | _ => {
                    let (sample, truncated) = fs::read(&path)
                        .ok()
                        .and_then(|bytes| {
                            sample_glyphs_from_ttf_bytes(&bytes, WELCOME_SAMPLE_LIMIT)
                        })
                        .unwrap_or_default();
                    let _ = truncated;
                    (
                        installed_font_blocks_without_metadata(vec![sample]),
                        Vec::new(),
                        Vec::new(),
                        Vec::new(),
                    )
                }
            };
        let blocks = regroup_installed_sample_blocks(raw_blocks);
        if blocks.is_empty() && animation_rows.is_empty() {
            continue;
        }

        samples.push(InstalledFontSample {
            file_name,
            path,
            blocks,
            animation_rows,
            animation_previews,
            animation_exports,
        });
    }

    Ok(samples)
}

fn sample_from_installed_font_metadata(
    install_dir: &Path,
    installed_ttf: &Path,
) -> Result<InstalledFontMetadataSample> {
    let installed_canonical = installed_ttf.canonicalize().ok();
    let mut metadata_candidates = Vec::new();
    let mut matched_metadata = false;

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
        metadata_candidates.push(path);
    }

    for metadata_path in metadata_candidates {
        let raw = match fs::read_to_string(&metadata_path) {
            Ok(raw) => raw,
            Err(_) => continue,
        };
        let metadata = match serde_json::from_str::<InstalledFontMetadataRecord>(&raw) {
            Ok(metadata) => metadata,
            Err(_) => continue,
        };
        let metadata_ttf = PathBuf::from(metadata.installed_ttf);
        let ttf_matches = metadata_ttf == installed_ttf
            || installed_canonical
                .as_deref()
                .zip(metadata_ttf.canonicalize().ok().as_deref())
                .is_some_and(|(left, right)| left == right);
        if !ttf_matches {
            continue;
        }
        matched_metadata = true;
        if let Some(sample) = sample_from_manifest_path(Path::new(&metadata.manifest_path)) {
            let mut animation_rows = Vec::new();
            let mut animation_previews = Vec::new();
            let mut animation_exports = Vec::new();
            let manifest_path = Path::new(&metadata.manifest_path);
            let resolved_animation_blocks = installed_animation_blocks_from_manifest(manifest_path);
            let static_block_details = installed_static_block_details_from_manifest(manifest_path);

            let mut all_animation_frames = HashSet::new();
            for snapshot in metadata.animation_snapshots {
                let type_label = match snapshot.animation_type {
                    AnimationType::Standard => "standard",
                    AnimationType::Grid => "grid",
                };
                let frame_blocks = resolved_animation_blocks
                    .get(&snapshot.name)
                    .filter(|blocks| !blocks.is_empty())
                    .cloned()
                    .unwrap_or(snapshot.frame_blocks);
                let grayscale_label =
                    grayscale_summary_from_processing(snapshot.grayscale_processing);
                let threshold_label = installed_animation_threshold_summary_label(
                    snapshot.uniform_threshold,
                    snapshot.variable_threshold,
                );

                for frame in &frame_blocks {
                    all_animation_frames.insert(frame.trim().to_string());
                }

                animation_rows.push(format!(
                    "Animation: {} ({}, {} fps, {} frames, {}, th {})",
                    snapshot.name,
                    type_label,
                    snapshot.fps,
                    frame_blocks.len(),
                    grayscale_label,
                    threshold_label,
                ));
                animation_previews.push(InstalledFontAnimationPreview {
                    fps: snapshot.fps,
                    frame_blocks: frame_blocks.clone(),
                });
                let mut export = format!(
                    "name: {}\ntype: {}\nfps: {}\ngrayscale: {}\nthreshold: {}\n",
                    snapshot.name, type_label, snapshot.fps, grayscale_label, threshold_label
                );
                if !frame_blocks.is_empty() {
                    export.push('\n');
                    export.push_str(&frame_blocks.join("\n\n"));
                }
                animation_exports.push(export);
            }

            let sample = prune_static_sample_blocks(sample, &all_animation_frames);
            let sample = installed_font_blocks_with_details(sample, &static_block_details);

            return Ok(InstalledFontMetadataSample::Matched((
                sample,
                animation_rows,
                animation_previews,
                animation_exports,
            )));
        }
    }

    if matched_metadata {
        Ok(InstalledFontMetadataSample::MissingSampleForMatchedMetadata)
    } else {
        Ok(InstalledFontMetadataSample::NoMetadataMatch)
    }
}

fn sample_from_manifest_path(manifest_path: &Path) -> Option<Vec<String>> {
    let manifest = read_manifest(manifest_path).ok()?;
    let project_dir = manifest_path.parent().unwrap_or_else(|| Path::new("."));
    let sample_path = project_dir.join(manifest.out_dir).join("glyph-sample.txt");
    let sample = fs::read_to_string(sample_path).ok()?;
    let sample = sample.trim_end().to_string();
    if sample.is_empty() {
        None
    } else {
        Some(
            sample
                .split("\n\n")
                .map(|s| s.to_string())
                .collect::<Vec<_>>(),
        )
    }
}

#[derive(Debug, Clone)]
struct InstalledStaticBlockDetails {
    entries_by_char: BTreeMap<char, InstalledStaticGlyphEntry>,
}

#[derive(Debug, Clone)]
struct InstalledStaticGlyphEntry {
    glyph_name: String,
    source_file: String,
    threshold: u8,
}

fn installed_static_block_details_from_manifest(
    manifest_path: &Path,
) -> Option<InstalledStaticBlockDetails> {
    let manifest = read_manifest(manifest_path).ok()?;
    let project_dir = manifest_path.parent().unwrap_or_else(|| Path::new("."));
    let mapping_path = project_dir.join(&manifest.out_dir).join("glyph-map.json");
    let mapping_raw = fs::read_to_string(mapping_path).ok()?;
    let mappings: Vec<MappingEntry> = serde_json::from_str(&mapping_raw).ok()?;
    let entries_by_char = mappings
        .into_iter()
        .filter_map(|entry| {
            let ch = format_codepoint_char(&entry.codepoint)?;
            let threshold_source = animation_frame_parent_source(&entry.source_file);
            let threshold = manifest
                .threshold_overrides
                .get(&threshold_source)
                .copied()
                .unwrap_or(manifest.threshold);
            Some((
                ch,
                InstalledStaticGlyphEntry {
                    glyph_name: entry.glyph_name,
                    source_file: entry.source_file,
                    threshold,
                },
            ))
        })
        .collect();

    Some(InstalledStaticBlockDetails { entries_by_char })
}

fn installed_font_blocks_without_metadata(blocks: Vec<String>) -> Vec<InstalledFontBlock> {
    blocks
        .into_iter()
        .map(|block| {
            let glyph_count = block.chars().filter(|ch| !ch.is_whitespace()).count();
            let type_label = if block.contains('\n') {
                "grid"
            } else {
                "standard"
            };
            let label = format!(
                "Glyphs: unknown ({type_label}, {glyph_count} glyph{}, gray n/a, th n/a)",
                if glyph_count == 1 { "" } else { "s" }
            );
            InstalledFontBlock {
                label,
                export: block.clone(),
                block,
            }
        })
        .collect()
}

fn installed_font_blocks_with_details(
    blocks: Vec<String>,
    details: &Option<InstalledStaticBlockDetails>,
) -> Vec<InstalledFontBlock> {
    blocks
        .into_iter()
        .map(|block| {
            details
                .as_ref()
                .and_then(|details| installed_font_block_with_details(block.clone(), details))
                .unwrap_or_else(|| {
                    installed_font_blocks_without_metadata(vec![block])
                        .into_iter()
                        .next()
                        .expect("one fallback block")
                })
        })
        .collect()
}

fn installed_font_block_with_details(
    block: String,
    details: &InstalledStaticBlockDetails,
) -> Option<InstalledFontBlock> {
    let entries = block
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .filter_map(|ch| details.entries_by_char.get(&ch))
        .collect::<Vec<_>>();
    if entries.is_empty() {
        return None;
    }

    let thresholds = entries
        .iter()
        .map(|entry| entry.threshold)
        .collect::<BTreeSet<_>>();
    let threshold_label = if thresholds.len() == 1 {
        thresholds
            .first()
            .map(|value| value.to_string())
            .unwrap_or_else(|| "n/a".to_string())
    } else {
        "var. threshold".to_string()
    };

    let (title, kind_details) = if block.contains('\n') {
        let first_source = entries
            .first()
            .map(|entry| entry.source_file.as_str())
            .unwrap_or("grid");
        let (name, rows, cols) = parse_compose_tile_key(first_source)
            .map(|(parent, rows, cols, _, _)| (parent.to_string(), rows, cols))
            .unwrap_or_else(|| (first_source.to_string(), block.lines().count().max(1), 1));
        (format!("Grid: {name}"), format!("grid, {rows}x{cols}"))
    } else {
        let names = entries
            .iter()
            .map(|entry| entry.glyph_name.as_str())
            .collect::<Vec<_>>();
        let title = if names.len() == 1 {
            format!("Glyph: {}", names[0])
        } else {
            format!("Glyphs: {}", compact_name_list(&names, 3))
        };
        (
            title,
            format!(
                "standard, {} glyph{}",
                entries.len(),
                if entries.len() == 1 { "" } else { "s" }
            ),
        )
    };
    let label = format!("{title} ({kind_details}, gray n/a, th {threshold_label})");
    let export = format!("{label}\n\n{block}");

    Some(InstalledFontBlock {
        label,
        block,
        export,
    })
}

fn compact_name_list(names: &[&str], max_visible: usize) -> String {
    let visible = names
        .iter()
        .take(max_visible)
        .copied()
        .collect::<Vec<_>>()
        .join(", ");
    if names.len() > max_visible {
        format!("{visible}, +{} more", names.len() - max_visible)
    } else {
        visible
    }
}

fn prune_static_sample_blocks(
    sample_blocks: Vec<String>,
    animation_frame_blocks: &HashSet<String>,
) -> Vec<String> {
    let mut animation_chars = HashSet::new();
    for block in animation_frame_blocks {
        for ch in block.chars().filter(|ch| !ch.is_whitespace()) {
            animation_chars.insert(ch);
        }
    }

    sample_blocks
        .into_iter()
        .filter_map(|block| {
            if animation_frame_blocks.contains(block.trim()) {
                return None;
            }
            let filtered = block
                .chars()
                .filter(|ch| ch.is_whitespace() || !animation_chars.contains(ch))
                .collect::<String>();
            if filtered.trim().is_empty() {
                None
            } else {
                Some(filtered)
            }
        })
        .collect()
}

fn installed_animation_blocks_from_manifest(manifest_path: &Path) -> BTreeMap<String, Vec<String>> {
    let manifest = match read_manifest(manifest_path) {
        Ok(manifest) => manifest,
        Err(_) => return BTreeMap::new(),
    };
    let project_dir = manifest_path.parent().unwrap_or_else(|| Path::new("."));
    let mapping_path = project_dir.join(&manifest.out_dir).join("glyph-map.json");
    let mapping_raw = match fs::read_to_string(mapping_path) {
        Ok(raw) => raw,
        Err(_) => return BTreeMap::new(),
    };
    let mappings: Vec<MappingEntry> = match serde_json::from_str(&mapping_raw) {
        Ok(mappings) => mappings,
        Err(_) => return BTreeMap::new(),
    };
    let by_source = mappings
        .into_iter()
        .map(|entry| (entry.source_file, entry.codepoint))
        .collect::<BTreeMap<_, _>>();

    manifest
        .animations
        .into_iter()
        .map(|animation| {
            let blocks = installed_animation_blocks_for_definition(&animation, &by_source);
            (animation.name, blocks)
        })
        .collect()
}

fn installed_animation_blocks_for_definition(
    animation: &AnimationDef,
    by_source: &BTreeMap<String, String>,
) -> Vec<String> {
    animation
        .frames
        .iter()
        .map(|frame| match animation.animation_type {
            AnimationType::Standard => installed_animation_source_block(by_source, frame)
                .unwrap_or_else(|| format!("[missing:{frame}]")),
            AnimationType::Grid => {
                let rows = animation.rows.unwrap_or(1);
                let cols = emitted_composition_cols(animation.cols.unwrap_or(1));
                (0..rows)
                    .map(|row| {
                        (0..cols)
                            .map(|col| {
                                let key = format!("{frame}#compose:{rows}x{cols}:{row}:{col}");
                                installed_animation_source_block(by_source, &key)
                                    .and_then(|block| block.chars().next())
                                    .unwrap_or(' ')
                            })
                            .collect::<String>()
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            }
        })
        .collect()
}

fn emitted_composition_cols(logical_cols: usize) -> usize {
    logical_cols.checked_mul(2).unwrap_or(logical_cols)
}

fn remap_standard_source_key_unambiguous<'a>(
    existing_keys: impl Iterator<Item = &'a String>,
    source_key: &str,
) -> Option<String> {
    if source_key.contains("#compose:") {
        return None;
    }
    let mut matched = existing_keys.filter(|candidate| {
        if candidate.contains("#compose:") {
            return false;
        }
        candidate.as_str() == source_key
            || candidate.ends_with(&format!("/{source_key}"))
            || source_key.ends_with(&format!("/{candidate}"))
    });
    let first = matched.next()?;
    if matched.next().is_some() {
        return None; // Ambiguous
    }
    Some(first.clone())
}

fn installed_animation_source_block(
    by_source: &BTreeMap<String, String>,
    source_key: &str,
) -> Option<String> {
    if let Some(codepoint) = by_source.get(source_key) {
        return format_codepoint_char(codepoint).map(|c| c.to_string());
    }
    let codepoint = by_source
        .get(source_key)
        .cloned()
        .or_else(|| {
            remap_compose_source_key_unambiguous(by_source.keys(), source_key)
                .and_then(|resolved| by_source.get(&resolved).cloned())
        })
        .or_else(|| {
            remap_standard_source_key_unambiguous(by_source.keys(), source_key)
                .and_then(|resolved| by_source.get(&resolved).cloned())
        });
    codepoint
        .as_deref()
        .and_then(|cp| format_codepoint_char(cp))
        .map(|c| c.to_string())
}

fn format_codepoint_char(codepoint: &str) -> Option<char> {
    let raw = codepoint.strip_prefix("U+").unwrap_or(codepoint);
    u32::from_str_radix(raw, 16).ok().and_then(char::from_u32)
}

fn glyph_matches_animation_frame_source(glyph: &InteractiveGlyph, frame_source_key: &str) -> bool {
    if glyph.glyph.source_key == frame_source_key
        || glyph.glyph.source_parent_key == frame_source_key
    {
        return true;
    }
    let Some((frame_parent, frame_rows, frame_cols, frame_row, frame_col)) =
        parse_compose_tile_key(frame_source_key)
    else {
        return false;
    };
    let Some((glyph_parent, glyph_rows, glyph_cols, glyph_row, glyph_col)) =
        parse_compose_tile_key(&glyph.glyph.source_key)
    else {
        return false;
    };
    glyph_parent == frame_parent
        && glyph_rows == frame_rows
        && glyph_cols == frame_cols
        && glyph_row == frame_row
        && glyph_col == frame_col
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

fn remap_compose_source_key_unambiguous<'a>(
    existing_keys: impl Iterator<Item = &'a String>,
    source_key: &str,
) -> Option<String> {
    let (parent, rows, cols, row, col) = parse_compose_tile_key(source_key)?;
    let mut matched = existing_keys.filter_map(|candidate| {
        let (candidate_parent, candidate_rows, candidate_cols, candidate_row, candidate_col) =
            parse_compose_tile_key(candidate)?;
        let parent_matches = candidate_parent == parent
            || candidate_parent.ends_with(&format!("/{parent}"))
            || parent.ends_with(&format!("/{candidate_parent}"));

        if parent_matches
            && candidate_rows == rows
            && candidate_cols == cols
            && candidate_row == row
            && candidate_col == col
        {
            Some(candidate.clone())
        } else {
            None
        }
    });
    let first = matched.next()?;
    if matched.next().is_some() {
        return None; // Ambiguous
    }
    Some(first)
}

pub(crate) fn regroup_installed_sample_blocks(
    blocks: Vec<InstalledFontBlock>,
) -> Vec<InstalledFontBlock> {
    let mut standard_blocks = Vec::new();
    let mut grid_blocks = Vec::new();

    for block in blocks {
        let normalized = block.block.trim().to_string();
        if normalized.is_empty() {
            continue;
        }
        if normalized.contains('\n') {
            grid_blocks.push(InstalledFontBlock {
                block: normalized,
                ..block
            });
        } else {
            standard_blocks.push(InstalledFontBlock {
                block: normalized,
                ..block
            });
        }
    }

    let mut grouped = Vec::new();
    if !standard_blocks.is_empty() {
        let block = expand_standard_sample_cells(
            &standard_blocks
                .iter()
                .map(|block| block.block.as_str())
                .collect::<Vec<_>>()
                .join(" "),
        );
        let label = if standard_blocks.len() == 1 {
            standard_blocks[0].label.clone()
        } else {
            let glyph_count = block.chars().filter(|ch| !ch.is_whitespace()).count();
            let thresholds = standard_blocks
                .iter()
                .filter_map(|block| block.label.rsplit_once("th ").map(|(_, th)| th))
                .collect::<BTreeSet<_>>();
            let threshold_label = if thresholds.len() == 1 {
                thresholds
                    .first()
                    .copied()
                    .unwrap_or("n/a")
                    .trim_end_matches(')')
                    .to_string()
            } else {
                "var. threshold".to_string()
            };
            format!(
                "Glyphs: mixed (standard, {glyph_count} glyph{}, gray n/a, th {threshold_label})",
                if glyph_count == 1 { "" } else { "s" }
            )
        };
        grouped.push(InstalledFontBlock {
            label: label.clone(),
            export: format!("{label}\n\n{block}"),
            block,
        });
    }
    grouped.extend(grid_blocks);
    grouped
}

fn expand_standard_sample_cells(sample: &str) -> String {
    let mut out = String::with_capacity(sample.len() * 2);
    for ch in sample.chars() {
        if ch.is_whitespace() {
            continue;
        }
        out.push(ch);
        out.push_str("   ");
    }
    out.trim_end().to_string()
}

pub(crate) fn sample_glyphs_from_ttf_bytes(bytes: &[u8], limit: usize) -> Option<(String, bool)> {
    if limit == 0 {
        return None;
    }

    let face = ttf_parser::Face::parse(bytes, 0).ok()?;
    let cmap = face.tables().cmap?;
    let mut codepoints = BTreeSet::new();
    let mut truncated = false;

    for subtable in cmap.subtables {
        if !subtable.is_unicode() {
            continue;
        }

        subtable.codepoints(|codepoint| {
            if codepoint <= 0x20 || codepoint > 0x10_FFFF || (0xD800..=0xDFFF).contains(&codepoint)
            {
                return;
            }
            let Some(ch) = char::from_u32(codepoint) else {
                return;
            };
            let Some(glyph_id) = face.glyph_index(ch) else {
                return;
            };
            if face.glyph_bounding_box(glyph_id).is_none() {
                return;
            }

            if codepoints.contains(&codepoint) {
                return;
            }

            if codepoints.len() < limit {
                codepoints.insert(codepoint);
            } else {
                truncated = true;
            }
        });

        if codepoints.len() >= limit && truncated {
            break;
        }
    }

    let sample = codepoints
        .into_iter()
        .filter_map(char::from_u32)
        .collect::<String>();
    if sample.is_empty() {
        None
    } else {
        Some((sample, truncated))
    }
}
