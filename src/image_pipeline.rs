use anyhow::{Context, Result, bail};
use image::imageops::FilterType;
use image::{GrayImage, ImageBuffer, Luma, Rgba, RgbaImage};
use resvg::tiny_skia::{Pixmap, Transform};
use resvg::usvg;
use std::fs;
use std::path::Path;

use crate::glyph_debug;

const OPAQUE_CONTENT_EPSILON: u8 = 6;
const DEBUG_ASCII_THRESHOLD: u8 = 64;

#[derive(Debug, Clone)]
struct SourceCoverage {
    width: u32,
    height: u32,
    coverage: Vec<u8>,
    content_min: u8,
}

pub(crate) fn preprocess_standard_source(
    path: &Path,
    glyph_size: u32,
    source_key: &str,
) -> Result<Vec<u8>> {
    if glyph_size == 0 {
        bail!("glyph_size must be > 0");
    }

    glyph_debug::log_step(
        "standard.start",
        format!("source={source_key} glyph_size={glyph_size}"),
    );
    let source = load_source_rgba(path, glyph_size)?;
    glyph_debug::write_rgba_png("01_standard_input_rgba", source_key, &source);

    let source_coverage = coverage_from_rgba(&source);
    glyph_debug::write_coverage_png(
        "02_standard_input_coverage",
        source_key,
        source_coverage.width,
        source_coverage.height,
        &source_coverage.coverage,
    );

    let fitted = fit_coverage_to_canvas(
        &source_coverage,
        glyph_size,
        glyph_size,
        Some(source_key),
        "standard",
    )?;

    glyph_debug::write_coverage_png(
        "06_standard_final_coverage",
        source_key,
        glyph_size,
        glyph_size,
        &fitted,
    );
    glyph_debug::write_ascii_coverage(
        "07_standard_final_ascii",
        source_key,
        glyph_size,
        glyph_size,
        &fitted,
        DEBUG_ASCII_THRESHOLD,
    );
    glyph_debug::log_step("standard.done", format!("source={source_key}"));

    Ok(fitted)
}

pub(crate) fn preprocess_composition_grid_source(
    path: &Path,
    rows: usize,
    cols: usize,
    glyph_size: u32,
    source_key: &str,
) -> Result<Vec<u8>> {
    if rows == 0 || cols == 0 {
        bail!("composition rows/cols must be > 0");
    }
    if glyph_size == 0 {
        bail!("glyph_size must be > 0");
    }

    glyph_debug::log_step(
        "grid.start",
        format!("source={source_key} rows={rows} cols={cols} glyph_size={glyph_size}"),
    );

    let render_hint = glyph_size
        .checked_mul(u32::try_from(rows.max(cols)).context("composition grid dimensions overflow")?)
        .context("composition render size overflow")?;
    let source = load_source_rgba(path, render_hint)?;
    glyph_debug::write_rgba_png("01_grid_input_rgba", source_key, &source);

    let source_coverage = coverage_from_rgba(&source);
    glyph_debug::write_coverage_png(
        "02_grid_input_coverage",
        source_key,
        source_coverage.width,
        source_coverage.height,
        &source_coverage.coverage,
    );

    let target_w = glyph_size
        .checked_mul(u32::try_from(cols).context("composition cols overflow u32")?)
        .context("composition grid width overflow")?;
    let target_h = glyph_size
        .checked_mul(u32::try_from(rows).context("composition rows overflow u32")?)
        .context("composition grid height overflow")?;

    let fitted = fit_coverage_to_canvas(
        &source_coverage,
        target_w,
        target_h,
        Some(source_key),
        "grid",
    )?;

    glyph_debug::write_coverage_png(
        "06_grid_fitted_full_coverage",
        source_key,
        target_w,
        target_h,
        &fitted,
    );
    glyph_debug::write_ascii_coverage(
        "07_grid_fitted_full_ascii",
        source_key,
        target_w,
        target_h,
        &fitted,
        DEBUG_ASCII_THRESHOLD,
    );
    glyph_debug::log_step(
        "grid.done",
        format!("source={source_key} target={}x{}", target_w, target_h),
    );

    Ok(fitted)
}

pub(crate) fn coverage_map_from_image(source: &RgbaImage, glyph_size: u32) -> Result<Vec<u8>> {
    if glyph_size == 0 {
        bail!("glyph_size must be > 0");
    }

    let source_coverage = coverage_from_rgba(source);
    fit_coverage_to_canvas(&source_coverage, glyph_size, glyph_size, None, "generic")
}

fn fit_coverage_to_canvas(
    source: &SourceCoverage,
    target_w: u32,
    target_h: u32,
    debug_label: Option<&str>,
    debug_mode: &str,
) -> Result<Vec<u8>> {
    if target_w == 0 || target_h == 0 {
        bail!("target dimensions must be > 0");
    }

    let expected_len = usize::try_from(source.width)
        .context("source width overflow")?
        .checked_mul(usize::try_from(source.height).context("source height overflow")?)
        .ok_or_else(|| anyhow::anyhow!("source coverage size overflow"))?;
    if source.coverage.len() != expected_len {
        bail!(
            "source coverage size mismatch: expected {expected_len}, got {}",
            source.coverage.len()
        );
    }

    let Some((min_x, min_y, max_x, max_y)) = content_bounds_from_coverage(
        &source.coverage,
        source.width,
        source.height,
        source.content_min,
    ) else {
        if let Some(label) = debug_label {
            glyph_debug::log_step(
                &format!("{debug_mode}.empty"),
                format!(
                    "source={label} no content; writing blank {}x{}",
                    target_w, target_h
                ),
            );
        }
        let len = usize::try_from(target_w)
            .context("target width overflow")?
            .checked_mul(usize::try_from(target_h).context("target height overflow")?)
            .ok_or_else(|| anyhow::anyhow!("target coverage size overflow"))?;
        return Ok(vec![0u8; len]);
    };

    let crop_w = max_x - min_x + 1;
    let crop_h = max_y - min_y + 1;
    let cropped = crop_coverage(&source.coverage, source.width, min_x, min_y, crop_w, crop_h)?;
    let cropped_img = GrayImage::from_raw(crop_w, crop_h, cropped)
        .ok_or_else(|| anyhow::anyhow!("failed to construct grayscale source image"))?;

    if let Some(label) = debug_label {
        glyph_debug::log_step(
            &format!("{debug_mode}.bounds"),
            format!(
                "source={label} bounds=({},{})->({},{}) crop={}x{} target={}x{}",
                min_x, min_y, max_x, max_y, crop_w, crop_h, target_w, target_h
            ),
        );
        glyph_debug::write_coverage_png(
            &format!("03_{debug_mode}_cropped_coverage"),
            label,
            crop_w,
            crop_h,
            cropped_img.as_raw(),
        );
    }

    let scale_x = target_w as f64 / crop_w as f64;
    let scale_y = target_h as f64 / crop_h as f64;
    let scale = scale_x.min(scale_y);
    let scaled_w = ((crop_w as f64 * scale).round() as u32).clamp(1, target_w);
    let scaled_h = ((crop_h as f64 * scale).round() as u32).clamp(1, target_h);

    let resized = image::imageops::resize(&cropped_img, scaled_w, scaled_h, FilterType::Lanczos3);
    if let Some(label) = debug_label {
        glyph_debug::write_coverage_png(
            &format!("04_{debug_mode}_resized_coverage"),
            label,
            scaled_w,
            scaled_h,
            resized.as_raw(),
        );
    }

    let mut canvas = GrayImage::from_pixel(target_w, target_h, Luma([0]));
    let offset_x = ((target_w - scaled_w) / 2) as i64;
    let offset_y = ((target_h - scaled_h) / 2) as i64;
    image::imageops::overlay(&mut canvas, &resized, offset_x, offset_y);

    if let Some(label) = debug_label {
        glyph_debug::log_step(
            &format!("{debug_mode}.placement"),
            format!(
                "source={label} resized={}x{} offset=({}, {})",
                scaled_w, scaled_h, offset_x, offset_y
            ),
        );
        glyph_debug::write_coverage_png(
            &format!("05_{debug_mode}_centered_coverage"),
            label,
            target_w,
            target_h,
            canvas.as_raw(),
        );
    }

    Ok(canvas.into_raw())
}

fn crop_coverage(
    coverage: &[u8],
    source_width: u32,
    x0: u32,
    y0: u32,
    width: u32,
    height: u32,
) -> Result<Vec<u8>> {
    let source_width = usize::try_from(source_width).context("source width overflow")?;
    let x0 = usize::try_from(x0).context("x0 overflow")?;
    let y0 = usize::try_from(y0).context("y0 overflow")?;
    let width = usize::try_from(width).context("crop width overflow")?;
    let height = usize::try_from(height).context("crop height overflow")?;

    let mut out = Vec::with_capacity(width.saturating_mul(height));
    for y in 0..height {
        let start = (y0 + y)
            .checked_mul(source_width)
            .and_then(|v| v.checked_add(x0))
            .ok_or_else(|| anyhow::anyhow!("crop row start overflow"))?;
        let end = start
            .checked_add(width)
            .ok_or_else(|| anyhow::anyhow!("crop row end overflow"))?;
        out.extend_from_slice(&coverage[start..end]);
    }

    Ok(out)
}

fn content_bounds_from_coverage(
    coverage: &[u8],
    width: u32,
    height: u32,
    content_min: u8,
) -> Option<(u32, u32, u32, u32)> {
    if width == 0 || height == 0 {
        return None;
    }

    let mut min_x = width;
    let mut min_y = height;
    let mut max_x = 0u32;
    let mut max_y = 0u32;
    let mut found = false;

    for (idx, value) in coverage.iter().enumerate() {
        if *value < content_min {
            continue;
        }

        found = true;
        let idx = idx as u32;
        let x = idx % width;
        let y = idx / width;
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
    }

    found.then_some((min_x, min_y, max_x, max_y))
}

fn coverage_from_rgba(source: &RgbaImage) -> SourceCoverage {
    let has_transparency = source.pixels().any(|p| p[3] < 255);

    if has_transparency {
        let mut coverage =
            Vec::with_capacity((source.width() as usize) * (source.height() as usize));
        for pixel in source.pixels() {
            coverage.push(pixel[3]);
        }

        return SourceCoverage {
            width: source.width(),
            height: source.height(),
            coverage,
            content_min: 1,
        };
    }

    let background = estimate_background_rgb(source);
    let mut coverage = Vec::with_capacity((source.width() as usize) * (source.height() as usize));
    for pixel in source.pixels() {
        coverage.push(opaque_coverage(pixel, background));
    }

    SourceCoverage {
        width: source.width(),
        height: source.height(),
        coverage,
        content_min: OPAQUE_CONTENT_EPSILON.saturating_add(1),
    }
}

pub(crate) fn load_source_rgba(path: &Path, target_hint: u32) -> Result<RgbaImage> {
    let ext = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();

    if ext == "svg" {
        render_svg(path, target_hint)
    } else {
        let img = image::open(path)
            .with_context(|| format!("failed to decode image {}", path.display()))?;
        Ok(img.to_rgba8())
    }
}

fn render_svg(path: &Path, target_hint: u32) -> Result<RgbaImage> {
    let data = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    let options = usvg::Options::default();
    let tree = usvg::Tree::from_data(&data, &options)
        .with_context(|| format!("failed to parse SVG {}", path.display()))?;

    let size = tree.size().to_int_size();
    let src_w = size.width().max(1);
    let src_h = size.height().max(1);
    let target = (target_hint.max(16) * 4).max(64);

    let scale = (target as f32 / src_w as f32).min(target as f32 / src_h as f32);
    let out_w = ((src_w as f32 * scale).round() as u32).max(1);
    let out_h = ((src_h as f32 * scale).round() as u32).max(1);

    let mut pixmap = Pixmap::new(out_w, out_h)
        .ok_or_else(|| anyhow::anyhow!("failed to allocate SVG render target"))?;

    let transform = Transform::from_scale(out_w as f32 / src_w as f32, out_h as f32 / src_h as f32);
    resvg::render(&tree, transform, &mut pixmap.as_mut());

    let pixels = pixmap.data().to_vec();
    ImageBuffer::from_raw(out_w, out_h, pixels)
        .ok_or_else(|| anyhow::anyhow!("failed to convert rendered SVG to RGBA image"))
}

fn estimate_background_rgb(source: &RgbaImage) -> [u8; 3] {
    let (width, height) = source.dimensions();
    let max_x = width.saturating_sub(1);
    let max_y = height.saturating_sub(1);
    let coords = [(0, 0), (max_x, 0), (0, max_y), (max_x, max_y)];

    let mut sum = [0u32; 3];
    for (x, y) in coords {
        let pixel = source.get_pixel(x, y);
        sum[0] += pixel[0] as u32;
        sum[1] += pixel[1] as u32;
        sum[2] += pixel[2] as u32;
    }

    [
        (sum[0] / coords.len() as u32) as u8,
        (sum[1] / coords.len() as u32) as u8,
        (sum[2] / coords.len() as u32) as u8,
    ]
}

fn opaque_coverage(pixel: &Rgba<u8>, background: [u8; 3]) -> u8 {
    if pixel[3] == 0 {
        return 0;
    }

    let dr = pixel[0].abs_diff(background[0]) as u16;
    let dg = pixel[1].abs_diff(background[1]) as u16;
    let db = pixel[2].abs_diff(background[2]) as u16;
    ((dr + dg + db) / 3) as u8
}
