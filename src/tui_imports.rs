#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WindowsCreationWorkflowPickerConfig {
    title: &'static str,
    filter: &'static str,
    multiselect: bool,
}

fn windows_creation_workflow_uses_picker_for_os(os: &str) -> bool {
    os == "windows"
}

fn windows_creation_workflow_uses_picker() -> bool {
    windows_creation_workflow_uses_picker_for_os(env::consts::OS)
}

fn creation_workflow_import_area_label(
    animation_media_mode: bool,
    windows_picker_mode: bool,
) -> &'static str {
    match (windows_picker_mode, animation_media_mode) {
        (true, true) => "PICK/PASTE MEDIA HERE",
        (true, false) => "PICK/PASTE IMAGES HERE",
        (false, true) => "DRAG/PASTE MEDIA HERE",
        (false, false) => "DRAG/PASTE IMAGES HERE",
    }
}

fn creation_workflow_import_fallback_label(
    animation_media_mode: bool,
    windows_picker_mode: bool,
) -> &'static str {
    match (windows_picker_mode, animation_media_mode) {
        (true, true) => " Pick media files here with the Windows file picker.",
        (true, false) => " Pick image files here with the Windows file picker.",
        (false, true) => " Drop, paste, or drag media files here.",
        (false, false) => " Drop, paste, or drag image files here.",
    }
}

fn home_workflow_import_hint_for_os(kind: HomeCreationKind, os: &str) -> &'static str {
    if windows_creation_workflow_uses_picker_for_os(os) {
        return match kind {
            HomeCreationKind::Glyph | HomeCreationKind::Grid => {
                "pick files in the Windows file picker for this popup."
            }
            HomeCreationKind::AnimatedGlyph | HomeCreationKind::AnimatedGridGlyph => {
                "pick images/GIFs/videos in the Windows file picker for this popup."
            }
        };
    }

    match kind {
        HomeCreationKind::AnimatedGlyph | HomeCreationKind::AnimatedGridGlyph => {
            "drop, paste, or drag images/GIFs/videos in this popup."
        }
        HomeCreationKind::Glyph | HomeCreationKind::Grid => {
            "drop, paste, or drag files in this popup."
        }
    }
}

fn home_workflow_import_hint(kind: HomeCreationKind) -> &'static str {
    home_workflow_import_hint_for_os(kind, env::consts::OS)
}

fn home_import_missing_sources_message_for_os(kind: HomeCreationKind, os: &str) -> &'static str {
    if windows_creation_workflow_uses_picker_for_os(os) {
        return match kind {
            HomeCreationKind::Glyph => {
                "pick at least one source image in the Windows file picker, then press Enter"
            }
            HomeCreationKind::Grid => {
                "create grid: pick exactly one image in the Windows file picker, then press Enter"
            }
            HomeCreationKind::AnimatedGlyph | HomeCreationKind::AnimatedGridGlyph => {
                "pick at least one frame media file in the Windows file picker, then press Enter"
            }
        };
    }

    match kind {
        HomeCreationKind::Glyph => "drop at least one source image in the popup, then press Enter",
        HomeCreationKind::Grid => {
            "create grid: drop exactly one image in the popup, then press Enter"
        }
        HomeCreationKind::AnimatedGlyph | HomeCreationKind::AnimatedGridGlyph => {
            "drop at least one frame media file in the popup, then press Enter"
        }
    }
}

fn home_import_missing_sources_message(kind: HomeCreationKind) -> &'static str {
    home_import_missing_sources_message_for_os(kind, env::consts::OS)
}

fn import_step_enter_help_for_os(os: &str) -> &'static str {
    if windows_creation_workflow_uses_picker_for_os(os) {
        "open file picker / continue after import"
    } else {
        "go to tweaking step after import"
    }
}

fn import_step_enter_help() -> &'static str {
    import_step_enter_help_for_os(env::consts::OS)
}

fn windows_creation_workflow_picker_config(
    kind: HomeCreationKind,
) -> WindowsCreationWorkflowPickerConfig {
    match kind {
        HomeCreationKind::Glyph => WindowsCreationWorkflowPickerConfig {
            title: "Pick source images for petiglyph",
            filter: "Supported images|*.png;*.jpg;*.jpeg;*.webp;*.bmp;*.gif;*.svg;*.avif|All files|*.*",
            multiselect: true,
        },
        HomeCreationKind::Grid => WindowsCreationWorkflowPickerConfig {
            title: "Pick one source image for the petiglyph grid",
            filter: "Supported images|*.png;*.jpg;*.jpeg;*.webp;*.bmp;*.gif;*.svg;*.avif|All files|*.*",
            multiselect: false,
        },
        HomeCreationKind::AnimatedGlyph => WindowsCreationWorkflowPickerConfig {
            title: "Pick animation media for petiglyph",
            filter: "Supported media|*.png;*.jpg;*.jpeg;*.webp;*.bmp;*.svg;*.avif;*.gif;*.mp4;*.mov;*.mkv;*.webm;*.avi;*.m4v|All files|*.*",
            multiselect: true,
        },
        HomeCreationKind::AnimatedGridGlyph => WindowsCreationWorkflowPickerConfig {
            title: "Pick animation media for the petiglyph animated grid",
            filter: "Supported media|*.png;*.jpg;*.jpeg;*.webp;*.bmp;*.svg;*.avif;*.gif;*.mp4;*.mov;*.mkv;*.webm;*.avi;*.m4v|All files|*.*",
            multiselect: true,
        },
    }
}

fn powershell_single_quoted_string(value: &str) -> String {
    value.replace('\'', "''")
}

fn windows_creation_workflow_picker_script(config: WindowsCreationWorkflowPickerConfig) -> String {
    let title = powershell_single_quoted_string(config.title);
    let filter = powershell_single_quoted_string(config.filter);
    let multiselect = if config.multiselect { "$true" } else { "$false" };
    format!(
        "Add-Type -AssemblyName System.Windows.Forms | Out-Null\n\
$dialog = New-Object System.Windows.Forms.OpenFileDialog\n\
$dialog.Title = '{title}'\n\
$dialog.Filter = '{filter}'\n\
$dialog.Multiselect = {multiselect}\n\
$dialog.CheckFileExists = $true\n\
$dialog.CheckPathExists = $true\n\
$dialog.RestoreDirectory = $true\n\
if ($dialog.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) {{\n\
  $dialog.FileNames | ForEach-Object {{ [Console]::Out.WriteLine($_) }}\n\
}}\n"
    )
}

fn open_windows_creation_workflow_picker(kind: HomeCreationKind) -> Result<Option<String>> {
    if !windows_creation_workflow_uses_picker() {
        return Ok(None);
    }

    let script =
        windows_creation_workflow_picker_script(windows_creation_workflow_picker_config(kind));
    let mut attempted = Vec::new();
    let mut errors = Vec::new();

    for command in ["powershell.exe", "pwsh.exe", "powershell", "pwsh"] {
        let Some(resolved) = resolve_command_path(command) else {
            continue;
        };
        attempted.push(command);
        let output = std::process::Command::new(&resolved)
            .args(["-NoProfile", "-NonInteractive", "-STA", "-Command", &script])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .with_context(|| format!("failed to launch {command} for Windows file picker"))?;
        if output.status.success() {
            let payload = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if payload.is_empty() {
                return Ok(None);
            }
            return Ok(Some(payload));
        }

        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if stderr.is_empty() {
            errors.push(format!("{command} exited with status {}", output.status));
        } else {
            errors.push(format!("{command}: {stderr}"));
        }
    }

    if attempted.is_empty() {
        bail!("Windows file picker requires `powershell` or `pwsh` on PATH");
    }

    bail!("Windows file picker failed: {}", errors.join(" | "))
}

fn looks_like_path_payload(payload: &str) -> bool {
    let trimmed = payload.trim();
    if trimmed.is_empty() {
        return false;
    }
    trimmed.contains('/') || trimmed.starts_with("file://") || trimmed.contains('\\')
}

fn count_supported_sources(input_dir: &Path) -> Result<usize> {
    if !input_dir.exists() {
        return Ok(0);
    }

    let mut count = 0usize;
    for entry in WalkDir::new(input_dir).follow_links(true) {
        let entry =
            entry.with_context(|| format!("failed while scanning {}", input_dir.display()))?;
        if entry.file_type().is_file() && is_supported_source(entry.path()) {
            count += 1;
        }
    }

    Ok(count)
}

fn glyph_source_fingerprint(input_dir: &Path) -> Result<u64> {
    if !input_dir.exists() {
        return Ok(0);
    }

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for entry in WalkDir::new(input_dir).follow_links(true) {
        let entry =
            entry.with_context(|| format!("failed while scanning {}", input_dir.display()))?;
        if !entry.file_type().is_file() || !is_supported_source(entry.path()) {
            continue;
        }

        entry.path().hash(&mut hasher);
        let metadata = entry
            .metadata()
            .with_context(|| format!("failed to read metadata for {}", entry.path().display()))?;
        metadata.len().hash(&mut hasher);
        let modified = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|value| value.as_nanos())
            .unwrap_or_default();
        modified.hash(&mut hasher);
    }

    Ok(hasher.finish())
}

fn collect_dropped_paths(payload: &str) -> Vec<PathBuf> {
    let mut normalized = payload.replace("\r\n", "\n").replace('\r', "\n");
    if normalized.contains("file://") {
        normalized = normalized.replace("file://", "\nfile://");
    }
    let mut fragments = Vec::new();
    for line in normalized.lines() {
        let line = line.trim();
        if !line.is_empty() {
            fragments.push(line.to_string());
        }
    }

    let mut seen = BTreeSet::new();
    let mut out = Vec::new();

    for fragment in fragments {
        let mut candidates = vec![fragment.clone()];
        candidates.extend(split_shell_like_tokens(&fragment));

        for candidate in candidates {
            let Some(path) = normalize_dropped_path_candidate(&candidate) else {
                continue;
            };
            let key = path.to_string_lossy().to_string();
            if seen.insert(key) {
                out.push(path);
            }
        }
    }

    out
}

fn should_apply_static_import_grayscale(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            matches!(
                ext.to_ascii_lowercase().as_str(),
                "png" | "jpg" | "jpeg" | "bmp"
            )
        })
}

fn split_shell_like_tokens(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut escaped = false;
    let mut in_single_quote = false;
    let mut in_double_quote = false;

    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }

        if ch == '\\' && !in_single_quote {
            match chars.peek().copied() {
                Some(' ') | Some('\t') | Some('"') | Some('\'') | Some('\\') => {
                    escaped = true;
                    continue;
                }
                _ => {
                    current.push(ch);
                    continue;
                }
            }
        }

        if ch == '\'' && !in_double_quote {
            in_single_quote = !in_single_quote;
            continue;
        }

        if ch == '"' && !in_single_quote {
            in_double_quote = !in_double_quote;
            continue;
        }

        if ch.is_whitespace() && !in_single_quote && !in_double_quote {
            if !current.is_empty() {
                tokens.push(current.clone());
                current.clear();
            }
            continue;
        }

        current.push(ch);
    }

    if escaped {
        current.push('\\');
    }

    if !current.is_empty() {
        tokens.push(current);
    }

    tokens
}

fn normalize_dropped_path_candidate(candidate: &str) -> Option<PathBuf> {
    let trimmed = candidate.trim().trim_end_matches('\0');
    if trimmed.is_empty() {
        return None;
    }

    let stripped = strip_wrapping_quotes(trimmed);
    if let Some(uri_path) = stripped.strip_prefix("file://") {
        return Some(PathBuf::from(decode_file_uri_path(uri_path)));
    }

    Some(PathBuf::from(unescape_backslashes(stripped)))
}

fn strip_wrapping_quotes(value: &str) -> &str {
    if value.len() >= 2 {
        let bytes = value.as_bytes();
        let starts = bytes[0];
        let ends = bytes[value.len() - 1];
        if (starts == b'"' && ends == b'"') || (starts == b'\'' && ends == b'\'') {
            return &value[1..value.len() - 1];
        }
    }
    value
}

fn decode_file_uri_path(uri_path: &str) -> String {
    let mut path = uri_path;
    if let Some(rest) = path.strip_prefix("localhost") {
        path = rest;
    }
    percent_decode(path)
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0usize;

    while index < bytes.len() {
        if bytes[index] == b'%'
            && index + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (hex_value(bytes[index + 1]), hex_value(bytes[index + 2]))
        {
            out.push((hi << 4) | lo);
            index += 3;
            continue;
        }

        out.push(bytes[index]);
        index += 1;
    }

    String::from_utf8_lossy(&out).into_owned()
}

fn hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn unescape_backslashes(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }

        match chars.peek().copied() {
            Some(' ') | Some('\t') | Some('"') | Some('\'') | Some('\\') => {
                out.push(chars.next().expect("peeked a char"));
            }
            Some(next) => {
                out.push('\\');
                out.push(next);
                chars.next();
            }
            None => out.push('\\'),
        }
    }
    out
}

fn paths_resolve_to_same_file(left: &Path, right: &Path) -> bool {
    let Ok(left) = fs::canonicalize(left) else {
        return false;
    };
    let Ok(right) = fs::canonicalize(right) else {
        return false;
    };
    left == right
}

fn next_available_import_destination(
    input_dir: &Path,
    file_name: &std::ffi::OsStr,
) -> (PathBuf, bool) {
    let candidate = input_dir.join(file_name);
    if !candidate.exists() {
        return (candidate, false);
    }

    let stem = Path::new(file_name)
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("glyph");
    let ext = Path::new(file_name)
        .extension()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty());

    for index in 1.. {
        let renamed = match ext {
            Some(ext) => format!("{stem}-{index}.{ext}"),
            None => format!("{stem}-{index}"),
        };
        let next = input_dir.join(renamed);
        if !next.exists() {
            return (next, true);
        }
    }

    (candidate, false)
}

fn files_have_same_contents(left: &Path, right: &Path) -> bool {
    let Ok(left_meta) = fs::metadata(left) else {
        return false;
    };
    let Ok(right_meta) = fs::metadata(right) else {
        return false;
    };
    if left_meta.len() != right_meta.len() {
        return false;
    }

    fs::read(left)
        .ok()
        .zip(fs::read(right).ok())
        .is_some_and(|(left, right)| left == right)
}

fn format_drop_import_status(
    imported: usize,
    renamed: usize,
    skipped_existing: usize,
    skipped_unsupported: usize,
    skipped_missing: usize,
) -> String {
    format!(
        "drop import: {imported} added, {renamed} renamed, {skipped_existing} already present, {skipped_unsupported} unsupported, {skipped_missing} missing"
    )
}

fn format_animation_media_import_status(
    imported: usize,
    renamed: usize,
    skipped_existing: usize,
    skipped_unsupported: usize,
    skipped_missing: usize,
    media_files_processed: usize,
    frames_extracted: usize,
) -> String {
    format!(
        "animation media import: {media_files_processed} media processed, {frames_extracted} extracted frames, {imported} added, {renamed} renamed, {skipped_existing} already present, {skipped_unsupported} unsupported, {skipped_missing} missing"
    )
}

fn import_image_files_to_input(
    input_dir: &Path,
    payload: &str,
    existing_policy: ExistingImportPolicy,
    processing: animation_media::AnimationImportProcessingOptions,
) -> Result<DropImportResult> {
    fs::create_dir_all(input_dir)
        .with_context(|| format!("failed to create {}", input_dir.display()))?;

    let dropped_paths = collect_dropped_paths(payload);
    if dropped_paths.is_empty() {
        bail!("drop did not include readable file paths");
    }

    let mut imported = 0usize;
    let mut renamed = 0usize;
    let mut skipped_existing = 0usize;
    let mut skipped_unsupported = 0usize;
    let mut skipped_missing = 0usize;
    let mut imported_source_keys = Vec::new();
    let mut created_source_keys = Vec::new();

    for source in dropped_paths {
        if !source.is_file() {
            skipped_missing += 1;
            continue;
        }

        if !is_supported_source(&source) && !animation_media::is_avif_image(&source) {
            skipped_unsupported += 1;
            continue;
        }

        let Some(file_name) = animation_media::static_import_file_name(&source) else {
            skipped_missing += 1;
            continue;
        };

        let canonical_destination = input_dir.join(&file_name);
        if paths_resolve_to_same_file(&source, &canonical_destination) {
            imported_source_keys.push(source_key_from_input_path(
                input_dir,
                &canonical_destination,
            ));
            skipped_existing += 1;
            continue;
        }

        if existing_policy == ExistingImportPolicy::ReuseIdentical
            && canonical_destination.exists()
            && files_have_same_contents(&source, &canonical_destination)
        {
            imported_source_keys.push(source_key_from_input_path(
                input_dir,
                &canonical_destination,
            ));
            skipped_existing += 1;
            continue;
        }

        let (destination, was_renamed) =
            next_available_import_destination(input_dir, file_name.as_os_str());
        if animation_media::is_avif_image(&source) {
            animation_media::convert_avif_image_to_png(&source, &destination)?;
        } else {
            fs::copy(&source, &destination).with_context(|| {
                format!(
                    "failed to import {} into {}",
                    source.display(),
                    destination.display()
                )
            })?;
        }
        if processing.grayscale_enabled && should_apply_static_import_grayscale(&destination) {
            let _ = animation_media::apply_grayscale_processing_to_image_file(
                &destination,
                processing.grayscale,
            );
        }

        imported_source_keys.push(source_key_from_input_path(input_dir, &destination));
        created_source_keys.push(source_key_from_input_path(input_dir, &destination));
        imported += 1;
        if was_renamed {
            renamed += 1;
        }
    }

    Ok(DropImportResult {
        imported,
        renamed,
        skipped_existing,
        skipped_unsupported,
        skipped_missing,
        imported_source_keys,
        created_source_keys,
    })
}

fn load_interactive_glyphs_from_config(config: &RuntimeConfig) -> Result<LoadedGlyphs> {
    let mut sources = Vec::new();
    for entry in WalkDir::new(&config.input_dir).follow_links(true) {
        let entry = entry
            .with_context(|| format!("failed while scanning {}", config.input_dir.display()))?;
        if entry.file_type().is_file() && is_supported_source(entry.path()) {
            sources.push(entry.path().to_path_buf());
        }
    }
    sources.sort();

    let glyphs = preprocess_sources_for_config(&sources, config)?
    .into_iter()
    .map(|glyph| {
        let saved_threshold = config
            .threshold_overrides
            .get(&glyph.source_parent_key)
            .copied();
        let working_threshold = saved_threshold.unwrap_or(config.base_threshold);
        let saved_invert = config
            .invert_overrides
            .get(&glyph.source_parent_key)
            .copied()
            .unwrap_or(false);
        InteractiveGlyph {
            glyph,
            saved_threshold,
            working_threshold,
            saved_invert,
            working_invert: saved_invert,
        }
    })
    .collect::<Vec<_>>();

    Ok(LoadedGlyphs {
        glyphs,
        source_fingerprint: glyph_source_fingerprint(&config.input_dir)?,
    })
}

fn load_interactive_glyphs_for_source_keys(
    config: &RuntimeConfig,
    source_keys: &[String],
) -> Result<Vec<InteractiveGlyph>> {
    let mut seen = BTreeSet::new();
    let mut sources = Vec::new();
    for source_key in source_keys {
        if !seen.insert(source_key.clone()) {
            continue;
        }
        let source_path = config.input_dir.join(source_key);
        if source_path.is_file() && is_supported_source(&source_path) {
            sources.push(source_path);
        }
    }
    sources.sort();

    let glyphs = preprocess_sources_for_config(&sources, config)?
    .into_iter()
    .map(|glyph| {
        let saved_threshold = config
            .threshold_overrides
            .get(&glyph.source_parent_key)
            .copied();
        let working_threshold = saved_threshold.unwrap_or(config.base_threshold);
        let saved_invert = config
            .invert_overrides
            .get(&glyph.source_parent_key)
            .copied()
            .unwrap_or(false);
        InteractiveGlyph {
            glyph,
            saved_threshold,
            working_threshold,
            saved_invert,
            working_invert: saved_invert,
        }
    })
    .collect::<Vec<_>>();

    Ok(glyphs)
}
