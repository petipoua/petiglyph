use anyhow::{Context, Result, bail};
use image::codecs::gif::GifDecoder;
use image::{AnimationDecoder, DynamicImage, ImageFormat, RgbImage};
use std::ffi::OsStr;
use std::fs;
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::image_pipeline::load_raster_image;

#[derive(Debug, Clone)]
pub(crate) struct AnimationMediaImportResult {
    pub(crate) imported: usize,
    pub(crate) renamed: usize,
    pub(crate) skipped_existing: usize,
    pub(crate) skipped_unsupported: usize,
    pub(crate) skipped_missing: usize,
    pub(crate) imported_source_keys: Vec<String>,
    pub(crate) created_source_keys: Vec<String>,
    pub(crate) media_files_processed: usize,
    pub(crate) frames_extracted: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct AnimationGrayscaleOptions {
    pub(crate) brightness: i16,
    pub(crate) contrast: i16,
    pub(crate) gamma_percent: u16,
}

impl Default for AnimationGrayscaleOptions {
    fn default() -> Self {
        Self {
            brightness: 0,
            contrast: 0,
            gamma_percent: 100,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct AnimationImportProcessingOptions {
    pub(crate) grayscale_enabled: bool,
    pub(crate) grayscale: AnimationGrayscaleOptions,
}

impl Default for AnimationImportProcessingOptions {
    fn default() -> Self {
        Self {
            grayscale_enabled: true,
            grayscale: AnimationGrayscaleOptions::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExistingImportPolicy {
    ReuseIdentical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AnimationInputKind {
    StillImage,
    Gif,
    Video,
    Unsupported,
}

const MAX_FRAMES_PER_MEDIA_INPUT: usize = 1200;
const MAX_TOTAL_EXTRACTED_FRAMES_PER_IMPORT: usize = 3000;

struct TempExtractDir(PathBuf);

impl TempExtractDir {
    fn new(tag: &str) -> Result<Self> {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should be monotonic")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("petiglyph-frame-extract-{tag}-{nonce}"));
        fs::create_dir_all(&dir).with_context(|| format!("failed to create {}", dir.display()))?;
        Ok(Self(dir))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempExtractDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub(crate) fn import_animation_media_to_input(
    input_dir: &Path,
    payload: &str,
    existing_policy: ExistingImportPolicy,
    processing: AnimationImportProcessingOptions,
) -> Result<AnimationMediaImportResult> {
    fs::create_dir_all(input_dir)
        .with_context(|| format!("failed to create {}", input_dir.display()))?;

    let dropped_paths = collect_dropped_paths(payload);
    if dropped_paths.is_empty() {
        bail!("drop did not include readable file paths");
    }

    let mut result = AnimationMediaImportResult {
        imported: 0,
        renamed: 0,
        skipped_existing: 0,
        skipped_unsupported: 0,
        skipped_missing: 0,
        imported_source_keys: Vec::new(),
        created_source_keys: Vec::new(),
        media_files_processed: 0,
        frames_extracted: 0,
    };

    for source in dropped_paths {
        if !source.is_file() {
            result.skipped_missing += 1;
            continue;
        }

        match classify_input_kind(&source) {
            AnimationInputKind::Unsupported => {
                result.skipped_unsupported += 1;
            }
            AnimationInputKind::StillImage => {
                import_one_file(input_dir, &source, None, existing_policy, &mut result, true)?;
            }
            AnimationInputKind::Gif => {
                result.media_files_processed += 1;
                let remaining_total =
                    MAX_TOTAL_EXTRACTED_FRAMES_PER_IMPORT.saturating_sub(result.frames_extracted);
                if remaining_total == 0 {
                    bail!(
                        "drop exceeded total extracted frame limit ({MAX_TOTAL_EXTRACTED_FRAMES_PER_IMPORT})"
                    );
                }
                let cap = MAX_FRAMES_PER_MEDIA_INPUT.min(remaining_total);
                let (_temp_dir, temp_paths) = expand_gif_frames_to_temp_pngs(&source, cap)?;
                import_expanded_frames(
                    input_dir,
                    &source,
                    &temp_paths,
                    existing_policy,
                    processing,
                    &mut result,
                )?;
            }
            AnimationInputKind::Video => {
                result.media_files_processed += 1;
                let remaining_total =
                    MAX_TOTAL_EXTRACTED_FRAMES_PER_IMPORT.saturating_sub(result.frames_extracted);
                if remaining_total == 0 {
                    bail!(
                        "drop exceeded total extracted frame limit ({MAX_TOTAL_EXTRACTED_FRAMES_PER_IMPORT})"
                    );
                }
                let cap = MAX_FRAMES_PER_MEDIA_INPUT.min(remaining_total);
                let (_temp_dir, temp_paths) = expand_video_frames_to_temp_pngs(&source, cap)?;
                import_expanded_frames(
                    input_dir,
                    &source,
                    &temp_paths,
                    existing_policy,
                    processing,
                    &mut result,
                )?;
            }
        }
    }

    Ok(result)
}

fn import_expanded_frames(
    input_dir: &Path,
    source_media_path: &Path,
    temp_frame_paths: &[PathBuf],
    existing_policy: ExistingImportPolicy,
    processing: AnimationImportProcessingOptions,
    result: &mut AnimationMediaImportResult,
) -> Result<()> {
    if temp_frame_paths.is_empty() {
        bail!(
            "{} had zero extractable frames",
            source_media_path.to_string_lossy()
        );
    }

    let media_hash = media_identity_hash_hex8(source_media_path)?;
    let stem = slug_stem(source_media_path);

    for (idx, frame_path) in temp_frame_paths.iter().enumerate() {
        if result.frames_extracted >= MAX_TOTAL_EXTRACTED_FRAMES_PER_IMPORT {
            bail!(
                "drop exceeded total extracted frame limit ({MAX_TOTAL_EXTRACTED_FRAMES_PER_IMPORT})"
            );
        }
        if processing.grayscale_enabled {
            apply_grayscale_processing_to_image_file(frame_path, processing.grayscale)?;
        }

        let deterministic_name = format!("{stem}--pgf-{media_hash}-f{:06}.png", idx + 1);
        import_one_file(
            input_dir,
            frame_path,
            Some(OsStr::new(&deterministic_name)),
            existing_policy,
            result,
            false,
        )?;
        result.frames_extracted += 1;
    }

    Ok(())
}

pub(crate) fn is_avif_image(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("avif"))
}

pub(crate) fn static_import_file_name(source: &Path) -> Option<PathBuf> {
    if !is_avif_image(source) {
        return source.file_name().map(PathBuf::from);
    }

    let stem = source
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("glyph");
    Some(PathBuf::from(format!("{stem}.png")))
}

pub(crate) fn convert_avif_image_to_png(source: &Path, destination: &Path) -> Result<()> {
    let Some(ffmpeg_path) = resolve_command_path("ffmpeg") else {
        bail!("ffmpeg not found; install ffmpeg to import AVIF images");
    };

    let temp_path = temp_png_import_path(destination);
    let output = Command::new(&ffmpeg_path)
        .arg("-v")
        .arg("error")
        .arg("-y")
        .arg("-i")
        .arg(source)
        .arg("-frames:v")
        .arg("1")
        .arg(&temp_path)
        .output()
        .with_context(|| format!("failed to run ffmpeg for {}", source.display()))?;

    if !output.status.success() {
        let _ = fs::remove_file(&temp_path);
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "AVIF conversion failed for {}: {}",
            source.display(),
            stderr.trim()
        );
    }

    fs::rename(&temp_path, destination).with_context(|| {
        let _ = fs::remove_file(&temp_path);
        format!(
            "failed to place converted AVIF at {}",
            destination.display()
        )
    })?;
    Ok(())
}

fn temp_png_import_path(destination: &Path) -> PathBuf {
    let file_name = destination
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("image.png");
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or_default();
    destination.with_file_name(format!(
        ".{file_name}.petiglyph-avif-{}-{nonce}.tmp.png",
        std::process::id()
    ))
}

pub(crate) fn apply_grayscale_processing_to_image_file(
    frame_path: &Path,
    options: AnimationGrayscaleOptions,
) -> Result<()> {
    let (image, format) = load_raster_image(frame_path)
        .with_context(|| format!("failed to decode extracted frame {}", frame_path.display()))?;
    let mut image = image.to_rgba8();
    let adjustment_lut = grayscale_adjustment_lut(options);

    for pixel in image.pixels_mut() {
        let luma = luminance_byte(pixel[0], pixel[1], pixel[2]);
        let adjusted = adjustment_lut[usize::from(luma)];
        pixel[0] = adjusted;
        pixel[1] = adjusted;
        pixel[2] = adjusted;
    }

    let output = if format == ImageFormat::Jpeg {
        let mut rgb_raw = Vec::with_capacity(
            (image.width() as usize).saturating_mul(image.height() as usize) * 3,
        );
        for pixel in image.pixels() {
            rgb_raw.extend_from_slice(&pixel.0[..3]);
        }
        let rgb = RgbImage::from_raw(image.width(), image.height(), rgb_raw)
            .ok_or_else(|| anyhow::anyhow!("failed to construct grayscale JPEG image"))?;
        DynamicImage::ImageRgb8(rgb)
    } else {
        DynamicImage::ImageRgba8(image)
    };
    let temp_path = temp_rewrite_path(frame_path);
    let save_result = output
        .save_with_format(&temp_path, format)
        .with_context(|| format!("failed to encode grayscale image {}", temp_path.display()));
    if let Err(error) = save_result {
        let _ = fs::remove_file(&temp_path);
        return Err(error);
    }
    fs::rename(&temp_path, frame_path).with_context(|| {
        let _ = fs::remove_file(&temp_path);
        format!("failed to replace grayscale image {}", frame_path.display())
    })?;
    Ok(())
}

fn temp_rewrite_path(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("image");
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or_default();
    path.with_file_name(format!(
        ".{file_name}.petiglyph-grayscale-{}-{nonce}.tmp",
        std::process::id()
    ))
}

fn luminance_byte(r: u8, g: u8, b: u8) -> u8 {
    // Integer approximation of BT.601 luma.
    (((77u16 * r as u16) + (150u16 * g as u16) + (29u16 * b as u16)) >> 8) as u8
}

fn grayscale_adjustment_lut(options: AnimationGrayscaleOptions) -> [u8; 256] {
    let mut lut = [0u8; 256];
    for (value, adjusted) in lut.iter_mut().enumerate() {
        *adjusted = apply_grayscale_adjustments(value as u8, options);
    }
    lut
}

fn apply_grayscale_adjustments(value: u8, options: AnimationGrayscaleOptions) -> u8 {
    let gamma = (options.gamma_percent as f32 / 100.0).clamp(0.50, 2.00);
    let mut pixel = (value as f32 / 255.0).powf(1.0 / gamma) * 255.0;

    let contrast_factor = 1.0 + (options.contrast as f32 / 100.0);
    pixel = ((pixel - 128.0) * contrast_factor) + 128.0;
    pixel += options.brightness as f32;

    pixel.round().clamp(0.0, 255.0) as u8
}

fn import_one_file(
    input_dir: &Path,
    source: &Path,
    preferred_file_name: Option<&OsStr>,
    existing_policy: ExistingImportPolicy,
    result: &mut AnimationMediaImportResult,
    count_unsupported: bool,
) -> Result<()> {
    if count_unsupported && !is_supported_still_image(source) {
        result.skipped_unsupported += 1;
        return Ok(());
    }

    let file_name = preferred_file_name
        .map(PathBuf::from)
        .or_else(|| static_import_file_name(source))
        .ok_or_else(|| anyhow::anyhow!("missing file name for {}", source.display()))?;

    let canonical_destination = input_dir.join(&file_name);
    if paths_resolve_to_same_file(source, &canonical_destination) {
        result.imported_source_keys.push(source_key_from_input_path(
            input_dir,
            &canonical_destination,
        ));
        result.skipped_existing += 1;
        return Ok(());
    }

    if existing_policy == ExistingImportPolicy::ReuseIdentical
        && canonical_destination.exists()
        && files_have_same_contents(source, &canonical_destination)
    {
        result.imported_source_keys.push(source_key_from_input_path(
            input_dir,
            &canonical_destination,
        ));
        result.skipped_existing += 1;
        return Ok(());
    }

    let (destination, was_renamed) = next_available_import_destination(input_dir, &file_name);
    if is_avif_image(source) {
        convert_avif_image_to_png(source, &destination)?;
    } else {
        fs::copy(source, &destination).with_context(|| {
            format!(
                "failed to import {} into {}",
                source.display(),
                destination.display()
            )
        })?;
    }

    result
        .imported_source_keys
        .push(source_key_from_input_path(input_dir, &destination));
    result
        .created_source_keys
        .push(source_key_from_input_path(input_dir, &destination));
    result.imported += 1;
    if was_renamed {
        result.renamed += 1;
    }

    Ok(())
}

fn classify_input_kind(path: &Path) -> AnimationInputKind {
    let Some(ext) = path.extension().and_then(|ext| ext.to_str()) else {
        return AnimationInputKind::Unsupported;
    };
    let ext = ext.to_ascii_lowercase();

    if matches!(ext.as_str(), "gif") {
        return AnimationInputKind::Gif;
    }
    if matches!(ext.as_str(), "mp4" | "mov" | "mkv" | "webm" | "avi" | "m4v") {
        return AnimationInputKind::Video;
    }
    if is_supported_still_image(path) {
        return AnimationInputKind::StillImage;
    }

    AnimationInputKind::Unsupported
}

fn is_supported_still_image(path: &Path) -> bool {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some(ext) => matches!(
            ext.to_ascii_lowercase().as_str(),
            "png" | "jpg" | "jpeg" | "webp" | "avif" | "bmp" | "svg"
        ),
        None => false,
    }
}

fn expand_gif_frames_to_temp_pngs(
    path: &Path,
    max_frames: usize,
) -> Result<(TempExtractDir, Vec<PathBuf>)> {
    if max_frames == 0 {
        bail!("frame extraction limit is zero");
    }
    let file = File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let decoder = GifDecoder::new(BufReader::new(file))
        .with_context(|| format!("failed to decode gif {}", path.display()))?;
    let temp_root = TempExtractDir::new("gif")?;
    let mut out = Vec::new();

    for (idx, frame) in decoder.into_frames().enumerate() {
        if idx >= max_frames {
            break;
        }
        let out_path = temp_root.path().join(format!("{:06}.png", idx + 1));
        frame
            .with_context(|| {
                format!(
                    "failed to decode gif frame {} from {}",
                    idx + 1,
                    path.display()
                )
            })?
            .into_buffer()
            .save(&out_path)
            .with_context(|| format!("failed to write gif frame {}", out_path.display()))?;
        out.push(out_path);
    }

    Ok((temp_root, out))
}

fn expand_video_frames_to_temp_pngs(
    path: &Path,
    max_frames: usize,
) -> Result<(TempExtractDir, Vec<PathBuf>)> {
    if max_frames == 0 {
        bail!("frame extraction limit is zero");
    }
    let temp_root = TempExtractDir::new("video")?;
    let output_pattern = temp_root.path().join("%06d.png");

    let Some(ffmpeg_path) = resolve_command_path("ffmpeg") else {
        bail!("ffmpeg not found; install ffmpeg to import video files");
    };

    let ffmpeg_check = Command::new(&ffmpeg_path).arg("-version").output();
    if ffmpeg_check.is_err() {
        bail!("ffmpeg not found; install ffmpeg to import video files");
    }

    let output = Command::new(&ffmpeg_path)
        .arg("-v")
        .arg("error")
        .arg("-i")
        .arg(path)
        .arg("-vsync")
        .arg("0")
        .arg("-start_number")
        .arg("1")
        .arg("-frames:v")
        .arg(max_frames.to_string())
        .arg(&output_pattern)
        .output()
        .with_context(|| format!("failed to run ffmpeg for {}", path.display()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "video frame extraction failed for {}: {}",
            path.display(),
            stderr.trim()
        );
    }

    let mut frames = Vec::new();
    for entry in fs::read_dir(temp_root.path()).with_context(|| {
        format!(
            "failed to scan extracted frames in {}",
            temp_root.path().display()
        )
    })? {
        let path = entry?.path();
        if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("png"))
        {
            frames.push(path);
        }
    }
    frames.sort();
    Ok((temp_root, frames))
}

fn resolve_command_path(command: &str) -> Option<PathBuf> {
    let candidate = PathBuf::from(command);
    if candidate.is_absolute() && candidate.is_file() {
        return Some(candidate);
    }
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        if dir.as_os_str().is_empty() {
            continue;
        }
        let full = dir.join(&candidate);
        if full.is_file() {
            return Some(full);
        }
    }
    None
}

fn media_identity_hash_hex8(path: &Path) -> Result<String> {
    let metadata =
        fs::metadata(path).with_context(|| format!("failed to read {}", path.display()))?;
    let mut identity = path.to_string_lossy().into_owned().into_bytes();
    identity.extend_from_slice(&metadata.len().to_le_bytes());
    let modified_nanos = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0u128, |d| d.as_nanos());
    identity.extend_from_slice(&modified_nanos.to_le_bytes());

    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in identity {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    Ok(format!("{:08x}", (hash & 0xffff_ffff) as u32))
}

fn slug_stem(path: &Path) -> String {
    let raw = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("media");
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if ch == '-' || ch == '_' {
            out.push(ch);
        }
    }
    if out.is_empty() {
        "media".to_string()
    } else {
        out
    }
}

fn collect_dropped_paths(payload: &str) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for token in split_drop_payload_tokens(payload) {
        if let Some(path) = normalize_dropped_path_candidate(&token) {
            paths.push(path);
        }
    }
    paths
}

fn split_drop_payload_tokens(payload: &str) -> Vec<String> {
    let mut normalized = payload.replace(['\r', '\n'], " ");
    if normalized.contains("file://") {
        normalized = normalized.replace("file://", "\nfile://");
    }

    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut escaped = false;

    let mut chars = normalized.chars().peekable();
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

fn next_available_import_destination(input_dir: &Path, file_name: &Path) -> (PathBuf, bool) {
    let candidate = input_dir.join(file_name);
    if !candidate.exists() {
        return (candidate, false);
    }

    let stem = file_name
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("glyph");
    let ext = file_name
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

fn source_key_from_input_path(input_dir: &Path, source_path: &Path) -> String {
    source_path
        .strip_prefix(input_dir)
        .unwrap_or(source_path)
        .to_string_lossy()
        .replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::AnimationImportProcessingOptions;
    use super::classify_input_kind;
    use super::media_identity_hash_hex8;
    use super::resolve_command_path;
    use super::slug_stem;
    use super::{AnimationInputKind, ExistingImportPolicy, import_animation_media_to_input};
    use image::{Rgba, RgbaImage};
    use std::fs;
    use std::path::Path;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn make_temp_dir(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time is valid")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("petiglyph-{name}-{nonce}"));
        fs::create_dir_all(&dir).expect("temp dir is created");
        dir
    }

    fn write_test_png(path: &Path) {
        let mut img = RgbaImage::from_pixel(8, 8, Rgba([255, 255, 255, 0]));
        img.put_pixel(2, 2, Rgba([0, 0, 0, 255]));
        img.save(path).expect("test image is written");
    }

    #[test]
    fn classifies_still_gif_video_and_unsupported() {
        assert_eq!(
            classify_input_kind(Path::new("a.png")),
            AnimationInputKind::StillImage
        );
        assert_eq!(
            classify_input_kind(Path::new("a.gif")),
            AnimationInputKind::Gif
        );
        assert_eq!(
            classify_input_kind(Path::new("a.mp4")),
            AnimationInputKind::Video
        );
        assert_eq!(
            classify_input_kind(Path::new("a.txt")),
            AnimationInputKind::Unsupported
        );
    }

    #[test]
    fn slug_stem_filters_non_ascii_for_file_prefixes() {
        assert_eq!(slug_stem(Path::new("Runner Fast!!.mp4")), "runnerfast");
    }

    #[test]
    fn media_hash_is_stable_for_same_file_state() {
        let dir = make_temp_dir("anim-media-hash");
        let path = dir.join("x.bin");
        fs::write(&path, [1u8, 2u8, 3u8, 4u8]).expect("write test bytes");

        let a = media_identity_hash_hex8(&path).expect("hash a");
        let b = media_identity_hash_hex8(&path).expect("hash b");
        assert_eq!(a, b);

        fs::remove_dir_all(dir).expect("temp dir removed");
    }

    #[test]
    fn still_image_imports_and_reuses_identical_existing_file() {
        let dir = make_temp_dir("anim-media-import-reuse");
        let input_dir = dir.join("images");
        fs::create_dir_all(&input_dir).expect("images created");

        let source = dir.join("frame.png");
        write_test_png(&source);

        let payload = source.to_string_lossy().to_string();
        let first = import_animation_media_to_input(
            &input_dir,
            &payload,
            ExistingImportPolicy::ReuseIdentical,
            AnimationImportProcessingOptions::default(),
        )
        .expect("first import");
        assert_eq!(first.imported, 1);

        let second = import_animation_media_to_input(
            &input_dir,
            &payload,
            ExistingImportPolicy::ReuseIdentical,
            AnimationImportProcessingOptions::default(),
        )
        .expect("second import");
        assert_eq!(second.imported, 0);
        assert_eq!(second.skipped_existing, 1);
        assert_eq!(second.imported_source_keys, vec!["frame.png".to_string()]);

        fs::remove_dir_all(dir).expect("temp dir removed");
    }

    #[test]
    fn drop_payload_split_preserves_windows_path_separators() {
        let payload = r#"C:\Users\petiglyph\frame.png "C:\Users\petiglyph\space frame.png""#;
        let tokens = super::split_drop_payload_tokens(payload);

        assert_eq!(
            tokens,
            vec![
                r"C:\Users\petiglyph\frame.png".to_string(),
                r"C:\Users\petiglyph\space frame.png".to_string(),
            ]
        );
    }

    #[test]
    fn unescape_backslashes_preserves_windows_path_separators() {
        assert_eq!(
            super::unescape_backslashes(r"C:\Users\alice\images\frame.png"),
            r"C:\Users\alice\images\frame.png"
        );
    }

    #[test]
    fn resolve_command_path_accepts_absolute_file_path() {
        let dir = make_temp_dir("anim-media-resolve");
        let bin = dir.join("tool-stub");
        fs::write(&bin, b"stub").expect("stub command file is written");

        let resolved = resolve_command_path(bin.to_str().expect("path should be utf-8"));
        assert_eq!(resolved, Some(bin.clone()));

        fs::remove_dir_all(dir).expect("temp dir removed");
    }

    #[test]
    fn resolve_command_path_returns_none_for_missing_command() {
        let missing = format!(
            "petiglyph-anim-media-does-not-exist-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time is valid")
                .as_nanos()
        );
        assert_eq!(resolve_command_path(&missing), None);
    }
}
