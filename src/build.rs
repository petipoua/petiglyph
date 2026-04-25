use anyhow::{Context, Result, bail};
use image::imageops::FilterType;
use image::{ImageBuffer, Rgba, RgbaImage};
use resvg::tiny_skia::{Pixmap, Transform};
use resvg::usvg;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use crate::project::RuntimeConfig;

#[derive(Debug, Clone)]
pub(crate) struct PreprocessedGlyph {
    pub(crate) source_path: PathBuf,
    pub(crate) source_key: String,
    pub(crate) glyph_name: String,
    pub(crate) size: u32,
    pub(crate) coverage: Vec<u8>,
}

#[derive(Debug, Clone)]
pub(crate) struct GlyphBitmap {
    pub(crate) size: u32,
    pub(crate) pixels: Vec<bool>,
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

pub(crate) fn build_outputs(config: &RuntimeConfig) -> Result<BuildSummary> {
    let sources = collect_source_files(&config.input_dir)?;
    let glyphs = preprocess_sources(&sources, &config.input_dir, config.glyph_size)?;
    validate_codepoint_range(config.codepoint_start, glyphs.len())?;

    let previews_dir = config.out_dir.join("previews");
    fs::create_dir_all(&previews_dir)
        .with_context(|| format!("failed to create {}", previews_dir.display()))?;

    let mut mapping = Vec::with_capacity(glyphs.len());
    let mut bdf_glyphs = Vec::with_capacity(glyphs.len());

    for (idx, glyph) in glyphs.iter().enumerate() {
        let idx_u32 = u32::try_from(idx).context("too many glyphs to assign Unicode codepoints")?;
        let codepoint = config.codepoint_start + idx_u32;
        let threshold = effective_threshold(
            config.base_threshold,
            &config.threshold_overrides,
            &glyph.source_key,
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
    write_ttf(&ttf_path, &config.font_name, config.glyph_size, &bdf_glyphs)?;

    let sample_path = config.out_dir.join("glyph-sample.txt");
    let sample = glyph_sample_string(config.codepoint_start, bdf_glyphs.len());
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

pub(crate) fn preprocess_sources(
    sources: &[PathBuf],
    input_dir: &Path,
    glyph_size: u32,
) -> Result<Vec<PreprocessedGlyph>> {
    let mut used_names = HashSet::new();
    let mut out = Vec::with_capacity(sources.len());

    for source in sources {
        let source_rgba = load_source_rgba(source, glyph_size)?;
        let coverage = coverage_map(&source_rgba, glyph_size)?;
        let glyph_name = unique_glyph_name(source, &mut used_names);
        let source_key = source_manifest_key(source, input_dir);
        out.push(PreprocessedGlyph {
            source_path: source.clone(),
            source_key,
            glyph_name,
            size: glyph_size,
            coverage,
        });
    }

    Ok(out)
}

fn load_source_rgba(path: &Path, glyph_size: u32) -> Result<RgbaImage> {
    let ext = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();

    if ext == "svg" {
        render_svg(path, glyph_size)
    } else {
        let img = image::open(path)
            .with_context(|| format!("failed to decode image {}", path.display()))?;
        Ok(img.to_rgba8())
    }
}

fn render_svg(path: &Path, glyph_size: u32) -> Result<RgbaImage> {
    let data = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    let options = usvg::Options::default();
    let tree = usvg::Tree::from_data(&data, &options)
        .with_context(|| format!("failed to parse SVG {}", path.display()))?;

    let size = tree.size().to_int_size();
    let src_w = size.width().max(1);
    let src_h = size.height().max(1);
    let target = (glyph_size.max(16) * 4).max(64);

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

pub(crate) fn coverage_map(source: &RgbaImage, glyph_size: u32) -> Result<Vec<u8>> {
    const OPAQUE_CONTENT_EPSILON: u8 = 6;

    if glyph_size == 0 {
        bail!("glyph_size must be > 0");
    }

    let has_transparency = source.pixels().any(|p| p[3] < 255);
    let background = (!has_transparency).then(|| estimate_background_rgb(source));
    let content =
        crop_source_to_content(source, has_transparency, background, OPAQUE_CONTENT_EPSILON);
    let fitted = fit_to_canvas(&content, glyph_size);

    let mut out = vec![0u8; (glyph_size as usize) * (glyph_size as usize)];

    for (idx, pixel) in fitted.pixels().enumerate() {
        let coverage = if has_transparency {
            pixel[3]
        } else {
            opaque_coverage(
                pixel,
                background.expect("background exists for opaque sources"),
            )
        };

        out[idx] = coverage;
    }

    Ok(out)
}

fn crop_source_to_content(
    source: &RgbaImage,
    has_transparency: bool,
    background: Option<[u8; 3]>,
    opaque_content_epsilon: u8,
) -> RgbaImage {
    let Some((min_x, min_y, max_x, max_y)) =
        content_bounds(source, has_transparency, background, opaque_content_epsilon)
    else {
        return source.clone();
    };

    let (width, height) = source.dimensions();
    if min_x == 0
        && min_y == 0
        && max_x == width.saturating_sub(1)
        && max_y == height.saturating_sub(1)
    {
        return source.clone();
    }

    let crop_w = max_x - min_x + 1;
    let crop_h = max_y - min_y + 1;
    image::imageops::crop_imm(source, min_x, min_y, crop_w, crop_h).to_image()
}

fn content_bounds(
    source: &RgbaImage,
    has_transparency: bool,
    background: Option<[u8; 3]>,
    opaque_content_epsilon: u8,
) -> Option<(u32, u32, u32, u32)> {
    let (width, height) = source.dimensions();
    if width == 0 || height == 0 {
        return None;
    }

    let mut min_x = width;
    let mut min_y = height;
    let mut max_x = 0u32;
    let mut max_y = 0u32;
    let mut found = false;

    for y in 0..height {
        for x in 0..width {
            let pixel = source.get_pixel(x, y);
            let is_content = if has_transparency {
                pixel[3] > 0
            } else {
                let bg = background.expect("background is available for opaque sources");
                opaque_coverage(pixel, bg) > opaque_content_epsilon
            };

            if !is_content {
                continue;
            }

            found = true;
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }
    }

    found.then_some((min_x, min_y, max_x, max_y))
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

fn fit_to_canvas(source: &RgbaImage, glyph_size: u32) -> RgbaImage {
    let (width, height) = source.dimensions();
    let width = width.max(1);
    let height = height.max(1);

    let scale = (glyph_size as f32 / width as f32).min(glyph_size as f32 / height as f32);
    let scaled_w = ((width as f32 * scale).round() as u32).max(1);
    let scaled_h = ((height as f32 * scale).round() as u32).max(1);

    let resized = image::imageops::resize(source, scaled_w, scaled_h, FilterType::Lanczos3);

    let mut canvas = RgbaImage::from_pixel(glyph_size, glyph_size, Rgba([255, 255, 255, 0]));
    let offset_x = ((glyph_size - scaled_w) / 2) as i64;
    let offset_y = ((glyph_size - scaled_h) / 2) as i64;
    image::imageops::overlay(&mut canvas, &resized, offset_x, offset_y);

    canvas
}

fn threshold_bitmap(glyph: &PreprocessedGlyph, threshold: u8) -> GlyphBitmap {
    let pixels = glyph.coverage.iter().map(|v| *v >= threshold).collect();
    GlyphBitmap {
        size: glyph.size,
        pixels,
    }
}

fn write_preview_png(path: &Path, bitmap: &GlyphBitmap) -> Result<()> {
    let mut img = RgbaImage::from_pixel(bitmap.size, bitmap.size, Rgba([255, 255, 255, 0]));

    for y in 0..bitmap.size as usize {
        for x in 0..bitmap.size as usize {
            let idx = y * bitmap.size as usize + x;
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

    out.push_str("STARTFONT 2.1\n");
    out.push_str(&format!("FONT {}\n", bdf_font_name(font_name, glyph_size)));
    out.push_str(&format!("SIZE {} 75 75\n", glyph_size));
    out.push_str(&format!(
        "FONTBOUNDINGBOX {} {} 0 0\n",
        glyph_size, glyph_size
    ));
    out.push_str("STARTPROPERTIES 2\n");
    out.push_str(&format!("FONT_ASCENT {}\n", glyph_size));
    out.push_str("FONT_DESCENT 0\n");
    out.push_str("ENDPROPERTIES\n");
    out.push_str(&format!("CHARS {}\n", glyphs.len()));

    for (name, codepoint, bitmap) in glyphs {
        out.push_str(&format!("STARTCHAR {}\n", name));
        out.push_str(&format!("ENCODING {}\n", codepoint));
        out.push_str("SWIDTH 500 0\n");
        out.push_str(&format!("DWIDTH {} 0\n", glyph_size));
        out.push_str(&format!("BBX {} {} 0 0\n", glyph_size, glyph_size));
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

fn write_ttf(
    path: &Path,
    font_name: &str,
    glyph_size: u32,
    glyphs: &[(String, u32, GlyphBitmap)],
) -> Result<()> {
    let bytes = build_ttf(font_name, glyph_size, glyphs)?;
    fs::write(path, bytes).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

fn build_ttf(
    font_name: &str,
    glyph_size: u32,
    glyphs: &[(String, u32, GlyphBitmap)],
) -> Result<Vec<u8>> {
    let units_per_em = glyph_size
        .checked_mul(16)
        .context("glyph_size is too large for TTF export")?;
    let units_per_em =
        u16::try_from(units_per_em).context("glyph_size is too large for TTF export")?;

    let mut ttf_glyphs = Vec::with_capacity(glyphs.len() + 2);
    ttf_glyphs.push(bitmap_glyph_to_ttf(
        &notdef_bitmap(glyph_size),
        None,
        units_per_em,
        units_per_em,
    )?);
    ttf_glyphs.push(bitmap_glyph_to_ttf(
        &GlyphBitmap {
            size: glyph_size,
            pixels: vec![false; (glyph_size as usize) * (glyph_size as usize)],
        },
        Some(0x0020),
        units_per_em,
        units_per_em / 2,
    )?);

    for (_, codepoint, bitmap) in glyphs {
        ttf_glyphs.push(bitmap_glyph_to_ttf(
            bitmap,
            Some(*codepoint),
            units_per_em,
            units_per_em,
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
        let right_side_bearing = glyph.advance_width as i32
            - i32::from(glyph.left_side_bearing)
            - i32::from(glyph.x_max);
        min_right_side_bearing = min_right_side_bearing.min(right_side_bearing as i16);
        x_max_extent = x_max_extent.max(glyph.left_side_bearing.saturating_add(glyph.x_max));

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
        units_per_em,
        advance_width_max,
        min_left_side_bearing,
        min_right_side_bearing,
        x_max_extent,
        num_glyphs,
    );
    let maxp = build_maxp_table(num_glyphs, max_points, max_contours);
    let loca_table = build_loca_table(&loca);
    let cmap = build_cmap_table(&mappings);
    let name = build_name_table(font_name);
    let post = build_post_table();
    let os2 = build_os2_table(units_per_em, &mappings, advance_width_max);

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

fn notdef_bitmap(size: u32) -> GlyphBitmap {
    let mut pixels = vec![false; (size as usize) * (size as usize)];
    let thickness = (size / 16).max(1);

    for y in 0..size {
        for x in 0..size {
            let border = x < thickness
                || y < thickness
                || x >= size.saturating_sub(thickness)
                || y >= size.saturating_sub(thickness);
            if border {
                let idx = y as usize * size as usize + x as usize;
                pixels[idx] = true;
            }
        }
    }

    GlyphBitmap { size, pixels }
}

fn bitmap_glyph_to_ttf(
    bitmap: &GlyphBitmap,
    codepoint: Option<u32>,
    units_per_em: u16,
    advance_width: u16,
) -> Result<TtfGlyph> {
    if bitmap.size == 0 {
        bail!("glyph bitmap size must be > 0 for TTF export");
    }

    let pixel_units = i16::try_from(u32::from(units_per_em) / bitmap.size)
        .context("invalid pixel scaling for TTF export")?;

    let mut points = Vec::new();
    let mut end_points = Vec::new();
    let mut x_min = i16::MAX;
    let mut y_min = i16::MAX;
    let mut x_max = i16::MIN;
    let mut y_max = i16::MIN;

    for y in 0..bitmap.size as usize {
        for x in 0..bitmap.size as usize {
            if !bitmap.pixels[y * bitmap.size as usize + x] {
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
        left_side_bearing: 0,
        x_min,
        y_min,
        x_max,
        y_max,
        contour_count,
        point_count,
        data,
    })
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
    units_per_em: u16,
    advance_width_max: u16,
    min_left_side_bearing: i16,
    min_right_side_bearing: i16,
    x_max_extent: i16,
    number_of_h_metrics: u16,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(36);
    push_u32(&mut out, 0x0001_0000);
    push_i16(&mut out, units_per_em as i16);
    push_i16(&mut out, 0);
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

fn build_name_table(font_name: &str) -> Vec<u8> {
    let family = font_name.trim();
    let family = if family.is_empty() {
        "Petiglyph"
    } else {
        family
    };
    let postscript = postscript_name(family);
    let full_name = format!("{family} Regular");
    let unique = format!("{family};Regular");

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

fn build_os2_table(units_per_em: u16, mappings: &[(u32, u16)], advance_width: u16) -> Vec<u8> {
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
    push_u16(&mut out, 0x0040);
    push_u16(&mut out, first_char);
    push_u16(&mut out, last_char);
    push_i16(&mut out, units_per_em as i16);
    push_i16(&mut out, 0);
    push_i16(&mut out, 0);
    push_u16(&mut out, units_per_em);
    push_u16(&mut out, 0);
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
    let width = bitmap.size as usize;
    let height = bitmap.size as usize;
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

fn bdf_font_name(font_name: &str, glyph_size: u32) -> String {
    let slug = slugify(font_name);
    let glyph_size = glyph_size.max(1);
    if slug.is_empty() {
        format!(
            "-misc-petiglyph-medium-r-normal--{glyph_size}-{glyph_size}0-75-75-c-{glyph_size}0-iso10646-1"
        )
    } else {
        format!(
            "-misc-{slug}-medium-r-normal--{glyph_size}-{glyph_size}0-75-75-c-{glyph_size}0-iso10646-1"
        )
    }
}

pub(crate) fn glyph_sample_string(codepoint_start: u32, glyph_count: usize) -> String {
    let mut out = String::new();
    for idx in 0..glyph_count {
        if let Some(ch) = char::from_u32(codepoint_start + idx as u32) {
            out.push(ch);
            out.push(' ');
        }
    }
    out.trim_end().to_string()
}
