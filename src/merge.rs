use std::thread;
use std::time::Instant;

use anyhow::Context;
use crossbeam_channel::bounded;
use rusqlite::{params, Connection};

use wimage::palette;
use wimage::tilehistory::DateHours;
use wimage::{PalettedImage, tilehistory::TileHistory};

// ─── Types ───────────────────────────────────────────────────────────────────

struct MergeJob {
    z: i32,
    x: i32,
    y: i32,
    date_hours: DateHours,
    /// 4 source TileHistory blobs from z-1: [TL, TR, BL, BR], None if tile missing in DB
    tiles: [Option<Vec<u8>>; 4],
    /// Existing TileHistory blob at (z, x, y) if any
    existing: Option<Vec<u8>>,
}

/// Job for a "double merge" that skips an intermediate z-level.
/// Reads 16 tiles from z-2 (a 4×4 grid), does two rounds of stitch+downscale,
/// and writes directly to z (saving one intermediate z-level from storage).
struct MergeJobDouble {
    z: i32,
    x: i32,
    y: i32,
    date_hours: DateHours,
    /// 16 source TileHistory blobs from z-2, row-major (y then x):
    /// [TL00, TL01, TL10, TL11,  TR00, TR01, TR10, TR11,  BL00, BL01, BL10, BL11,  BR00, BR01, BR10, BR11]
    /// Each group of 4 corresponds to a quadrant at the intermediate level.
    tiles: [Option<Vec<u8>>; 16],
    /// Existing TileHistory blob at (z, x, y) if any
    existing: Option<Vec<u8>>,
}

struct MergeResult {
    z: i32,
    x: i32,
    y: i32,
    data: Vec<u8>,
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Weights for downscale_mode_weighted: visible colours win over background.
fn make_weights() -> [u32; 256] {
    let mut w = [100u32; 256];
    w[palette::TRANSPARENT as usize] = 0;
    w
}

/// Return an empty 1000×1000 tile filled with TRANSPARENT.
fn empty_tile() -> PalettedImage {
    PalettedImage {
        width: 1000,
        height: 1000,
        indices: vec![palette::TRANSPARENT; 1000 * 1000],
    }
}

/// Stitch four 1000×1000 tiles into a 2000×2000 canvas, then downscale to 1000×1000.
fn stitch_and_downscale(tiles: [PalettedImage; 4]) -> PalettedImage {
    let mut canvas = vec![palette::TRANSPARENT; 2000 * 2000];

    // top-left  (0..1000, 0..1000)
    for row in 0..1000 {
        let src_off = row * 1000;
        let dst_off = row * 2000;
        canvas[dst_off..dst_off + 1000].copy_from_slice(&tiles[0].indices[src_off..src_off + 1000]);
    }
    // top-right  (1000..2000, 0..1000)
    for row in 0..1000 {
        let src_off = row * 1000;
        let dst_off = row * 2000 + 1000;
        canvas[dst_off..dst_off + 1000].copy_from_slice(&tiles[1].indices[src_off..src_off + 1000]);
    }
    // bottom-left  (0..1000, 1000..2000)
    for row in 0..1000 {
        let src_off = row * 1000;
        let dst_off = (row + 1000) * 2000;
        canvas[dst_off..dst_off + 1000].copy_from_slice(&tiles[2].indices[src_off..src_off + 1000]);
    }
    // bottom-right  (1000..2000, 1000..2000)
    for row in 0..1000 {
        let src_off = row * 1000;
        let dst_off = (row + 1000) * 2000 + 1000;
        canvas[dst_off..dst_off + 1000].copy_from_slice(&tiles[3].indices[src_off..src_off + 1000]);
    }

    let big = PalettedImage {
        width: 2000,
        height: 2000,
        indices: canvas,
    };

    let weights = make_weights();
    big.downscale_mode_weighted(&weights, 2)
}

// ─── Worker: process one job ────────────────────────────────────────────────

/// Decode the 4 source tiles, stitch+downscale, update TileHistory, return result blob.
fn process_job(job: MergeJob) -> MergeResult {
    let date_hours = job.date_hours;

    // Decode each source tile image at the target date_hours; missing → empty TRANSPARENT tile.
    let decode_tile = |blob: Option<Vec<u8>>| -> PalettedImage {
        match blob {
            Some(data) => {
                let th = TileHistory::from_bytes(&data).expect("valid TileHistory");
                match th.get(date_hours) {
                    Ok(img) => img,
                    Err(_) => empty_tile(),
                }
            }
            None => empty_tile(),
        }
    };

    let tiles = [
        decode_tile(job.tiles[0].clone()),
        decode_tile(job.tiles[1].clone()),
        decode_tile(job.tiles[2].clone()),
        decode_tile(job.tiles[3].clone()),
    ];

    // Check if all tiles are empty (all missing + no data at date_hours)
    let all_empty = tiles.iter().all(|t| {
        t.indices.iter().all(|&v| v == palette::TRANSPARENT)
    });
    if all_empty {
        // Nothing to merge; skip but return a valid result with empty TileHistory
        return MergeResult {
            z: job.z,
            x: job.x,
            y: job.y,
            data: vec![],
        };
    }

    let merged = stitch_and_downscale(tiles);

    // Build or update TileHistory
    let mut th = match &job.existing {
        Some(data) => TileHistory::from_bytes(data).expect("valid TileHistory"),
        None => TileHistory {
            imgs: Default::default(),
        },
    };

    th.set(date_hours, merged)
        .expect("TileHistory::set should succeed");

    MergeResult {
        z: job.z,
        x: job.x,
        y: job.y,
        data: th.to_bytes(),
    }
}

/// Decode a source tile blob at the target date_hours; missing → empty TRANSPARENT tile.
fn decode_tile_for_date(blob: Option<Vec<u8>>, date_hours: DateHours) -> PalettedImage {
    match blob {
        Some(data) => {
            let th = TileHistory::from_bytes(&data).expect("valid TileHistory");
            match th.get(date_hours) {
                Ok(img) => img,
                Err(_) => empty_tile(),
            }
        }
        None => empty_tile(),
    }
}

/// Process a double-merge job: 16 tiles from z-2 → two rounds of stitch+downscale → z.
fn process_job_double(job: MergeJobDouble) -> MergeResult {
    let date_hours = job.date_hours;

    // Decode all 16 source tiles.
    let tiles: Vec<PalettedImage> = job.tiles.iter()
        .map(|t| decode_tile_for_date(t.clone(), date_hours))
        .collect();

    // Check if all tiles are empty.
    let all_empty = tiles.iter().all(|t| {
        t.indices.iter().all(|&v| v == palette::TRANSPARENT)
    });
    if all_empty {
        return MergeResult {
            z: job.z,
            x: job.x,
            y: job.y,
            data: vec![],
        };
    }

    // Round 1: 16 tiles (4×4 at z-2) → 4 intermediate tiles (2×2 at z-1).
    // Layout of 16 tiles (row-major):
    //   [0,  1,  4,  5]     [ 8,  9, 12, 13]
    //   [2,  3,  6,  7]     [10, 11, 14, 15]
    //
    // Quadrants at intermediate level:
    //   TL: tiles[0..4]  (z-1 children for TL quadrant)
    //   TR: tiles[4..8]
    //   BL: tiles[8..12]
    //   BR: tiles[12..16]
    let intermediates: [PalettedImage; 4] = [
        stitch_and_downscale([
            tiles[0].clone(), tiles[1].clone(),
            tiles[2].clone(), tiles[3].clone(),
        ]),
        stitch_and_downscale([
            tiles[4].clone(), tiles[5].clone(),
            tiles[6].clone(), tiles[7].clone(),
        ]),
        stitch_and_downscale([
            tiles[8].clone(), tiles[9].clone(),
            tiles[10].clone(), tiles[11].clone(),
        ]),
        stitch_and_downscale([
            tiles[12].clone(), tiles[13].clone(),
            tiles[14].clone(), tiles[15].clone(),
        ]),
    ];

    // Round 2: 4 intermediate tiles → final tile at z.
    let merged = stitch_and_downscale(intermediates);

    // Build or update TileHistory.
    let mut th = match &job.existing {
        Some(data) => TileHistory::from_bytes(data).expect("valid TileHistory"),
        None => TileHistory {
            imgs: Default::default(),
        },
    };

    th.set(date_hours, merged)
        .expect("TileHistory::set should succeed");

    MergeResult {
        z: job.z,
        x: job.x,
        y: job.y,
        data: th.to_bytes(),
    }
}

// ─── Pipeline ───────────────────────────────────────────────────────────────

/// Run the full merge pipeline for a single z-level.
///
/// The reader streams source tiles in x-strips (50 parent tiles at a time) to
/// keep memory bounded — only ~250 TileHistory blobs are held in memory
/// simultaneously, regardless of map size.
fn merge_level(db_path: &str, z: i32, date_hours: DateHours) -> anyhow::Result<()> {
    let source_z = z + 1;

    // Open a connection to discover the source tile bounding box.
    let conn = Connection::open(db_path).context("open db")?;
    conn.pragma_update(None, "journal_mode", "WAL")?;

    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM tiles WHERE z = ?1",
        params![source_z],
        |r| r.get(0),
    )?;
    if count == 0 {
        return Ok(());
    }

    let (min_x, max_x, min_y, max_y) = conn.query_row(
        "SELECT MIN(x), MAX(x), MIN(y), MAX(y) FROM tiles WHERE z = ?1",
        params![source_z],
        |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?, r.get::<_, i64>(3)?)),
    )?;

    // Parent tile range (cast to i32; tile coords are well within i32 range).
    let parent_min_x = (min_x / 2) as i32;
    let parent_max_x = (max_x / 2) as i32;
    let parent_min_y = (min_y / 2) as i32;
    let parent_max_y = (max_y / 2) as i32;

    drop(conn); // bounding-box query done; reader will open its own connection.

    // Channels.
    let (job_tx, job_rx) = bounded::<MergeJob>(2000);
    let (res_tx, res_rx) = bounded::<MergeResult>(2000);

    let db_path = db_path.to_string();

    // ── Reader thread: streams jobs from SQLite in x-strips ──
    const STRIP_SIZE: i32 = 50; // parent x-values per strip

    let reader_path = db_path.clone();
    let reader = thread::spawn(move || {
        let conn = Connection::open(&reader_path).expect("reader open db");
        conn.pragma_update(None, "journal_mode", "WAL").expect("WAL mode");

        let mut fetch_source = conn.prepare_cached(
            "SELECT x, y, data FROM tiles WHERE z = ?1 AND y = ?2 AND x >= ?3 AND x <= ?4",
        ).expect("prepare fetch_source");

        let mut fetch_existing = conn.prepare_cached(
            "SELECT x, y, data FROM tiles WHERE z = ?1 AND y = ?2 AND x >= ?3 AND x <= ?4",
        ).expect("prepare fetch_existing");

        for py in parent_min_y..=parent_max_y {
            let sy0 = 2 * py;
            let sy1 = 2 * py + 1;

            // Process parent x in strips.
            for strip_start in (parent_min_x..=parent_max_x).step_by(STRIP_SIZE as usize) {
                let strip_end = std::cmp::min(strip_start + STRIP_SIZE - 1, parent_max_x);

                // Source x range needed: 2*strip_start .. 2*strip_end+1.
                let sx_min = 2 * strip_start;
                let sx_max = 2 * strip_end + 1;

                // Query source tiles for this strip (two y-rows).
                let mut source_map: std::collections::HashMap<(i32, i32), Vec<u8>> =
                    std::collections::HashMap::new();

                for &sy in &[sy0, sy1] {
                    let rows = fetch_source.query_map(
                        params![source_z, sy, sx_min, sx_max],
                        |r| Ok((r.get::<_, i32>(0).unwrap(), r.get::<_, i32>(1).unwrap(), r.get::<_, Vec<u8>>(2).unwrap())),
                    ).expect("query source tiles");

                    for row in rows {
                        let (x, y, data) = row.expect("row");
                        source_map.insert((x, y), data);
                    }
                }

                // Query existing tiles for this strip.
                let mut existing_map: std::collections::HashMap<(i32, i32), Vec<u8>> =
                    std::collections::HashMap::new();

                let rows = fetch_existing.query_map(
                    params![z, py, strip_start, strip_end],
                    |r| Ok((r.get::<_, i32>(0).unwrap(), r.get::<_, i32>(1).unwrap(), r.get::<_, Vec<u8>>(2).unwrap())),
                ).expect("query existing tiles");

                for row in rows {
                    let (x, y, data) = row.expect("row");
                    existing_map.insert((x, y), data);
                }

                // Assemble and send jobs for this strip.
                for px in strip_start..=strip_end {
                    let sx0 = 2 * px;
                    let sx1 = sx0 + 1;

                    let tl = source_map.get(&(sx0, sy0)).cloned();
                    let tr = source_map.get(&(sx1, sy0)).cloned();
                    let bl = source_map.get(&(sx0, sy1)).cloned();
                    let br = source_map.get(&(sx1, sy1)).cloned();

                    if tl.is_none() && tr.is_none() && bl.is_none() && br.is_none() {
                        continue;
                    }

                    let existing = existing_map.get(&(px, py)).cloned();

                    job_tx.send(MergeJob {
                        z,
                        x: px,
                        y: py,
                        date_hours,
                        tiles: [tl, tr, bl, br],
                        existing,
                    }).expect("send job");
                }

                // source_map and existing_map are dropped here — memory freed per strip.
            }
        }
    });

    // ── Worker threads ──
    let n_workers = num_cpus::get();
    let mut workers = Vec::new();
    for _ in 0..n_workers {
        let job_rx = job_rx.clone();
        let res_tx = res_tx.clone();
        workers.push(thread::spawn(move || {
            for job in job_rx {
                let result = process_job(job);
                if !result.data.is_empty() {
                    res_tx.send(result).unwrap();
                }
            }
        }));
    }
    drop(res_tx);

    // ── Writer thread: batched inserts/updates ──
    let writer = thread::spawn(move || {
        let mut conn = Connection::open(&db_path).unwrap();
        conn.pragma_update(None, "synchronous", "NORMAL").unwrap();

        let mut batch = Vec::with_capacity(2000);
        let flush = |conn: &mut Connection, batch: &mut Vec<MergeResult>| {
            let tx = conn.transaction().unwrap();
            {
                let mut stmt = tx.prepare_cached(
                    "INSERT OR REPLACE INTO tiles (z, x, y, data) VALUES (?1, ?2, ?3, ?4)",
                ).unwrap();
                for r in batch.drain(..) {
                    stmt.execute(params![r.z, r.x, r.y, r.data]).unwrap();
                }
            }
            tx.commit().unwrap();
        };

        // Metrics
        let start = Instant::now();
        let mut processed: usize = 0;
        let mut last_print = start;

        for r in res_rx {
            batch.push(r);
            processed += 1;
            if batch.len() >= 2000 {
                flush(&mut conn, &mut batch);
                if last_print.elapsed() >= std::time::Duration::from_secs(10) {
                    let elapsed = start.elapsed();
                    let rate = processed as f64 / elapsed.as_secs_f64();
                    eprintln!(
                        "  {} jobs processed in {:.1}s ({:.0} jobs/s)",
                        processed, elapsed.as_secs_f64(), rate
                    );
                    last_print = Instant::now();
                }
            }
        }
        if !batch.is_empty() {
            flush(&mut conn, &mut batch);
        }

        // Final metrics
        let elapsed = start.elapsed();
        let rate = processed as f64 / elapsed.as_secs_f64();
        eprintln!(
            "  {} jobs processed in {:.1}s ({:.0} jobs/s)",
            processed, elapsed.as_secs_f64(), rate
        );
    });

    reader.join().unwrap();
    for w in workers {
        w.join().unwrap();
    }
    writer.join().unwrap();

    Ok(())
}

/// Like [`merge_level`], but skips an intermediate z-level.
///
/// Reads 16 source tiles from z-2 (a 4×4 grid), does two rounds of
/// stitch+downscale, and writes directly to z. This saves storage by
/// never persisting the skipped level.
fn merge_level_double(db_path: &str, z: i32, date_hours: DateHours) -> anyhow::Result<()> {
    let source_z = z + 2;

    let conn = Connection::open(db_path).context("open db")?;
    conn.pragma_update(None, "journal_mode", "WAL")?;

    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM tiles WHERE z = ?1",
        params![source_z],
        |r| r.get(0),
    )?;
    if count == 0 {
        return Ok(());
    }

    let (min_x, max_x, min_y, max_y) = conn.query_row(
        "SELECT MIN(x), MAX(x), MIN(y), MAX(y) FROM tiles WHERE z = ?1",
        params![source_z],
        |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?, r.get::<_, i64>(3)?)),
    )?;

    // Grandparent tile range (divide by 4).
    let gp_min_x = (min_x / 4) as i32;
    let gp_max_x = (max_x / 4) as i32;
    let gp_min_y = (min_y / 4) as i32;
    let gp_max_y = (max_y / 4) as i32;

    drop(conn);

    let (job_tx, job_rx) = bounded::<MergeJobDouble>(2000);
    let (res_tx, res_rx) = bounded::<MergeResult>(2000);

    let db_path = db_path.to_string();

    // ── Reader thread ──
    const STRIP_SIZE: i32 = 50;

    let reader_path = db_path.clone();
    let reader = thread::spawn(move || {
        let conn = Connection::open(&reader_path).expect("reader open db");
        conn.pragma_update(None, "journal_mode", "WAL").expect("WAL mode");

        let mut fetch_source = conn.prepare_cached(
            "SELECT x, y, data FROM tiles WHERE z = ?1 AND y = ?2 AND x >= ?3 AND x <= ?4",
        ).expect("prepare fetch_source");

        let mut fetch_existing = conn.prepare_cached(
            "SELECT x, y, data FROM tiles WHERE z = ?1 AND y = ?2 AND x >= ?3 AND x <= ?4",
        ).expect("prepare fetch_existing");

        for gpy in gp_min_y..=gp_max_y {
            let sy0 = 4 * gpy;
            let sy1 = sy0 + 1;
            let sy2 = sy0 + 2;
            let sy3 = sy0 + 3;

            for strip_start in (gp_min_x..=gp_max_x).step_by(STRIP_SIZE as usize) {
                let strip_end = std::cmp::min(strip_start + STRIP_SIZE - 1, gp_max_x);

                // Source x range: 4*strip_start .. 4*strip_end+3.
                let sx_min = 4 * strip_start;
                let sx_max = 4 * strip_end + 3;

                // Query source tiles for this strip (four y-rows).
                let mut source_map: std::collections::HashMap<(i32, i32), Vec<u8>> =
                    std::collections::HashMap::new();

                for &sy in &[sy0, sy1, sy2, sy3] {
                    let rows = fetch_source.query_map(
                        params![source_z, sy, sx_min, sx_max],
                        |r| Ok((r.get::<_, i32>(0).unwrap(), r.get::<_, i32>(1).unwrap(), r.get::<_, Vec<u8>>(2).unwrap())),
                    ).expect("query source tiles");

                    for row in rows {
                        let (x, y, data) = row.expect("row");
                        source_map.insert((x, y), data);
                    }
                }

                // Query existing tiles for this strip.
                let mut existing_map: std::collections::HashMap<(i32, i32), Vec<u8>> =
                    std::collections::HashMap::new();

                let rows = fetch_existing.query_map(
                    params![z, gpy, strip_start, strip_end],
                    |r| Ok((r.get::<_, i32>(0).unwrap(), r.get::<_, i32>(1).unwrap(), r.get::<_, Vec<u8>>(2).unwrap())),
                ).expect("query existing tiles");

                for row in rows {
                    let (x, y, data) = row.expect("row");
                    existing_map.insert((x, y), data);
                }

                // Assemble and send jobs.
                for gpx in strip_start..=strip_end {
                    let sx0 = 4 * gpx;

                    // 16 tiles in row-major order:
                    // Row 0: (sx0, sy0), (sx0+1, sy0), (sx0+2, sy0), (sx0+3, sy0)
                    // Row 1: (sx0, sy1), (sx0+1, sy1), (sx0+2, sy1), (sx0+3, sy1)
                    // Row 2: (sx0, sy2), (sx0+1, sy2), (sx0+2, sy2), (sx0+3, sy2)
                    // Row 3: (sx0, sy3), (sx0+1, sy3), (sx0+2, sy3), (sx0+3, sy3)
                    let mut tiles: [Option<Vec<u8>>; 16] = Default::default();
                    let mut any = false;
                    for dy in 0..4i32 {
                        for dx in 0..4i32 {
                            let idx = (dy * 4 + dx) as usize;
                            tiles[idx] = source_map.get(&(sx0 + dx, sy0 + dy)).cloned();
                            if tiles[idx].is_some() {
                                any = true;
                            }
                        }
                    }

                    if !any {
                        continue;
                    }

                    let existing = existing_map.get(&(gpx, gpy)).cloned();

                    job_tx.send(MergeJobDouble {
                        z,
                        x: gpx,
                        y: gpy,
                        date_hours,
                        tiles,
                        existing,
                    }).expect("send job");
                }
            }
        }
    });

    // ── Worker threads ──
    let n_workers = num_cpus::get();
    let mut workers = Vec::new();
    for _ in 0..n_workers {
        let job_rx = job_rx.clone();
        let res_tx = res_tx.clone();
        workers.push(thread::spawn(move || {
            for job in job_rx {
                let result = process_job_double(job);
                if !result.data.is_empty() {
                    res_tx.send(result).unwrap();
                }
            }
        }));
    }
    drop(res_tx);

    // ── Writer thread ──
    let writer = thread::spawn(move || {
        let mut conn = Connection::open(&db_path).unwrap();
        conn.pragma_update(None, "synchronous", "NORMAL").unwrap();

        let mut batch = Vec::with_capacity(2000);
        let flush = |conn: &mut Connection, batch: &mut Vec<MergeResult>| {
            let tx = conn.transaction().unwrap();
            {
                let mut stmt = tx.prepare_cached(
                    "INSERT OR REPLACE INTO tiles (z, x, y, data) VALUES (?1, ?2, ?3, ?4)",
                ).unwrap();
                for r in batch.drain(..) {
                    stmt.execute(params![r.z, r.x, r.y, r.data]).unwrap();
                }
            }
            tx.commit().unwrap();
        };

        let start = Instant::now();
        let mut processed: usize = 0;
        let mut last_print = start;

        for r in res_rx {
            batch.push(r);
            processed += 1;
            if batch.len() >= 2000 {
                flush(&mut conn, &mut batch);
                if last_print.elapsed() >= std::time::Duration::from_secs(10) {
                    let elapsed = start.elapsed();
                    let rate = processed as f64 / elapsed.as_secs_f64();
                    eprintln!(
                        "  {} jobs processed in {:.1}s ({:.0} jobs/s)",
                        processed, elapsed.as_secs_f64(), rate
                    );
                    last_print = Instant::now();
                }
            }
        }
        if !batch.is_empty() {
            flush(&mut conn, &mut batch);
        }

        let elapsed = start.elapsed();
        let rate = processed as f64 / elapsed.as_secs_f64();
        eprintln!(
            "  {} jobs processed in {:.1}s ({:.0} jobs/s)",
            processed, elapsed.as_secs_f64(), rate
        );
    });

    reader.join().unwrap();
    for w in workers {
        w.join().unwrap();
    }
    writer.join().unwrap();

    Ok(())
}

// ─── Public API ──────────────────────────────────────────────────────────────

/// Merge tiles from z=11 down to z=0 for the given DateHours.
///
/// z=10 is skipped (not stored) — z=9 is produced via a double-merge
/// directly from z=11 to save storage.
pub fn merge(db_path: &str, date_hours: DateHours) -> anyhow::Result<()> {
    // z=9: double-merge from z=11 (skips storing z=10).
    eprintln!("Merging level z=9 (double from z=11) ...");
    merge_level_double(db_path, 9, date_hours)?;

    // z=8 .. z=0: normal merge from z+1.
    for z in (0..=8).rev() {
        eprintln!("Merging level z={z} ...");
        merge_level(db_path, z, date_hours)?;
    }
    Ok(())
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use wimage::palette;

    fn make_tile(value: u8) -> PalettedImage {
        PalettedImage {
            width: 1000,
            height: 1000,
            indices: vec![value; 1000 * 1000],
        }
    }

 
    fn make_tilehistory_with_img(img: PalettedImage, date_hours: DateHours) -> Vec<u8> {
        let mut th = TileHistory {
            imgs: Default::default(),
        };
        th.set(date_hours, img).unwrap();
        th.to_bytes()
    }

    // ─── empty_tile ───

    #[test]
    fn test_empty_tile_is_transparent() {
        let t = empty_tile();
        assert_eq!(t.width, 1000);
        assert_eq!(t.height, 1000);
        assert!(t.indices.iter().all(|&v| v == palette::TRANSPARENT));
    }

    // ─── stitch_and_downscale ───

    #[test]
    fn test_stitch_uniform_tile() {
        let t = make_tile(42u8);
        let result = stitch_and_downscale([t.clone(), t.clone(), t.clone(), t.clone()]);
        assert_eq!(result.width, 1000);
        assert_eq!(result.height, 1000);
        // All pixels should be 42 (dominant colour).
        assert!(result.indices.iter().all(|&v| v == 42));
    }

    #[test]
    fn test_stitch_different_colors_quadrants() {
        // Each quadrant has a different colour. After 2x2 downscale each output pixel
        // comes from 4 source pixels (one from each quadrant in the center area).
        // The weighted mode should pick the colour with the highest weight.
        // For simple 4-different-colour blocks, each block has 4 unique colours.
        let t0 = make_tile(1u8);  // BLACK
        let t1 = make_tile(2u8);  // DARK_GRAY
        let t2 = make_tile(3u8);  // GRAY
        let t3 = make_tile(4u8);  // LIGHT_GRAY
        let result = stitch_and_downscale([t0, t1, t2, t3]);
        assert_eq!(result.width, 1000);
        assert_eq!(result.height, 1000);
        // Each output pixel is from a 2x2 block of the 2000x2000 canvas.
        // The top-left quadrant of output comes from top-left of canvas (all BLACK=1).
        // So top-left 500x500 should be BLACK.
        // Similarly for other quadrants.
        // Top-left output quadrant: source is 0..1000, 0..1000 of canvas → all tile[0] (BLACK=1)
        for y in 0..500 {
            for x in 0..500 {
                assert_eq!(result.indices[y * 1000 + x], 1, "TL quadrant at ({x},{y})");
            }
        }
        // Top-right output quadrant: source is 1000..2000, 0..1000 → all tile[1] (DARK_GRAY=2)
        for y in 0..500 {
            for x in 500..1000 {
                assert_eq!(result.indices[y * 1000 + x], 2, "TR quadrant at ({x},{y})");
            }
        }
        // Bottom-left: tile[2] (GRAY=3)
        for y in 500..1000 {
            for x in 0..500 {
                assert_eq!(result.indices[y * 1000 + x], 3, "BL quadrant at ({x},{y})");
            }
        }
        // Bottom-right: tile[3] (LIGHT_GRAY=4)
        for y in 500..1000 {
            for x in 500..1000 {
                assert_eq!(result.indices[y * 1000 + x], 4, "BR quadrant at ({x},{y})");
            }
        }
    }

    #[test]
    fn test_stitch_with_transparent_tile() {
        // One tile is transparent, others are BLACK. The BLACK pixels should dominate
        // in blocks where they coexist with transparent.
        let black = make_tile(palette::BLACK);
        let trans = empty_tile();
        let result = stitch_and_downscale([black.clone(), black.clone(), trans, black.clone()]);
        assert_eq!(result.width, 1000);
        assert_eq!(result.height, 1000);
        // Top-left, top-right, bottom-right quadrants should be BLACK.
        // Bottom-left quadrant: source is all transparent → should be transparent.
        for y in 500..1000 {
            for x in 0..500 {
                assert_eq!(
                    result.indices[y * 1000 + x],
                    palette::TRANSPARENT,
                    "BL quadrant should be transparent at ({x},{y})"
                );
            }
        }
    }

    // ─── process_job ───

    #[test]
    fn test_process_job_all_tiles_present() {
        let img = make_tile(42u8);
        let blob = make_tilehistory_with_img(img, DateHours(1));

        let job = MergeJob {
            z: 10,
            x: 0,
            y: 0,
            date_hours: DateHours(1),
            tiles: [Some(blob.clone()), Some(blob.clone()), Some(blob.clone()), Some(blob.clone())],
            existing: None,
        };

        let result = process_job(job);
        assert_eq!(result.z, 10);
        assert_eq!(result.x, 0);
        assert_eq!(result.y, 0);
        assert!(!result.data.is_empty());

        // Verify the result TileHistory can be read back.
        let th = TileHistory::from_bytes(&result.data).unwrap();
        let merged = th.get(DateHours(1)).unwrap();
        assert_eq!(merged.width, 1000);
        assert_eq!(merged.height, 1000);
        assert!(merged.indices.iter().all(|&v| v == 42));
    }

    #[test]
    fn test_process_job_some_missing_tiles() {
        let img = make_tile(42u8);
        let blob = make_tilehistory_with_img(img, DateHours(1));

        // Only top-left tile present; others missing → transparent.
        let job = MergeJob {
            z: 10,
            x: 0,
            y: 0,
            date_hours: DateHours(1),
            tiles: [Some(blob), None, None, None],
            existing: None,
        };

        let result = process_job(job);
        assert!(!result.data.is_empty(), "Should produce data even with missing tiles");

        let th = TileHistory::from_bytes(&result.data).unwrap();
        let merged = th.get(DateHours(1)).unwrap();
        // Top-left quadrant should be 42, rest transparent.
        for y in 0..500 {
            for x in 0..500 {
                assert_eq!(merged.indices[y * 1000 + x], 42, "TL at ({x},{y})");
            }
        }
        for y in 500..1000 {
            for x in 0..1000 {
                assert_eq!(
                    merged.indices[y * 1000 + x],
                    palette::TRANSPARENT,
                    "BL at ({x},{y})"
                );
            }
        }
        for y in 0..500 {
            for x in 500..1000 {
                assert_eq!(
                    merged.indices[y * 1000 + x],
                    palette::TRANSPARENT,
                    "TR at ({x},{y})"
                );
            }
        }
    }

    #[test]
    fn test_process_job_all_missing_skips() {
        let job = MergeJob {
            z: 10,
            x: 0,
            y: 0,
            date_hours: DateHours(1),
            tiles: [None, None, None, None],
            existing: None,
        };

        let result = process_job(job);
        assert!(result.data.is_empty(), "All missing tiles should produce empty result");
    }

    #[test]
    fn test_process_job_updates_existing_history() {
        // Create an existing TileHistory with a version at DateHours(0).
        let existing_img = make_tile(10u8);
        let mut existing_th = TileHistory {
            imgs: Default::default(),
        };
        existing_th.set(DateHours(0), existing_img).unwrap();
        let existing_blob = existing_th.to_bytes();

        let new_img = make_tile(42u8);
        let new_blob = make_tilehistory_with_img(new_img, DateHours(1));

        let job = MergeJob {
            z: 10,
            x: 0,
            y: 0,
            date_hours: DateHours(1),
            tiles: [
                Some(new_blob.clone()),
                Some(new_blob.clone()),
                Some(new_blob.clone()),
                Some(new_blob.clone()),
            ],
            existing: Some(existing_blob),
        };

        let result = process_job(job);
        let th = TileHistory::from_bytes(&result.data).unwrap();

        // Should have both versions.
        assert_eq!(th.list().len(), 2);

        // Version at DateHours(0) should still be 10.
        let v0 = th.get(DateHours(0)).unwrap();
        assert!(v0.indices.iter().all(|&v| v == 10));

        // Version at DateHours(1) should be 42.
        let v1 = th.get(DateHours(1)).unwrap();
        assert!(v1.indices.iter().all(|&v| v == 42));
    }

    // ─── make_weights ───

    #[test]
    fn test_weights_background_lower() {
        let w = make_weights();
        assert!(w[palette::TRANSPARENT as usize] < w[1 as usize], "TRANSPARENT should have lower weight");
        assert!(w[palette::WHITE as usize] < w[1 as usize], "WHITE should have lower weight");
    }

    // ─── End-to-end with SQLite ───

    fn create_test_db(
        path: &str,
    ) -> anyhow::Result<()> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS tiles (
                z INTEGER NOT NULL,
                x INTEGER NOT NULL,
                y INTEGER NOT NULL,
                data BLOB NOT NULL,
                PRIMARY KEY (z, x, y)
            );
            CREATE TABLE IF NOT EXISTS versions (
                date INTEGER PRIMARY KEY,
                original_file TEXT
            );",
        )?;
        Ok(())
    }

    #[test]
    fn test_merge_single_level() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_str().unwrap();
        create_test_db(path).unwrap();

        // Insert 4 tiles at z=11 (x=0..1, y=0..1) with a solid colour.
        let img = make_tile(42u8);
        let blob = make_tilehistory_with_img(img, DateHours(1));

        let conn = Connection::open(path).unwrap();
        for x in 0..=1 {
            for y in 0..=1 {
                conn.execute(
                    "INSERT INTO tiles (z, x, y, data) VALUES (11, ?1, ?2, ?3)",
                    params![x, y, blob.clone()],
                )
                .unwrap();
            }
        }
        drop(conn);

        // Run merge for level z=10 only (we'll test the full pipeline in another test).
        merge_level(path, 10, DateHours(1)).unwrap();

        // Verify: tile (10, 0, 0) should exist.
        let conn = Connection::open(path).unwrap();
        let mut stmt = conn.prepare(
            "SELECT data FROM tiles WHERE z = 10 AND x = 0 AND y = 0"
        ).unwrap();
        let data: Vec<u8> = stmt.query_row(params![], |r| r.get(0)).unwrap();
        assert!(!data.is_empty());

        let th = TileHistory::from_bytes(&data).unwrap();
        let merged = th.get(DateHours(1)).unwrap();
        assert_eq!(merged.width, 1000);
        assert_eq!(merged.height, 1000);
        assert!(merged.indices.iter().all(|&v| v == 42));
    }

    #[test]
    fn test_merge_with_missing_parent_tiles() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_str().unwrap();
        create_test_db(path).unwrap();

        // Insert only 2 tiles at z=11: (0,0) and (1,0). Missing (0,1) and (1,1).
        let img = make_tile(42u8);
        let blob = make_tilehistory_with_img(img, DateHours(1));

        let conn = Connection::open(path).unwrap();
        conn.execute(
            "INSERT INTO tiles (z, x, y, data) VALUES (11, 0, 0, ?1)",
            params![blob.clone()],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO tiles (z, x, y, data) VALUES (11, 1, 0, ?1)",
            params![blob],
        )
        .unwrap();
        drop(conn);

        merge_level(path, 10, DateHours(1)).unwrap();

        // Verify: (10, 0, 0) exists with top half = 42, bottom half = transparent.
        let conn = Connection::open(path).unwrap();
        let mut stmt = conn.prepare(
            "SELECT data FROM tiles WHERE z = 10 AND x = 0 AND y = 0"
        ).unwrap();
        let data: Vec<u8> = stmt.query_row(params![], |r| r.get(0)).unwrap();
        let th = TileHistory::from_bytes(&data).unwrap();
        let merged = th.get(DateHours(1)).unwrap();

        // Top half (y=0..500) should be 42.
        for y in 0..500 {
            for x in 0..1000 {
                assert_eq!(merged.indices[y * 1000 + x], 42, "Top at ({x},{y})");
            }
        }
        // Bottom half (y=500..1000) should be transparent.
        for y in 500..1000 {
            for x in 0..1000 {
                assert_eq!(
                    merged.indices[y * 1000 + x],
                    palette::TRANSPARENT,
                    "Bottom at ({x},{y})"
                );
            }
        }
    }

    #[test]
    fn test_merge_multi_level() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_str().unwrap();
        create_test_db(path).unwrap();

        // Insert 16 tiles at z=11 covering (0..=3, 0..=3).
        // merge() does a double-merge for z=9 (from z=11), then normal merges z=8..0.
        let img = make_tile(42u8);
        let blob = make_tilehistory_with_img(img, DateHours(1));

        let conn = Connection::open(path).unwrap();
        for x in 0..=3 {
            for y in 0..=3 {
                conn.execute(
                    "INSERT INTO tiles (z, x, y, data) VALUES (11, ?1, ?2, ?3)",
                    params![x, y, blob.clone()],
                )
                .unwrap();
            }
        }
        drop(conn);

        // Run full merge (z=9 double from z=11, then z=8..0).
        merge(path, DateHours(1)).unwrap();

        // Verify all levels have tile (0, 0). z=10 is skipped, so z=0..9 (10 levels).
        let conn = Connection::open(path).unwrap();
        let mut stmt = conn.prepare(
            "SELECT COUNT(*) FROM tiles WHERE z <= 9 AND x = 0 AND y = 0"
        ).unwrap();
        let count: i64 = stmt.query_row(params![], |r| r.get(0)).unwrap();
        assert_eq!(count, 10, "Expected 10 levels (z=0..9), got {count}");

        // Verify z=10 was NOT created.
        let count_z10: i64 = conn.query_row(
            "SELECT COUNT(*) FROM tiles WHERE z = 10",
            [],
            |r| r.get(0),
        ).unwrap();
        assert_eq!(count_z10, 0, "z=10 should not exist (skipped)");

        // Verify each level produces a valid 1000x1000 image.
        let mut stmt = conn.prepare(
            "SELECT z, data FROM tiles WHERE x = 0 AND y = 0 AND z <= 9"
        ).unwrap();
        let rows: Vec<(i32, Vec<u8>)> = stmt
            .query_map(params![], |r| Ok((r.get(0).unwrap(), r.get(1).unwrap())))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();

        for (z, data) in rows {
            let th = TileHistory::from_bytes(&data).unwrap();
            let merged = th.get(DateHours(1)).unwrap();
            assert_eq!(merged.width, 1000, "z={z}: width should be 1000");
            assert_eq!(merged.height, 1000, "z={z}: height should be 1000");
            assert_eq!(merged.indices[0], 42, "z={z}: top-left pixel should be 42");
        }
    }

    #[test]
    fn test_merge_no_tiles_at_source_level() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_str().unwrap();
        create_test_db(path).unwrap();

        // No tiles at z=11 → merge should complete without error.
        merge(path, DateHours(1)).unwrap();

        // No tiles should have been created.
        let conn = Connection::open(path).unwrap();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM tiles",
            [],
            |r| r.get(0),
        ).unwrap();
        assert_eq!(count, 0);
    }
}
