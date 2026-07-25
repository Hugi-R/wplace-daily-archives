/// Implements the `merge -t DATEHOUR INPUTDB` command.
/// The merge operation take images on a x,y,z grid from INPUTDB and merge them into a single image.
/// The images are stored in TileHistory format, which is a time series of images for a given tile. DATEHOUR is the time to process.
///
/// The `wimage` crate already implements everything needed to manipulate the images (`PalettedImage`, `TileHistory`, `downscale_mode_weighted_2x2`, ...).
/// 
/// ## Merge:
/// For a Z level, take the 2x2 grid from Z-1 and merge them, forming a single image.
///
/// | Z levels | Processing |
/// | 11 | original, already exist, skip |
/// | 10-0 | 2x2 merge from z-1 |
/// Level z-1 need to be finished before starting level z.
/// To save on storage, we skip z=10 and merge directly from z=11 to z=9, which is a 4x4 merge. This is done by the `MergeJobDouble` job type.
///
/// ### Processing
/// #### Single Reader
/// - Read z-1 TileHistory from sqlite (2x2 grid). Note the tiles may not exist. If all tiles are missing skip the job. If at least one tile exists treat the missing ones as empty.
/// - Read the z TileHistory from sqlite, its content will be updated by the job. Note the tile may not exist/be empty.
/// - Reads can be batched.
/// - Send jobs to crossbeam channel.
///
/// #### Many Workers
/// - read job channel.
/// - decode+decompress TileHistory image for DATEHOUR.
/// - merge images.
/// - update result TileHistory with the new image for DATEHOUR.
/// - send to result channel, or skip if the result is identical to the existing version for DATEHOUR.
///
/// #### Single Writer
/// - read result channel.
/// - prepare batch transaction and wait for timer/enough results.
/// - run transaction.
/// 
/// ## Input
/// - SQLite db of tilehistory for z=11
/// - the datehour time T to process
///
/// The sqlite db has these tables:
/// ```sql
/// CREATE TABLE IF NOT EXISTS tiles (
///     z INTEGER NOT NULL,
///     x INTEGER NOT NULL,
///     y INTEGER NOT NULL,
///     data BLOB NOT NULL,
///     PRIMARY KEY (z, x, y)
/// );
///
/// CREATE TABLE IF NOT EXISTS versions (
///     date INTEGER PRIMARY KEY,
///     original_file TEXT
/// );
/// ```
/// The data blob is a `TileHistory` from `wimage`
///
/// ## Output
/// - the same db, but with z<=9 populated for DATEHOUR.

use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, OnceLock,
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use crossbeam_channel::{bounded, Receiver, SendTimeoutError, Sender};
use rusqlite::{params, Connection};

use wimage::palette;
use wimage::tilehistory::{DateHours, TileHistory};
use wimage::PalettedImage;

const TILE_SIZE: usize = 1000;
const HALF_TILE_SIZE: usize = TILE_SIZE / 2;
const TILE_PIXELS: usize = TILE_SIZE * TILE_SIZE;

const STRIP_SIZE: i32 = 32;
const WRITE_BATCH_SIZE: usize = 256;

// Keep queue memory bounded. TileHistory blobs can be large.
const DEFAULT_MAX_WORKERS: usize = 16;

// ─── Types ───────────────────────────────────────────────────────────────────

struct MergeJob {
    z: i32,
    x: i32,
    y: i32,
    date_hours: DateHours,
    tiles: [Option<Vec<u8>>; 4], // TL, TR, BL, BR
    existing: Option<Vec<u8>>,
}

struct MergeJobDouble {
    z: i32,
    x: i32,
    y: i32,
    date_hours: DateHours,

    // True 4×4 row-major order:
    //
    //  0   1   2   3
    //  4   5   6   7
    //  8   9  10  11
    // 12  13  14  15
    //
    tiles: [Option<Vec<u8>>; 16],
    existing: Option<Vec<u8>>,
}

struct MergeResult {
    z: i32,
    x: i32,
    y: i32,
    data: Vec<u8>,
}

#[derive(Clone, Copy)]
struct Bounds {
    min_x: i32,
    max_x: i32,
    min_y: i32,
    max_y: i32,
}

// ─── Palette / image helpers ─────────────────────────────────────────────────

fn make_weights() -> [u32; 256] {
    let mut weights = [100u32; 256];

    // Fully transparent pixels should never win a mode calculation.
    weights[palette::TRANSPARENT as usize] = 0;

    // This allow the more colourful pixels to win over white/black in case of equal counts.
    // Improving sligtly the visual.
    weights[palette::WHITE as usize] = 90;
    weights[palette::BLACK as usize] = 90;

    weights
}

fn downscale_weights() -> &'static [u32; 256] {
    static WEIGHTS: OnceLock<[u32; 256]> = OnceLock::new();
    WEIGHTS.get_or_init(make_weights)
}

fn empty_tile() -> PalettedImage {
    PalettedImage {
        width: TILE_SIZE,
        height: TILE_SIZE,
        indices: vec![palette::TRANSPARENT; TILE_PIXELS],
    }
}

fn validate_tile(tile: &PalettedImage) -> Result<()> {
    if tile.width as usize != TILE_SIZE
        || tile.height as usize != TILE_SIZE
        || tile.indices.len() != TILE_PIXELS
    {
        bail!(
            "expected a {}×{} tile with {} pixels, got {}×{} with {} pixels",
            TILE_SIZE,
            TILE_SIZE,
            TILE_PIXELS,
            tile.width,
            tile.height,
            tile.indices.len(),
        );
    }

    Ok(())
}

/// Merge four optional 1000×1000 tiles into one 1000×1000 tile.
///
/// This is equivalent to:
///
/// 1. stitching into a 2000×2000 canvas;
/// 2. downscaling by two.
///
/// But it avoids allocating the 4 MB canvas. Since 1000 is divisible by two,
/// each source tile maps exactly to one 500×500 output quadrant.
fn merge_four(tiles: [Option<PalettedImage>; 4]) -> Result<Option<PalettedImage>> {
    if tiles.iter().all(Option::is_none) {
        return Ok(None);
    }

    let mut output = empty_tile();

    for (quadrant, tile) in IntoIterator::into_iter(tiles).enumerate() {
        let Some(tile) = tile else {
            continue;
        };

        validate_tile(&tile)?;

        let reduced = tile.downscale_mode_weighted(downscale_weights(), 2);

        if reduced.width as usize != HALF_TILE_SIZE
            || reduced.height as usize != HALF_TILE_SIZE
            || reduced.indices.len() != HALF_TILE_SIZE * HALF_TILE_SIZE
        {
            bail!("unexpected downscaled tile dimensions");
        }

        let (dst_x, dst_y) = match quadrant {
            0 => (0, 0),                         // TL
            1 => (HALF_TILE_SIZE, 0),            // TR
            2 => (0, HALF_TILE_SIZE),            // BL
            3 => (HALF_TILE_SIZE, HALF_TILE_SIZE), // BR
            _ => unreachable!(),
        };

        for row in 0..HALF_TILE_SIZE {
            let src_offset = row * HALF_TILE_SIZE;
            let dst_offset = (dst_y + row) * TILE_SIZE + dst_x;

            output.indices[dst_offset..dst_offset + HALF_TILE_SIZE]
                .copy_from_slice(
                    &reduced.indices[src_offset..src_offset + HALF_TILE_SIZE],
                );
        }
    }

    Ok(Some(output))
}

// ─── TileHistory decoding / encoding ─────────────────────────────────────────

fn decode_tile_for_date(
    blob: Option<Vec<u8>>,
    date_hours: DateHours,
) -> Result<(Option<PalettedImage>, bool)> {
    let Some(blob) = blob else {
        return Ok((None, false));
    };

    let history =
        TileHistory::from_bytes(&blob).context("decode source TileHistory blob")?;
    
    // Indicates whether the history has a version for this date. This is used to determine if the merged image is new or not.
    let has = history.has(date_hours);

    // Missing history at this date is intentionally treated as transparent.
    let image = match history.get(date_hours) {
        Ok(image) => image,
        Err(_) => return Ok((None, false)),
    };

    validate_tile(&image)?;

    Ok((Some(image), has))
}

/// Encode a merged image into a TileHistory blob, preserving existing versions.
/// Returns `Ok(None)` if the merged image is identical to the existing version for this date.
fn encode_merged_history(
    z: i32,
    x: i32,
    y: i32,
    date_hours: DateHours,
    existing: Option<Vec<u8>>,
    image: PalettedImage,
) -> Result<Option<MergeResult>> {
    let mut history = match existing {
        Some(data) => TileHistory::from_bytes(&data)
            .context("decode existing TileHistory blob")?,
        None => TileHistory {
            imgs: Default::default(),
        },
    };

    let any_diff = history
        .set(date_hours, image)
        .context("write merged image into TileHistory")?;

    Ok(if any_diff {
        Some(MergeResult {
            z,
            x,
            y,
            data: history.to_bytes(),
        })
    } else {
        None
    })
}

// ─── Job processing ──────────────────────────────────────────────────────────

fn process_job(job: MergeJob) -> Result<Option<MergeResult>> {
    let MergeJob {
        z,
        x,
        y,
        date_hours,
        tiles,
        existing,
    } = job;

    let [tl, tr, bl, br] = tiles;
    let [(tl, has_tl), (tr, has_tr), (bl, has_bl), (br, has_br)] = [
        decode_tile_for_date(tl, date_hours)?,
        decode_tile_for_date(tr, date_hours)?,
        decode_tile_for_date(bl, date_hours)?,
        decode_tile_for_date(br, date_hours)?,
    ];

    // If none of the source tiles have a version for this date, skip the merge.
    if !has_tl && !has_tr && !has_bl && !has_br {
        return Ok(None);
    }

    let merged = merge_four([tl, tr, bl, br])?;

    let Some(merged) = merged else {
        return Ok(None);
    };

    return match encode_merged_history(z, x, y, date_hours, existing, merged) {
        Ok(Some(result)) => Ok(Some(result)),
        Ok(None) => Ok(None),
        Err(error) => Err(error).context("encode merged TileHistory"),
    }
}

fn process_job_double(job: MergeJobDouble) -> Result<Option<MergeResult>> {
    let MergeJobDouble {
        z,
        x,
        y,
        date_hours,
        tiles,
        existing,
    } = job;

    let mut decoded: [Option<PalettedImage>; 16] =
        std::array::from_fn(|_| None);
    let mut has_any = false;

    for (slot, blob) in decoded.iter_mut().zip(IntoIterator::into_iter(tiles)) {
        let has;
        (*slot, has) = decode_tile_for_date(blob, date_hours)?;
        has_any |= has;
    }

    // If none of the source tiles have a version for this date, skip the merge.
    if !has_any {
        return Ok(None);
    }

    let [
        t00, t01, t02, t03,
        t10, t11, t12, t13,
        t20, t21, t22, t23,
        t30, t31, t32, t33,
    ] = decoded;

    // Correct grouping for true row-major source layout.
    let top_left = merge_four([t00, t01, t10, t11])?;
    let top_right = merge_four([t02, t03, t12, t13])?;
    let bottom_left = merge_four([t20, t21, t30, t31])?;
    let bottom_right = merge_four([t22, t23, t32, t33])?;

    let merged = merge_four([
        top_left,
        top_right,
        bottom_left,
        bottom_right,
    ])?;

    let Some(merged) = merged else {
        return Ok(None);
    };

    return match encode_merged_history(z, x, y, date_hours, existing, merged) {
        Ok(Some(result)) => Ok(Some(result)),
        Ok(None) => Ok(None),
        Err(error) => Err(error).context("encode merged TileHistory job double"),
    }
}

// ─── Generic job construction ───────────────────────────────────────────────

trait TileMergeJob: Send + 'static + Sized {
    const SOURCE_FACTOR: i32;
    const SOURCE_Z_OFFSET: i32;

    fn from_source_rows(
        z: i32,
        x: i32,
        y: i32,
        date_hours: DateHours,
        source_x0: i32,
        source_rows: &mut [HashMap<i32, Vec<u8>>],
        existing: Option<Vec<u8>>,
    ) -> Option<Self>;
}

impl TileMergeJob for MergeJob {
    const SOURCE_FACTOR: i32 = 2;
    const SOURCE_Z_OFFSET: i32 = 1;

    fn from_source_rows(
        z: i32,
        x: i32,
        y: i32,
        date_hours: DateHours,
        source_x0: i32,
        source_rows: &mut [HashMap<i32, Vec<u8>>],
        existing: Option<Vec<u8>>,
    ) -> Option<Self> {
        debug_assert_eq!(source_rows.len(), 2);

        let tiles = [
            source_rows[0].remove(&source_x0),
            source_rows[0].remove(&(source_x0 + 1)),
            source_rows[1].remove(&source_x0),
            source_rows[1].remove(&(source_x0 + 1)),
        ];

        if tiles.iter().all(Option::is_none) {
            return None;
        }

        Some(Self {
            z,
            x,
            y,
            date_hours,
            tiles,
            existing,
        })
    }
}

impl TileMergeJob for MergeJobDouble {
    const SOURCE_FACTOR: i32 = 4;
    const SOURCE_Z_OFFSET: i32 = 2;

    fn from_source_rows(
        z: i32,
        x: i32,
        y: i32,
        date_hours: DateHours,
        source_x0: i32,
        source_rows: &mut [HashMap<i32, Vec<u8>>],
        existing: Option<Vec<u8>>,
    ) -> Option<Self> {
        debug_assert_eq!(source_rows.len(), 4);

        let tiles = std::array::from_fn(|index| {
            let row = index / 4;
            let col = index % 4;
            source_rows[row].remove(&(source_x0 + col as i32))
        });

        if tiles.iter().all(Option::is_none) {
            return None;
        }

        Some(Self {
            z,
            x,
            y,
            date_hours,
            tiles,
            existing,
        })
    }
}

// ─── SQLite helpers ──────────────────────────────────────────────────────────

fn open_db(path: &str) -> Result<Connection> {
    let conn = Connection::open(path)
        .with_context(|| format!("open database {path}"))?;

    conn.busy_timeout(Duration::from_secs(30))
        .context("set SQLite busy timeout")?;

    Ok(conn)
}

fn checked_i32(value: i64, name: &str) -> Result<i32> {
    i32::try_from(value)
        .with_context(|| format!("{name}={value} does not fit in i32"))
}

fn target_bounds(
    db_path: &str,
    source_z: i32,
    source_factor: i32,
) -> Result<Option<Bounds>> {
    let conn = open_db(db_path)?;

    conn.pragma_update(None, "journal_mode", "WAL")
        .context("enable WAL mode")?;

    let (min_x, max_x, min_y, max_y): (
        Option<i64>,
        Option<i64>,
        Option<i64>,
        Option<i64>,
    ) = conn.query_row(
        "SELECT MIN(x), MAX(x), MIN(y), MAX(y)
         FROM tiles
         WHERE z = ?1",
        params![source_z],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
            ))
        },
    )?;

    let (Some(min_x), Some(max_x), Some(min_y), Some(max_y)) =
        (min_x, max_x, min_y, max_y)
    else {
        return Ok(None);
    };

    let divisor = i64::from(source_factor);

    Ok(Some(Bounds {
        min_x: checked_i32(min_x.div_euclid(divisor), "min_x")?,
        max_x: checked_i32(max_x.div_euclid(divisor), "max_x")?,
        min_y: checked_i32(min_y.div_euclid(divisor), "min_y")?,
        max_y: checked_i32(max_y.div_euclid(divisor), "max_y")?,
    }))
}

// ─── Reader ──────────────────────────────────────────────────────────────────

fn send_with_cancel<T: Send>(
    tx: &Sender<T>,
    cancel: &AtomicBool,
    value: T,
) -> Result<bool> {
    let mut value = value;

    loop {
        if cancel.load(Ordering::Relaxed) {
            return Ok(false);
        }

        match tx.send_timeout(value, Duration::from_millis(100)) {
            Ok(()) => return Ok(true),
            Err(SendTimeoutError::Timeout(returned)) => {
                value = returned;
            }
            Err(SendTimeoutError::Disconnected(_)) => {
                return Ok(false);
            }
        }
    }
}

fn read_jobs<J: TileMergeJob>(
    db_path: &str,
    bounds: Bounds,
    z: i32,
    source_z: i32,
    date_hours: DateHours,
    job_tx: Sender<J>,
    cancel: &AtomicBool,
) -> Result<()> {
    let conn = open_db(db_path)?;

    let mut fetch_source = conn.prepare_cached(
        "SELECT x, data
         FROM tiles
         WHERE z = ?1
           AND y = ?2
           AND x >= ?3
           AND x <= ?4",
    )?;

    let mut fetch_existing = conn.prepare_cached(
        "SELECT x, data
         FROM tiles
         WHERE z = ?1
           AND y = ?2
           AND x >= ?3
           AND x <= ?4",
    )?;

    for parent_y in bounds.min_y..=bounds.max_y {
        if cancel.load(Ordering::Relaxed) {
            return Ok(());
        }

        let source_y0 = J::SOURCE_FACTOR * parent_y;

        for strip_start in
            (bounds.min_x..=bounds.max_x).step_by(STRIP_SIZE as usize)
        {
            if cancel.load(Ordering::Relaxed) {
                return Ok(());
            }

            let strip_end =
                (strip_start + STRIP_SIZE - 1).min(bounds.max_x);

            let source_x_min = J::SOURCE_FACTOR * strip_start;
            let source_x_max =
                J::SOURCE_FACTOR * strip_end + J::SOURCE_FACTOR - 1;

            let parent_count = (strip_end - strip_start + 1) as usize;
            let source_capacity =
                parent_count * J::SOURCE_FACTOR as usize;

            let mut source_rows: Vec<HashMap<i32, Vec<u8>>> =
                (0..J::SOURCE_FACTOR)
                    .map(|_| HashMap::with_capacity(source_capacity))
                    .collect();

            for (row_offset, row_map) in source_rows.iter_mut().enumerate() {
                let source_y = source_y0 + row_offset as i32;

                let mut rows = fetch_source.query(params![
                    source_z,
                    source_y,
                    source_x_min,
                    source_x_max,
                ])?;

                while let Some(row) = rows.next()? {
                    let source_x: i32 = row.get(0)?;
                    let data: Vec<u8> = row.get(1)?;
                    row_map.insert(source_x, data);
                }
            }

            let mut existing =
                HashMap::<i32, Vec<u8>>::with_capacity(parent_count);

            {
                let mut rows = fetch_existing.query(params![
                    z,
                    parent_y,
                    strip_start,
                    strip_end,
                ])?;

                while let Some(row) = rows.next()? {
                    let x: i32 = row.get(0)?;
                    let data: Vec<u8> = row.get(1)?;
                    existing.insert(x, data);
                }
            }

            for parent_x in strip_start..=strip_end {
                let source_x0 = J::SOURCE_FACTOR * parent_x;

                let Some(job) = J::from_source_rows(
                    z,
                    parent_x,
                    parent_y,
                    date_hours,
                    source_x0,
                    &mut source_rows,
                    existing.remove(&parent_x),
                ) else {
                    continue;
                };

                if !send_with_cancel(&job_tx, cancel, job)? {
                    return Ok(());
                }
            }
        }
    }

    Ok(())
}

// ─── Worker / writer pipeline ────────────────────────────────────────────────

fn merge_worker_count() -> usize {
    let automatic = thread::available_parallelism()
        .map(|count| count.get().saturating_sub(1)) // keep some headroom for reader/writer threads
        .unwrap_or(1)
        .min(DEFAULT_MAX_WORKERS)
        .max(1);

    std::env::var("WIMAGE_MERGE_WORKERS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|&count| count > 0)
        .unwrap_or(automatic)
}

fn flush_batch(
    conn: &mut Connection,
    batch: &mut Vec<MergeResult>,
) -> Result<()> {
    let tx = conn.transaction().context("start write transaction")?;

    {
        let mut stmt = tx.prepare(
            "INSERT INTO tiles (z, x, y, data)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(z, x, y)
             DO UPDATE SET data = excluded.data",
        )?;

        for result in batch.drain(..) {
            stmt.execute(params![
                result.z,
                result.x,
                result.y,
                result.data,
            ])
            .with_context(|| {
                format!(
                    "write tile z={}, x={}, y={}",
                    result.z, result.x, result.y
                )
            })?;
        }
    }

    tx.commit().context("commit tile write transaction")?;
    Ok(())
}

fn writer_loop(db_path: &str, result_rx: Receiver<MergeResult>) -> Result<()> {
    let mut conn = open_db(db_path)?;

    conn.pragma_update(None, "synchronous", "NORMAL")
        .context("set SQLite synchronous=NORMAL")?;

    let start = Instant::now();
    let mut last_report = start;
    let mut written = 0usize;

    let mut batch = Vec::with_capacity(WRITE_BATCH_SIZE);

    for result in result_rx {
        batch.push(result);
        written += 1;

        if batch.len() >= WRITE_BATCH_SIZE {
            flush_batch(&mut conn, &mut batch)?;

            if last_report.elapsed() >= Duration::from_secs(10) {
                let elapsed = start.elapsed().as_secs_f64().max(0.001);
                eprintln!(
                    "  {} tiles written in {:.1}s ({:.0} tiles/s)",
                    written,
                    elapsed,
                    written as f64 / elapsed,
                );
                last_report = Instant::now();
            }
        }
    }

    if !batch.is_empty() {
        flush_batch(&mut conn, &mut batch)?;
    }

    let elapsed = start.elapsed().as_secs_f64().max(0.001);
    eprintln!(
        "  {} tiles written in {:.1}s ({:.0} tiles/s)",
        written,
        elapsed,
        written as f64 / elapsed,
    );

    Ok(())
}

fn join_thread(
    thread: JoinHandle<Result<()>>,
    name: &str,
) -> Result<()> {
    match thread.join() {
        Ok(result) => result,
        Err(_) => Err(anyhow!("{name} thread panicked")),
    }
}

fn run_pipeline<J, Reader>(
    db_path: &str,
    reader_fn: Reader,
    process: fn(J) -> Result<Option<MergeResult>>,
) -> Result<()>
where
    J: Send + 'static,
    Reader: FnOnce(Sender<J>, Arc<AtomicBool>) -> Result<()> + Send + 'static,
{
    let worker_count = merge_worker_count();
    let channel_capacity = (worker_count * 2).max(4);

    let (job_tx, job_rx) = bounded::<J>(channel_capacity);
    let (result_tx, result_rx) =
        bounded::<MergeResult>(channel_capacity);

    let cancelled = Arc::new(AtomicBool::new(false));

    let writer_path = db_path.to_owned();
    let writer_cancel = Arc::clone(&cancelled);
    let writer = thread::spawn(move || {
        let result = writer_loop(&writer_path, result_rx);

        if result.is_err() {
            writer_cancel.store(true, Ordering::Relaxed);
        }

        result
    });

    let processor = Arc::new(process);
    let mut workers = Vec::with_capacity(worker_count);

    for _ in 0..worker_count {
        let job_rx = job_rx.clone();
        let result_tx = result_tx.clone();
        let processor = Arc::clone(&processor);
        let cancel = Arc::clone(&cancelled);

        workers.push(thread::spawn(move || -> Result<()> {
            for job in job_rx {
                if cancel.load(Ordering::Relaxed) {
                    break;
                }

                let result = match processor(job) {
                    Ok(result) => result,
                    Err(error) => {
                        cancel.store(true, Ordering::Relaxed);
                        return Err(error);
                    }
                };

                if let Some(result) = result {
                    if result_tx.send(result).is_err() {
                        cancel.store(true, Ordering::Relaxed);
                        bail!("writer thread stopped receiving results");
                    }
                }
            }

            Ok(())
        }));
    }

    // Critical: without this, workers never observe a disconnected job channel.
    drop(job_rx);

    // Only worker clones should keep the result channel alive.
    drop(result_tx);

    let reader_cancel = Arc::clone(&cancelled);
    let reader = thread::spawn(move || {
        let result = reader_fn(job_tx, Arc::clone(&reader_cancel));

        if result.is_err() {
            reader_cancel.store(true, Ordering::Relaxed);
        }

        result
    });

    let reader_result = join_thread(reader, "reader");
    let worker_results: Vec<Result<()>> = workers
        .into_iter()
        .map(|worker| join_thread(worker, "worker"))
        .collect();
    let writer_result = join_thread(writer, "writer");

    reader_result?;
    writer_result?;

    for result in worker_results {
        result?;
    }

    Ok(())
}

// ─── Merge levels ────────────────────────────────────────────────────────────

fn merge_with<J: TileMergeJob>(
    db_path: &str,
    z: i32,
    date_hours: DateHours,
    process: fn(J) -> Result<Option<MergeResult>>,
) -> Result<()> {
    let source_z = z + J::SOURCE_Z_OFFSET;

    let Some(bounds) =
        target_bounds(db_path, source_z, J::SOURCE_FACTOR)?
    else {
        return Ok(());
    };

    let reader_path = db_path.to_owned();

    run_pipeline(
        db_path,
        move |job_tx, cancelled| {
            read_jobs::<J>(
                &reader_path,
                bounds,
                z,
                source_z,
                date_hours,
                job_tx,
                &cancelled,
            )
        },
        process,
    )
}

fn merge_level(
    db_path: &str,
    z: i32,
    date_hours: DateHours,
) -> Result<()> {
    merge_with::<MergeJob>(db_path, z, date_hours, process_job)
}

fn merge_level_double(
    db_path: &str,
    z: i32,
    date_hours: DateHours,
) -> Result<()> {
    merge_with::<MergeJobDouble>(
        db_path,
        z,
        date_hours,
        process_job_double,
    )
}

// ─── Public API ──────────────────────────────────────────────────────────────

pub fn merge(db_path: &str, date_hours: DateHours) -> Result<()> {
    eprintln!("Merging level z=9 (double from z=11) ...");
    merge_level_double(db_path, 9, date_hours)?;

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

    // ─── Helpers ─────────────────────────────────────────────────────────────

    fn make_tile(value: u8) -> PalettedImage {
        PalettedImage {
            width: TILE_SIZE,
            height: TILE_SIZE,
            indices: vec![value; TILE_PIXELS],
        }
    }

    fn history_with(img: PalettedImage, date: DateHours) -> Vec<u8> {
        let mut th = TileHistory {
            imgs: Default::default(),
        };
        th.set(date, img).unwrap();
        th.to_bytes()
    }

    fn create_test_db(path: &str) -> Result<()> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "CREATE TABLE tiles (
                z INTEGER NOT NULL,
                x INTEGER NOT NULL,
                y INTEGER NOT NULL,
                data BLOB NOT NULL,
                PRIMARY KEY (z, x, y)
            );
            CREATE INDEX tiles_z_y_x_idx ON tiles (z, y, x);",
        )?;
        Ok(())
    }

    fn read_tile(conn: &Connection, z: i32, x: i32, y: i32) -> Option<Vec<u8>> {
        conn.query_row(
            "SELECT data FROM tiles WHERE z = ?1 AND x = ?2 AND y = ?3",
            params![z, x, y],
            |r| r.get(0),
        )
        .ok()
    }

    /// Assert an entire rectangular region of `img` has `expected`.
    fn assert_region(img: &PalettedImage, x0: usize, y0: usize, size: usize, expected: u8) {
        for y in y0..y0 + size {
            for x in x0..x0 + size {
                assert_eq!(
                    img.indices[y * TILE_SIZE + x],
                    expected,
                    "pixel ({x},{y})",
                );
            }
        }
    }

    // ─── merge_four ──────────────────────────────────────────────────────────

    #[test]
    fn merge_four_places_quadrants_correctly() {
        let merged = merge_four([
            Some(make_tile(1)), // TL
            Some(make_tile(2)), // TR
            Some(make_tile(3)), // BL
            Some(make_tile(4)), // BR
        ])
        .unwrap()
        .unwrap();

        assert_eq!(merged.width as usize, TILE_SIZE);
        assert!(!merged.indices.windows(2).all(|w| w[0] == w[1]));

        let h = HALF_TILE_SIZE;
        assert_region(&merged, 0, 0, h, 1);
        assert_region(&merged, h, 0, h, 2);
        assert_region(&merged, 0, h, h, 3);
        assert_region(&merged, h, h, h, 4);
    }

    #[test]
    fn merge_four_missing_quadrant_stays_transparent() {
        let merged = merge_four([
            Some(make_tile(9)),
            None, // missing TR
            Some(make_tile(9)),
            Some(make_tile(9)),
        ])
        .unwrap()
        .unwrap();

        let h = HALF_TILE_SIZE;
        assert_region(&merged, h, 0, h, palette::TRANSPARENT);
        assert_region(&merged, 0, 0, h, 9);
    }

    #[test]
    fn merge_four_all_missing_returns_none() {
        assert!(merge_four([None, None, None, None]).unwrap().is_none());
    }

    // ─── process_job ─────────────────────────────────────────────────────────

    #[test]
    fn process_job_all_missing_returns_none() {
        let job = MergeJob {
            z: 10,
            x: 0,
            y: 0,
            date_hours: DateHours(1),
            tiles: [None, None, None, None],
            existing: None,
        };
        assert!(process_job(job).unwrap().is_none());
    }

    #[test]
    fn process_job_merges_and_preserves_existing_versions() {
        let src = history_with(make_tile(42), DateHours(1));
        let existing = history_with(make_tile(10), DateHours(0));

        let result = process_job(MergeJob {
            z: 10,
            x: 0,
            y: 0,
            date_hours: DateHours(1),
            tiles: [Some(src.clone()), Some(src.clone()), Some(src), None],
            existing: Some(existing),
        })
        .unwrap()
        .unwrap();

        let th = TileHistory::from_bytes(&result.data).unwrap();
        assert_eq!(th.list().len(), 2, "both versions must survive");

        assert!(th.get(DateHours(0)).unwrap().indices.iter().all(|&v| v == 10));

        let merged = th.get(DateHours(1)).unwrap();
        assert_region(&merged, 0, 0, HALF_TILE_SIZE, 42);
        assert_region(
            &merged,
            HALF_TILE_SIZE,
            HALF_TILE_SIZE,
            HALF_TILE_SIZE,
            palette::TRANSPARENT,
        );
    }

    // ─── Double-merge layout regression ───────────────────────────────────────
    //
    // The reader now emits tiles in true 4×4 row-major order. Before the fix,
    // process_job_double assumed a quadrant-grouped layout, which scrambled
    // the output image. This test pins the correct behaviour.

    #[test]
    fn double_merge_preserves_4x4_row_major_layout() {
        let date = DateHours(1);
        let tiles = std::array::from_fn(|i| {
            Some(history_with(make_tile((i + 1) as u8), date))
        });

        let result = process_job_double(MergeJobDouble {
            z: 9,
            x: 0,
            y: 0,
            date_hours: date,
            tiles,
            existing: None,
        })
        .unwrap()
        .unwrap();

        let th = TileHistory::from_bytes(&result.data).unwrap();
        let img = th.get(date).unwrap();

        // After two downscales, each 1000² source tile → 250×250 block.
        for row in 0..4usize {
            for col in 0..4usize {
                let x = col * 250 + 125;
                let y = row * 250 + 125;
                assert_eq!(
                    img.indices[y * TILE_SIZE + x],
                    (row * 4 + col + 1) as u8,
                    "block (row={row}, col={col})"
                );
            }
        }
    }

    #[test]
    fn double_merge_single_source_tile_lands_top_left() {
        let date = DateHours(1);
        let mut tiles: [Option<Vec<u8>>; 16] = std::array::from_fn(|_| None);
        tiles[0] = Some(history_with(make_tile(5), date));

        let result = process_job_double(MergeJobDouble {
            z: 9,
            x: 0,
            y: 0,
            date_hours: date,
            tiles,
            existing: None,
        })
        .unwrap()
        .unwrap();

        let img = TileHistory::from_bytes(&result.data)
            .unwrap()
            .get(date)
            .unwrap();

        assert_region(&img, 0, 0, 250, 5);
        assert_region(&img, 250, 0, 250, palette::TRANSPARENT);
        assert_region(&img, 0, 250, 250, palette::TRANSPARENT);
    }

    // ─── End-to-end through SQLite ───────────────────────────────────────────

    #[test]
    fn merge_level_round_trips_and_upserts() -> Result<()> {
        let tmp = tempfile::NamedTempFile::new()?;
        let path = tmp.path().to_str().unwrap();
        create_test_db(path)?;

        // Source tiles carry two versions: colour 7 at date 1, colour 8 at date 2.
        let mut th = TileHistory {
            imgs: Default::default(),
        };
        th.set(DateHours(1), make_tile(7))?;
        th.set(DateHours(2), make_tile(8))?;
        let blob = th.to_bytes();

        let conn = Connection::open(path)?;
        for x in 0..=1 {
            for y in 0..=1 {
                conn.execute(
                    "INSERT INTO tiles (z, x, y, data) VALUES (11, ?1, ?2, ?3)",
                    params![x, y, blob],
                )?;
            }
        }
        drop(conn);

        // First pass: creates the z=10 tile.
        merge_level(path, 10, DateHours(1))?;
        // Second pass: must UPDATE the row, not duplicate or clobber history.
        merge_level(path, 10, DateHours(2))?;

        let conn = Connection::open(path)?;
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM tiles WHERE z = 10", [], |r| r.get(0))?;
        assert_eq!(count, 1, "upsert must not create duplicate rows");

        let data = read_tile(&conn, 10, 0, 0).expect("z=10 tile must exist");
        let th = TileHistory::from_bytes(&data).unwrap();
        assert_eq!(th.list().len(), 2);
        assert!(th.get(DateHours(1)).unwrap().indices.iter().all(|&v| v == 7));
        assert!(th.get(DateHours(2)).unwrap().indices.iter().all(|&v| v == 8));

        Ok(())
    }

    #[test]
    fn merge_level_empty_source_is_noop() -> Result<()> {
        let tmp = tempfile::NamedTempFile::new()?;
        let path = tmp.path().to_str().unwrap();
        create_test_db(path)?;

        merge_level(path, 10, DateHours(1))?;
        merge_level_double(path, 9, DateHours(1))?;

        let conn = Connection::open(path)?;
        let count: i64 =
            conn.query_row("SELECT COUNT(*) FROM tiles", [], |r| r.get(0))?;
        assert_eq!(count, 0);
        Ok(())
    }
}