/// Implements the `makebase --base BASE_DB --output ARCHIVE_DB [--datehours DATEHOURS]` command.
/// Convert the base image PNGs from BASE_DB into a fresh ARCHIVE_DB as TileHistory blobs at z=11.
///
/// See `tasks/task_03_makebase.md` for the full spec.

use std::io::Cursor;

use anyhow::{Context, Result};
use wimage::tilehistory::{DateHours, TileHistory};
use wimage::PalettedImage;

// ─── Types ───────────────────────────────────────────────────────────────────

pub(crate) struct MakebaseJob {
    x: i32,
    y: i32,
    date_hours: DateHours,
    png: Vec<u8>,
}

pub(crate) struct MakebaseResult {
    x: i32,
    y: i32,
    data: Vec<u8>,
}

// ─── Worker ──────────────────────────────────────────────────────────────────

/// Convert a PNG tile into a fresh TileHistory whose only version is the image at date_hours.
pub(crate) fn process_job(job: MakebaseJob) -> Result<Option<MakebaseResult>> {
    let MakebaseJob { x, y, date_hours, png } = job;

    let image =
        PalettedImage::from_png(Cursor::new(&png)).context("decode PNG tile")?;

    let mut history = TileHistory { imgs: Default::default() };
    history
        .set(date_hours, image)
        .context("set PNG into TileHistory")?;

    Ok(Some(MakebaseResult { x, y, data: history.to_bytes() }))
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use wimage::palette;

    const TILE_SIZE: usize = 1000;
    const TILE_PIXELS: usize = TILE_SIZE * TILE_SIZE;

    fn make_tile(value: u8) -> PalettedImage {
        PalettedImage {
            width: TILE_SIZE,
            height: TILE_SIZE,
            indices: vec![value; TILE_PIXELS],
        }
    }

    fn png_for(value: u8) -> Vec<u8> {
        make_tile(value).to_png().unwrap()
    }

    #[test]
    fn process_job_creates_full_image_at_date_zero() {
        let png = png_for(palette::WHITE);
        let result = process_job(MakebaseJob {
            x: 3,
            y: 4,
            date_hours: DateHours(0),
            png,
        })
        .unwrap()
        .unwrap();

        assert_eq!(result.x, 3);
        assert_eq!(result.y, 4);

        let th = TileHistory::from_bytes(&result.data).unwrap();
        assert_eq!(th.list(), vec![DateHours(0)]);
        let img = th.get(DateHours(0)).unwrap().unwrap();
        assert!(img.indices.iter().all(|&v| v == palette::WHITE));
    }

    #[test]
    fn process_job_creates_full_image_at_non_zero_date() {
        let png = png_for(7);
        let result = process_job(MakebaseJob {
            x: 0,
            y: 0,
            date_hours: DateHours(500),
            png,
        })
        .unwrap()
        .unwrap();

        let th = TileHistory::from_bytes(&result.data).unwrap();
        assert_eq!(th.list(), vec![DateHours(500)]);
        let img = th.get(DateHours(500)).unwrap().unwrap();
        assert!(img.indices.iter().all(|&v| v == 7));
    }
}