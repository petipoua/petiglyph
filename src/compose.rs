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

    let grid_height = glyph_size
        .checked_mul(u32::try_from(rows).context("composition rows overflow u32")?)
        .context("composition grid height overflow")?;
    let assembled = assemble_tiles_coverage(&tiles, rows, cols, glyph_size);
    glyph_debug::write_coverage_png(
        "09_grid_tiles_assembled",
        source_key,
        grid_width,
        grid_height,
        &assembled,
    );

    let mut assembled_bordered = assembled.clone();
    add_tile_borders(&mut assembled_bordered, rows, cols, glyph_size);
    glyph_debug::write_coverage_png(
        "10_grid_tiles_assembled_bordered",
        source_key,
        grid_width,
        grid_height,
        &assembled_bordered,
    );

    glyph_debug::log_step(
        "grid.assembled",
        format!("source={source_key} size={}x{}", grid_width, grid_height),
    );

    Ok(tiles)
}

fn assemble_tiles_coverage(
    tiles: &[ComposedTile],
    rows: usize,
    cols: usize,
    glyph_size: u32,
) -> Vec<u8> {
    let tile_size = glyph_size as usize;
    let width = cols.saturating_mul(tile_size);
    let height = rows.saturating_mul(tile_size);
    let mut assembled = vec![0u8; width.saturating_mul(height)];

    for tile in tiles {
        let x0 = tile.col.saturating_mul(tile_size);
        let y0 = tile.row.saturating_mul(tile_size);
        for y in 0..tile_size {
            let dst_start = (y0 + y).saturating_mul(width) + x0;
            let src_start = y.saturating_mul(tile_size);
            let dst_end = dst_start + tile_size;
            let src_end = src_start + tile_size;
            if dst_end <= assembled.len() && src_end <= tile.coverage.len() {
                assembled[dst_start..dst_end].copy_from_slice(&tile.coverage[src_start..src_end]);
            }
        }
    }

    assembled
}

fn add_tile_borders(coverage: &mut [u8], rows: usize, cols: usize, glyph_size: u32) {
    let tile_size = glyph_size as usize;
    if tile_size == 0 || rows == 0 || cols == 0 {
        return;
    }
    let width = cols.saturating_mul(tile_size);
    let height = rows.saturating_mul(tile_size);
    if coverage.len() != width.saturating_mul(height) {
        return;
    }

    for row in 0..rows {
        let y0 = row.saturating_mul(tile_size);
        let y1 = y0 + tile_size - 1;
        for col in 0..cols {
            let x0 = col.saturating_mul(tile_size);
            let x1 = x0 + tile_size - 1;

            for x in x0..=x1 {
                coverage[y0 * width + x] = 255;
                coverage[y1 * width + x] = 255;
            }
            for y in y0..=y1 {
                coverage[y * width + x0] = 255;
                coverage[y * width + x1] = 255;
            }
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
