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
    let source = load_source_rgba(source_path, rows, cols, glyph_size)?;
    let (width, height) = source.dimensions();
    if width == 0 || height == 0 {
        bail!("source image has invalid size: {}", source_path.display());
    }

    let has_transparency = source.pixels().any(|p| p[3] < 255);
    let background = (!has_transparency).then(|| estimate_background_rgb(&source));

    let mut tiles = Vec::with_capacity(rows.saturating_mul(cols));
    for row in 0..rows {
        let y0 = ((row as u32) * height) / rows as u32;
        let y1 = (((row + 1) as u32) * height) / rows as u32;
        let tile_h = y1.saturating_sub(y0).max(1);
        for col in 0..cols {
            let x0 = ((col as u32) * width) / cols as u32;
            let x1 = (((col + 1) as u32) * width) / cols as u32;
            let tile_w = x1.saturating_sub(x0).max(1);

            let tile = image::imageops::crop_imm(&source, x0, y0, tile_w, tile_h).to_image();
            let resized =
                image::imageops::resize(&tile, glyph_size, glyph_size, FilterType::Lanczos3);
            let coverage = coverage_from_resized_tile(&resized, has_transparency, background);
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

fn load_source_rgba(path: &Path, rows: usize, cols: usize, glyph_size: u32) -> Result<RgbaImage> {
    let ext = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();

    if ext == "svg" {
        render_svg(path, rows, cols, glyph_size)
    } else {
        let img = image::open(path)
            .with_context(|| format!("failed to decode image {}", path.display()))?;
        Ok(img.to_rgba8())
    }
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
    let target = (glyph_size.max(16) * longest_grid * 2).max(128);
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

fn coverage_from_resized_tile(
    tile: &RgbaImage,
    has_transparency: bool,
    background: Option<[u8; 3]>,
) -> Vec<u8> {
    let mut out = Vec::with_capacity((tile.width() as usize) * (tile.height() as usize));
    for pixel in tile.pixels() {
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
