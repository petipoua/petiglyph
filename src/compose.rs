use anyhow::{Context, Result, bail};
use image::imageops::FilterType;
use image::{ImageBuffer, Rgba, RgbaImage};
use resvg::tiny_skia::{Pixmap, Transform};
use resvg::usvg;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone)]
pub(crate) struct ComposedTile {
    pub(crate) rows: usize,
    pub(crate) cols: usize,
    pub(crate) row: usize,
    pub(crate) col: usize,
    pub(crate) coverage: Vec<u8>,
    pub(crate) fingerprint: String,
}

pub(crate) fn compose_tiles(
    source_path: &Path,
    rows: usize,
    cols: usize,
    glyph_size: u32,
) -> Result<Vec<ComposedTile>> {
    if rows == 0 || cols == 0 {
        bail!("composition rows/cols must be > 0");
    }
    if glyph_size == 0 {
        bail!("glyph_size must be > 0");
    }

    let source_bytes = fs::read(source_path)
        .with_context(|| format!("failed to read {}", source_path.display()))?;
    let source = load_source_rgba_fitted(source_path, rows, cols, glyph_size)?;
    let (width, height) = source.dimensions();
    if width == 0 || height == 0 {
        bail!("source image has invalid size: {}", source_path.display());
    }

    // Preserve global geometry for compositions by scaling once to the whole
    // grid target, then slicing fixed-size tiles. Resizing each tile
    // independently can shift content when source dimensions are not evenly
    // divisible by rows/cols.
    let grid_width = glyph_size
        .checked_mul(u32::try_from(cols).context("composition cols overflow u32")?)
        .context("composition grid width overflow")?;
    let grid_height = glyph_size
        .checked_mul(u32::try_from(rows).context("composition rows overflow u32")?)
        .context("composition grid height overflow")?;
    let scaled_grid =
        image::imageops::resize(&source, grid_width, grid_height, FilterType::Lanczos3);

    let has_transparency = scaled_grid.pixels().any(|p| p[3] < 255);
    let background = (!has_transparency).then(|| estimate_background_rgb(&scaled_grid));
    let mut coverage_grid = coverage_from_image(&scaled_grid, has_transparency, background);
    seal_internal_grid_seams(
        &mut coverage_grid,
        grid_width,
        grid_height,
        glyph_size,
        rows,
        cols,
    );

    let mut tiles = Vec::with_capacity(rows.saturating_mul(cols));
    for row in 0..rows {
        let y0 = glyph_size
            .checked_mul(u32::try_from(row).context("composition row overflow u32")?)
            .context("composition y offset overflow")?;
        for col in 0..cols {
            let x0 = glyph_size
                .checked_mul(u32::try_from(col).context("composition col overflow u32")?)
                .context("composition x offset overflow")?;

            let coverage = crop_coverage_tile(&coverage_grid, grid_width, x0, y0, glyph_size);
            let fingerprint = tile_fingerprint(&source_bytes, rows, cols, row, col);

            tiles.push(ComposedTile {
                rows,
                cols,
                row,
                col,
                coverage,
                fingerprint,
            });
        }
    }

    Ok(tiles)
}

fn load_source_rgba_fitted(
    path: &Path,
    rows: usize,
    cols: usize,
    glyph_size: u32,
) -> Result<RgbaImage> {
    let ext = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();

    let source = if ext == "svg" {
        // Render SVG with a target tied to the configured glyph size so composition
        // grids stay predictable and avoid pathological oversized rasterization.
        render_svg(path, rows, cols, glyph_size)?
    } else {
        image::open(path)
            .with_context(|| format!("failed to decode image {}", path.display()))?
            .to_rgba8()
    };

    let (src_w, src_h) = source.dimensions();
    let target_aspect = cols as f32 / rows as f32;
    let src_aspect = src_w as f32 / src_h as f32;

    let (new_w, new_h) = if src_aspect > target_aspect {
        // Source is wider than target aspect ratio (cols/rows)
        // We need to add vertical padding.
        let h = (src_w as f32 / target_aspect).round() as u32;
        (src_w, h)
    } else {
        // Source is taller than target aspect ratio
        // We need to add horizontal padding.
        let w = (src_h as f32 * target_aspect).round() as u32;
        (w, src_h)
    };

    let mut padded = ImageBuffer::new(new_w, new_h);
    let x_offset = (new_w.saturating_sub(src_w)) / 2;
    let y_offset = (new_h.saturating_sub(src_h)) / 2;

    image::imageops::replace(&mut padded, &source, x_offset as i64, y_offset as i64);
    Ok(padded)
}

fn render_svg(path: &Path, rows: usize, cols: usize, glyph_size: u32) -> Result<RgbaImage> {
    let data = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    let options = usvg::Options::default();
    let tree = usvg::Tree::from_data(&data, &options)
        .with_context(|| format!("failed to parse SVG {}", path.display()))?;

    let size = tree.size().to_int_size();
    let src_w = size.width().max(1);
    let src_h = size.height().max(1);
    let longest_grid = rows.max(cols).max(1) as u32;
    let target = (glyph_size.max(16) * longest_grid * 4).max(256);
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

fn coverage_from_image(
    image: &RgbaImage,
    has_transparency: bool,
    background: Option<[u8; 3]>,
) -> Vec<u8> {
    let mut out = Vec::with_capacity((image.width() as usize) * (image.height() as usize));
    for pixel in image.pixels() {
        let coverage = if has_transparency {
            pixel[3]
        } else {
            opaque_coverage(
                pixel,
                background.expect("background exists for opaque composition source"),
            )
        };
        out.push(coverage);
    }
    out
}

fn seal_internal_grid_seams(
    coverage: &mut [u8],
    width: u32,
    height: u32,
    glyph_size: u32,
    rows: usize,
    cols: usize,
) {
    let width = width as usize;
    let height = height as usize;
    let glyph_size = glyph_size as usize;

    if width == 0 || height == 0 || glyph_size == 0 {
        return;
    }

    for col in 1..cols {
        let seam_x = col.saturating_mul(glyph_size);
        if seam_x == 0 || seam_x >= width {
            continue;
        }

        for y in 0..height {
            let left_idx = y * width + seam_x - 1;
            let right_idx = y * width + seam_x;
            let coverage_max = coverage[left_idx].max(coverage[right_idx]);
            coverage[left_idx] = coverage_max;
            coverage[right_idx] = coverage_max;
        }
    }

    for row in 1..rows {
        let seam_y = row.saturating_mul(glyph_size);
        if seam_y == 0 || seam_y >= height {
            continue;
        }

        for x in 0..width {
            let top_idx = (seam_y - 1) * width + x;
            let bottom_idx = seam_y * width + x;
            let coverage_max = coverage[top_idx].max(coverage[bottom_idx]);
            coverage[top_idx] = coverage_max;
            coverage[bottom_idx] = coverage_max;
        }
    }
}

fn crop_coverage_tile(
    coverage: &[u8],
    grid_width: u32,
    x0: u32,
    y0: u32,
    glyph_size: u32,
) -> Vec<u8> {
    let grid_width = grid_width as usize;
    let x0 = x0 as usize;
    let y0 = y0 as usize;
    let glyph_size = glyph_size as usize;
    let mut out = Vec::with_capacity(glyph_size.saturating_mul(glyph_size));

    for y in 0..glyph_size {
        let start = (y0 + y) * grid_width + x0;
        let end = start + glyph_size;
        out.extend_from_slice(&coverage[start..end]);
    }

    out
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

fn tile_fingerprint(
    source_bytes: &[u8],
    rows: usize,
    cols: usize,
    row: usize,
    col: usize,
) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in source_bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    for value in [rows as u64, cols as u64, row as u64, col as u64] {
        for byte in value.to_le_bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    format!("fnv1a64:{hash:016x}")
}
