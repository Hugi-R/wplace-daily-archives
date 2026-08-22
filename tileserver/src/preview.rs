//! Whole-world z=0 preview image for the site's `og:image`/`twitter:image`.
//!
//! The latest week database is merged from the whole-world z=2 tile grid (a
//! fixed 4×4 set) down to a single 1000×1000 z=0 mosaic, composited over the
//! OSM map background (`osm000.png`), and exported as a full-color (RGBA) PNG.
//!
//! Unlike the archival merge, every colour — including transparent — is given
//! equal weight, so a pixel surrounded by empty neighbours stays empty instead
//! of being filled with a lone coloured pixel, and the map shows through.

use std::{
    fs,
    io::BufReader,
    path::Path,
    sync::OnceLock,
};

use anyhow::{anyhow, bail, Context, Result};
use axum::body::Bytes;
use png::{BitDepth, ColorType, Decoder, Encoder};
use tracing::info;
use wimage::{
    palette,
    tilehistory::{DateHours, TileHistory},
    PalettedImage,
};

use super::DatabaseManager;

const TILE_SIZE: usize = 1000;
const TILE_PIXELS: usize = TILE_SIZE * TILE_SIZE;

/// A 4×4 grid of z=2 tiles covering the whole world, indexed as `[y][x]`.
type WorldGrid = [[Option<PalettedImage>; 4]; 4];

/// Equal weights for every colour, including transparent. Unlike the normal
/// merge (where transparent never wins), this lets an empty pixel surrounded
/// by empty neighbours stay empty instead of being filled by a lone coloured
/// pixel.
fn downscale_weights() -> &'static [u32; 256] {
    static WEIGHTS: OnceLock<[u32; 256]> = OnceLock::new();
    WEIGHTS.get_or_init(|| [1u32; 256])
}

fn empty_tile() -> PalettedImage {
    PalettedImage {
        width: TILE_SIZE,
        height: TILE_SIZE,
        indices: vec![palette::TRANSPARENT; TILE_PIXELS],
    }
}

const BLOCK_SIZE: usize = 4;
const QUARTER_TILE_SIZE: usize = TILE_SIZE / BLOCK_SIZE;

fn validate_tile(tile: &PalettedImage) -> Result<()> {
    if tile.width != TILE_SIZE
        || tile.height != TILE_SIZE
        || tile.indices.len() != TILE_PIXELS
    {
        bail!(
            "expected a {TILE_SIZE}×{TILE_SIZE} tile with {TILE_PIXELS} pixels, \
             got {}×{} with {} pixels",
            tile.width,
            tile.height,
            tile.indices.len(),
        );
    }
    Ok(())
}

/// Merge the whole-world 4×4 z=2 grid into a single 1000×1000 z=0 image by
/// directly downscaling each present 1000×1000 tile by 4 (1000/4 = 250) into
/// its 250×250 block at `(x*250, y*250)`. Missing tiles leave their block empty
/// so the map shows through; all missing -> `None`.
fn merge_grid(tiles: WorldGrid) -> Result<Option<PalettedImage>> {
    if tiles.iter().flatten().all(Option::is_none) {
        return Ok(None);
    }

    let mut output = empty_tile();

    for (y, row) in tiles.iter().enumerate() {
        for (x, slot) in row.iter().enumerate() {
            let Some(tile) = slot else { continue };
            validate_tile(tile)?;

            let reduced = tile.downscale_mode_weighted(downscale_weights(), BLOCK_SIZE);
            if reduced.width != QUARTER_TILE_SIZE
                || reduced.height != QUARTER_TILE_SIZE
                || reduced.indices.len() != QUARTER_TILE_SIZE * QUARTER_TILE_SIZE
            {
                bail!("unexpected downscaled tile dimensions");
            }

            let dst_x = x * QUARTER_TILE_SIZE;
            let dst_y = y * QUARTER_TILE_SIZE;
            for row in 0..QUARTER_TILE_SIZE {
                let src_offset = row * QUARTER_TILE_SIZE;
                let dst_offset = (dst_y + row) * TILE_SIZE + dst_x;
                output.indices[dst_offset..dst_offset + QUARTER_TILE_SIZE]
                    .copy_from_slice(&reduced.indices[src_offset..src_offset + QUARTER_TILE_SIZE]);
            }
        }
    }

    Ok(Some(output))
}

/// Blit the (possibly partial) merged mosaic over an RGBA pixel buffer: opaque
/// wplace pixels overwrite the background, empty (`TRANSPARENT`) pixels keep it.
fn composite_rgba(buf: &mut [u8], merged: Option<&PalettedImage>) {
    let Some(merged) = merged else { return };
    debug_assert_eq!(buf.len(), merged.indices.len() * 4);
    for (i, &idx) in merged.indices.iter().enumerate() {
        if idx != palette::TRANSPARENT {
            buf[i * 4..i * 4 + 4].copy_from_slice(&palette::rgba_from_index(idx));
        }
    }
}

/// Decode a tile blob and return the image at `date`, or `None` if the tile or
/// that date is absent.
fn decode_tile_at(blob: Option<Vec<u8>>, date: u32) -> Result<Option<PalettedImage>> {
    let Some(blob) = blob else {
        return Ok(None);
    };
    let history = TileHistory::from_bytes(&blob).context("decode TileHistory blob")?;
    match history.get(DateHours(date)) {
        Ok(Some(image)) => Ok(Some(image)),
        Ok(None) => Ok(None),
        Err(error) => Err(error).context("get image from TileHistory"),
    }
}

/// Load the OSM map background (frontend/osm000.png, a 1000×1000 RGBA PNG) into
/// a raw RGBA pixel buffer. Fidelity is preserved — no palette quantization.
fn load_background_rgba(path: &Path) -> Result<Vec<u8>> {
    let file = fs::File::open(path)
        .with_context(|| format!("failed to open background image {}", path.display()))?;

    let mut reader = Decoder::new(BufReader::new(file)).read_info()?;
    let buf_size = reader
        .output_buffer_size()
        .ok_or_else(|| anyhow!("PNG decoder did not provide an output buffer size"))?;
    let mut buf = vec![0u8; buf_size];
    let info = reader.next_frame(&mut buf)?;
    let width = info.width as usize;
    let height = info.height as usize;
    if width != TILE_SIZE || height != TILE_SIZE {
        bail!("expected a {TILE_SIZE}×{TILE_SIZE} background, got {width}×{height}");
    }

    let rgba: Vec<u8> = match (info.color_type, info.bit_depth) {
        (ColorType::Rgba, BitDepth::Eight) => buf,
        (ColorType::Rgb, BitDepth::Eight) => {
            let mut out = Vec::with_capacity(TILE_PIXELS * 4);
            for px in buf.chunks_exact(3) {
                out.extend_from_slice(&[px[0], px[1], px[2], 255]);
            }
            out
        }
        other => bail!("unsupported background PNG format: {other:?}"),
    };

    if rgba.len() != TILE_PIXELS * 4 {
        bail!(
            "unexpected background size: {} pixels ({}×{}), expected {TILE_PIXELS}",
            rgba.len() / 4,
            width,
            height
        );
    }

    Ok(rgba)
}

/// Encode an RGBA pixel buffer as a full-color PNG (no palette).
fn encode_rgba_png(rgba: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut encoder = Encoder::new(&mut out, TILE_SIZE as u32, TILE_SIZE as u32);
    encoder.set_color(ColorType::Rgba);
    encoder.set_depth(BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(rgba)?;
    writer.finish()?;
    Ok(out)
}

/// Build the latest-state preview image (see the module docs). Runs once at
/// startup on a background thread (see `TileServer::new`).
pub fn make_latest_image(db: &DatabaseManager, data_path: &Path) -> Result<Bytes> {
    let background = load_background_rgba(&data_path.join("osm000.png"))?;

    // Latest week DB and newest snapshot date within it. The whole world at
    // z=2 is a fixed 4×4 grid; tiles the DB lacks are treated as empty.
    let Some((week, date)) = db.latest_snapshot() else {
        return Ok(Bytes::from(encode_rgba_png(&background)?));
    };

    let mut tiles: WorldGrid = std::array::from_fn(|_| std::array::from_fn(|_| None));
    for (y, row) in tiles.iter_mut().enumerate() {
        for (x, slot) in row.iter_mut().enumerate() {
            *slot = decode_tile_at(
                match db.get_tile(2, x as u16, y as u16, week) {
                    Ok(v) => Some(v),
                    Err(super::TileError::TileNotFound)
                    | Err(super::TileError::VersionNotFound(_)) => None,
                    Err(e) => return Err(e).with_context(|| "read z=2 tile"),
                },
                date,
            )?;
        }
    }

    let merged = merge_grid(tiles)?;
    let mut out = background;
    composite_rgba(&mut out, merged.as_ref());

    info!("preview image built (z=2 -> z=0, 4× reduction)");
    Ok(Bytes::from(encode_rgba_png(&out)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;

    fn tile_blob_at(value: u8, date: u32) -> Vec<u8> {
        let img = PalettedImage {
            width: TILE_SIZE,
            height: TILE_SIZE,
            indices: vec![value; TILE_PIXELS],
        };
        let mut th = TileHistory {
            imgs: Default::default(),
        };
        th.set(DateHours(date), img).unwrap();
        th.to_bytes()
    }

    /// One week-0 DB with all sixteen whole-world z=2 tiles populated for `date`,
    /// each a solid distinct palette colour, except (0,0) which is left absent so
    /// the empty quadrant shows the map background.
    fn create_preview_db(data_dir: &Path, date: u32) {
        std::fs::create_dir_all(data_dir.join("weeks")).unwrap();
        let conn =
            rusqlite::Connection::open(data_dir.join("weeks").join("w0_0.db")).unwrap();
        conn.execute_batch(
            "CREATE TABLE tiles (z INTEGER NOT NULL, x INTEGER NOT NULL, y INTEGER NOT NULL, data BLOB NOT NULL, PRIMARY KEY (z, x, y));
             CREATE TABLE versions (date INTEGER PRIMARY KEY, original_file TEXT);",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO versions (date, original_file) VALUES (?1, 'test')",
            params![date],
        )
        .unwrap();
        for y in 0..4i64 {
            for x in 0..4i64 {
                let value = ((y * 4 + x) % 62 + 1) as u8; // 1..=62, distinct per cell
                let blob = tile_blob_at(value, date);
                conn.execute(
                    "INSERT INTO tiles (z, x, y, data) VALUES (2, ?1, ?2, ?3)",
                    params![x, y, blob],
                )
                .unwrap();
            }
        }
        conn.execute("DELETE FROM tiles WHERE z = 2 AND x = 0 AND y = 0", [])
            .unwrap();
    }

    /// A 1000×1000 RGBA background of a single colour (74,107,58), written as a
    /// truecolor PNG like osm000.png.
    fn write_background(data_dir: &Path) {
        let size = TILE_SIZE as u32;
        let mut png = Vec::new();
        let mut enc = png::Encoder::new(&mut png, size, size);
        enc.set_color(ColorType::Rgba);
        enc.set_depth(BitDepth::Eight);
        let mut writer = enc.write_header().unwrap();
        let pixel = [74u8, 107, 58, 255];
        let data: Vec<u8> = pixel
            .iter()
            .cycle()
            .copied()
            .take(TILE_PIXELS * 4)
            .collect();
        writer.write_image_data(&data).unwrap();
        writer.finish().unwrap();
        std::fs::write(data_dir.join("osm000.png"), png).unwrap();
    }

    #[test]
    fn equal_weights_turn_isolated_pixel_into_empty() {
        // A 2×2 with three transparent pixels and one coloured pixel: with the
        // weighted mode (all weights equal) the mode is transparent, so the
        // isolated coloured pixel must NOT survive into the output.
        let img = PalettedImage {
            width: 2,
            height: 2,
            indices: vec![palette::TRANSPARENT, palette::TRANSPARENT, palette::TRANSPARENT, 7],
        };
        let out = img.downscale_mode_weighted(downscale_weights(), 2);
        assert_eq!(out.indices, vec![palette::TRANSPARENT]);
    }

    #[test]
    fn make_latest_image_merges_z2_into_1000x1000_over_background() {
        let tmp = tempfile::tempdir().unwrap();
        create_preview_db(tmp.path(), 0);
        write_background(tmp.path());

        let mut mgr = crate::DatabaseManager::new();
        mgr.initialize_week_databases(&tmp.path().join("weeks")).unwrap();

        let bytes = make_latest_image(&mgr, tmp.path()).unwrap();
        let img = decode_to_rgba(&bytes);
        assert_eq!(img.len(), TILE_PIXELS * 4);

        let px = |x: usize, y: usize| {
            let i = (y * TILE_SIZE + x) * 4;
            (img[i], img[i + 1], img[i + 2], img[i + 3])
        };
        let palette_rgb = |idx: u8| {
            let [r, g, b, _] = palette::rgba_from_index(idx);
            (r, g, b, 255)
        };
        for y in 0..4usize {
            for x in 0..4usize {
                let expected = palette_rgb(((y * 4 + x) % 62 + 1) as u8);
                let center = px(x * 250 + 125, y * 250 + 125);
                if x == 0 && y == 0 {
                    assert_eq!(
                        center,
                        (74, 107, 58, 255),
                        "empty (0,0) quadrant falls back to the map background"
                    );
                } else {
                    assert_eq!(center, expected, "tile ({x},{y}) at its 250×250 block");
                }
            }
        }
    }

    fn decode_to_rgba(bytes: &axum::body::Bytes) -> Vec<u8> {
        let mut reader = Decoder::new(std::io::Cursor::new(bytes)).read_info().unwrap();
        let info = reader.info().clone();
        let mut buf = vec![0u8; reader.output_buffer_size().unwrap()];
        reader.next_frame(&mut buf).unwrap();
        assert_eq!((info.width, info.height), (TILE_SIZE as u32, TILE_SIZE as u32));
        assert_eq!(info.color_type, ColorType::Rgba);
        assert_eq!(info.bit_depth, BitDepth::Eight);
        buf
    }
}