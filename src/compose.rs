use anyhow::{Context, Result, bail};
use std::fs;
use std::path::Path;

use crate::glyph_debug;
use crate::image_pipeline::preprocess_composition_grid_source;

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
    source_key: &str,
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

    // Composite flow: fit once to the full grid area, then split into fixed tiles
    // without any per-tile fit/centering so tiles stay perfectly contiguous.
    let grid_coverage =
        preprocess_composition_grid_source(source_path, rows, cols, glyph_size, source_key)?;
    let grid_width = glyph_size
        .checked_mul(u32::try_from(cols).context("composition cols overflow u32")?)
        .context("composition grid width overflow")?;

    let mut tiles = Vec::with_capacity(rows.saturating_mul(cols));
    for row in 0..rows {
        let y0 = glyph_size
            .checked_mul(u32::try_from(row).context("composition row overflow u32")?)
            .context("composition y offset overflow")?;
        for col in 0..cols {
            let x0 = glyph_size
                .checked_mul(u32::try_from(col).context("composition col overflow u32")?)
                .context("composition x offset overflow")?;

            let coverage = crop_coverage_tile(&grid_coverage, grid_width, x0, y0, glyph_size);
            glyph_debug::write_coverage_png(
                "08_grid_tile_coverage",
                &format!("{source_key}_r{}_c{}", row + 1, col + 1),
                glyph_size,
                glyph_size,
                &coverage,
            );
            glyph_debug::write_ascii_coverage(
                "09_grid_tile_ascii",
                &format!("{source_key}_r{}_c{}", row + 1, col + 1),
                glyph_size,
                glyph_size,
                &coverage,
                64,
            );
            glyph_debug::log_step(
                "grid.tile",
                format!(
                    "source={source_key} tile=({row},{col}) offset=({}, {}) size={}x{}",
                    x0, y0, glyph_size, glyph_size
                ),
            );
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
