/// Implements the `increment --archives ARCHIVES_FOLDER --increment INCREMENT_DB` command.
/// Convert the images from INCREMENT_DB and add them to the latest archive in ARCHIVES_FOLDER.
/// 
/// The `wimage` crate already implements everything needed to convert/manipulate the images (`PalettedImage`, `TileHistory`, `DateHours`, ...).
/// 
/// ## Input
/// ### ARCHIVES_FOLDER
/// Folder containing SQLite .db file like so:
/// - w71_12095.db
/// - w72_12262.db
/// - w73_12335.db
/// Named `w<week>_<datehours>.db`, With `week` the weeks number since 2025, and `datehours` the hours number since 2025-01-01T00:00:00Z.
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
/// The data blob is a `TileHistory` from `wimage`.
/// Images are tiled in x,y,z pyramid for displaying on a map. Where z represent the zoom level.
/// - 0 <= z <= 11
/// - 0 <= x <= 2048 (2^11)
/// - 0 <= y <= 2048 (2^11)
/// 
/// ### INCREMENT_DB
/// A SQLite db containing png images of tiles that need to be updated. The z level implicitely 11.
/// The filename indicate the time of the increment: `inc_2026-06-03T22-11-00Z.db`.
/// 
/// The sqlite db has this table:
/// ```sql
/// CREATE TABLE tiles (
///     -- z is 11
///     x INTEGER,
///     y INTEGER,
///     data BLOB,
///     PRIMARY KEY (x, y)
/// );
/// ```
/// The data blob is a PNG image.
/// 
/// ## Output
/// Update the latest archive in ARCHIVES_FOLDER, or, if the week changed, create a new one.
/// 
/// ## Processing
/// For every image in INCREMENT_DB, add it to its corresponding TileHistory of the latest archive in ARCHIVES_FOLDER. If INCREMENT_DB is from a new (different) week than the latest archive, a new archive is created.
/// 
/// ### New archive creation
/// Special step to prepare a new archive when INCREMENT_DB week number `W` is different than the latest week number `W-1`.
/// 
/// Create a new DB `wW_DATEHOURS.db` withe empty tables `tiles` and `versions`. DATEHOURS is calculated from the name of the INCREMENT_DB. 
/// 
/// For ALL existing tiles (any x,y,z) in the latest archive:
/// - Read the TileHistory blob.
/// - Get the latest image from that TileHistory (`TileHistory.from_byte(blob)` then `TileHistory.get(DateHours.max())`).
/// - Create a new TileHistory, and set that image as DateHours 0 (`TileHistory.set`).
/// - Write the blob (`TileHistory.to_bytes`) of that new TileHistory to the new DB.
/// 
/// This will setup the base, full images, of that new archive (all addition we then do to the TileHistory will be increment diff).
/// 
/// ### Usual increment step
/// For every image in INCREMENT_DB (usually ~60_000 PNGs, ~4GB ), add it to its corresponding TileHistory, at the datehours calculated from the timestamp of the INCREMENT_DB filename.
/// 
/// Add the datehours and filename the the `versions` table.
/// 
/// For every PNG:
/// - Read PNG blob and convert it to Paletted (`PalettedImage.from_png`)
/// - Read TileHistory blob, deserialize and set the PalletedImage at the datehours.
/// - Serialize the TileHistory and write to the latest archive.
/// 
/// If necessary, rename the latest archive to update the datehours in its name.

use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use chrono::DateTime;
use crossbeam_channel::{bounded, Receiver, SendTimeoutError, Sender};
use rusqlite::{params, Connection};

use wimage::tilehistory::{DateHours, TileHistory};
use wimage::PalettedImage;

use crate::common::{open_db, enable_wal, create_empty_archive};

const Z_TARGET: i32 = 11;
const WRITE_BATCH_SIZE: usize = 256;
const DEFAULT_MAX_WORKERS: usize = 16;

// ─── Types ───────────────────────────────────────────────────────────────────

/// A tile from INCREMENT_DB paired with its existing TileHistory (if any) from the target archive.
pub(crate) struct IncrementJob {
    x: i32,
    y: i32,
    date_hours: DateHours,
    png: Vec<u8>,
    existing: Option<Vec<u8>>,
}

pub(crate) struct IncrementResult {
    x: i32,
    y: i32,
    data: Vec<u8>,
}

/// A tile from the source archive whose latest image must be seeded into the new archive.
pub(crate) struct SeedJob {
    z: i32,
    x: i32,
    y: i32,
    data: Vec<u8>,
}

pub(crate) struct SeedResult {
    z: i32,
    x: i32,
    y: i32,
    data: Vec<u8>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct ArchiveName {
    week: u32,
    date_hours: u32,
}

// ─── Filename parsing ────────────────────────────────────────────────────────

/// Parse an increment filename like `inc_2026-06-03T22-11-00Z.db` into the UTC datetime.
pub fn parse_increment_datetime(filename: &str) -> Result<DateTime<chrono::Utc>> {
    let stem = Path::new(filename)
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| anyhow!("increment filename has no stem: {filename}"))?;

    let rest = stem
        .strip_prefix("inc_")
        .ok_or_else(|| anyhow!("increment filename must start with 'inc_': {filename}"))?;

    // `2026-06-03T22-11-00Z` → `2026-06-03T22:11:00Z`
    // Only the time portion (after 'T') uses ':' separators; the date keeps '-'.
    let normalized = match rest.find('T') {
        Some(pos) => {
            let (date, time) = rest.split_at(pos);
            format!("{date}{}", time.replace('-', ":"))
        }
        None => rest.to_owned(),
    };

    let dt = DateTime::parse_from_rfc3339(&normalized)
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .with_context(|| format!("parse increment timestamp from {filename}"))?;

    Ok(dt)
}

/// Parse an archive filename like `w71_12095.db` into (week, date_hours).
pub(crate) fn parse_archive_filename(filename: &str) -> Result<ArchiveName> {
    let stem = Path::new(filename)
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| anyhow!("archive filename has no stem: {filename}"))?;

    let rest = stem
        .strip_prefix('w')
        .ok_or_else(|| anyhow!("archive filename must start with 'w': {filename}"))?;

    let mut parts = rest.split('_');
    let week: u32 = parts
        .next()
        .ok_or_else(|| anyhow!("archive filename missing week: {filename}"))?
        .parse()
        .with_context(|| format!("parse archive week from {filename}"))?;

    let date_hours: u32 = parts
        .next()
        .ok_or_else(|| anyhow!("archive filename missing datehours: {filename}"))?
        .parse()
        .with_context(|| format!("parse archive datehours from {filename}"))?;

    Ok(ArchiveName { week, date_hours })
}

fn archive_filename(week: u32, date_hours: u32) -> String {
    format!("w{week}_{date_hours}.db")
}

pub fn increment_date_hours(increment_db: &str) -> Result<DateHours> {
    let filename = Path::new(increment_db)
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| anyhow!("increment path has no filename: {increment_db}"))?;

    Ok(DateHours::from_datetime(parse_increment_datetime(filename)?))
}

/// Find the latest archive (largest week, tie-broken by datehours) in a folder.
pub fn find_latest_archive(archives_folder: &str) -> Result<Option<PathBuf>> {
    let mut latest: Option<(ArchiveName, PathBuf)> = None;

    for entry in fs::read_dir(archives_folder)
        .with_context(|| format!("read archives folder {archives_folder}"))?
    {
        let entry = entry?;
        let path = entry.path();

        let Some(filename) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };

        if !filename.starts_with('w') || !filename.ends_with(".db") {
            continue;
        }

        let name = match parse_archive_filename(filename) {
            Ok(n) => n,
            Err(_) => continue,
        };

        if latest.as_ref().map_or(true, |(current, _)| {
            name.week > current.week
                || (name.week == current.week && name.date_hours > current.date_hours)
        }) {
            latest = Some((name, path));
        }
    }

    Ok(latest.map(|(_, path)| path))
}

// ─── Processors ──────────────────────────────────────────────────────────────

/// Decode a PNG blob into a PalettedImage.
fn decode_png(png: &[u8]) -> Result<PalettedImage> {
    PalettedImage::from_png(Cursor::new(png)).context("decode PNG tile")
}

/// Set a PNG tile into its TileHistory at date_hours, preserving previous versions.
/// Returns `None` when the image is identical to the previous version.
pub(crate) fn process_increment(job: IncrementJob) -> Result<Option<IncrementResult>> {
    let IncrementJob { x, y, date_hours, png, existing } = job;

    let image = decode_png(&png)?;

    let mut history = match existing {
        Some(data) => TileHistory::from_bytes(&data)
            .context("decode existing TileHistory blob")?,
        None => TileHistory { imgs: Default::default() },
    };

    let any_diff = history
        .set(date_hours, image)
        .context("set PNG into TileHistory")?;

    Ok(if any_diff {
        Some(IncrementResult {
            x,
            y,
            data: history.to_bytes(),
        })
    } else {
        None
    })
}

/// Seed a new archive: take the latest image of a source tile and store it as the
/// base (full image) at DateHours 0.
pub(crate) fn process_seed(job: SeedJob) -> Result<Option<SeedResult>> {
    let SeedJob { z, x, y, data } = job;

    let source = TileHistory::from_bytes(&data).context("decode source TileHistory blob")?;

    // The latest image is the one at the max datehours present in the history.
    let image = source.get(DateHours::max()).with_context(|| {
        format!("get latest image from source tile z={z}, x={x}, y={y}")
    })?;
    if image.is_none() {
        // No image in the source history, skip this tile.
        return Ok(None);
    }
    let image = image.unwrap();

    let mut seeded = TileHistory { imgs: Default::default() };
    seeded.set(DateHours(0), image).with_context(|| {
        format!("seed base image for tile z={z}, x={x}, y={y}")
    })?;

    Ok(Some(SeedResult {
        z,
        x,
        y,
        data: seeded.to_bytes(),
    }))
}

// ─── Readers ─────────────────────────────────────────────────────────────────

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

/// Stream PNG tiles from INCREMENT_DB, paired with the existing TileHistory blob
/// at (z=11, x, y) from the target archive.
fn read_increment_jobs(
    increment_db: &str,
    target_db: &str,
    date_hours: DateHours,
    job_tx: Sender<IncrementJob>,
    total_tx: Sender<u64>,
    cancel: &AtomicBool,
) -> Result<()> {
    let inc_conn = open_db(increment_db)?;
    let target_conn = open_db(target_db)?;

    let mut fetch_png = inc_conn.prepare("SELECT x, y, data FROM tiles ORDER BY x, y")?;

    let mut fetch_existing = target_conn.prepare_cached(
        "SELECT data FROM tiles WHERE z = ?1 AND x = ?2 AND y = ?3",
    )?;

    let mut rows = fetch_png.query([])?;
    let mut count = 0u64;

    while let Some(row) = rows.next()? {
        if cancel.load(Ordering::Relaxed) {
            return Ok(());
        }

        let x: i32 = row.get(0)?;
        let y: i32 = row.get(1)?;
        let png: Vec<u8> = row.get(2)?;

        let existing: Option<Vec<u8>> = match fetch_existing.query_row(
            params![Z_TARGET, x, y],
            |row| row.get::<_, Vec<u8>>(0),
        ) {
            Ok(data) => Some(data),
            Err(rusqlite::Error::QueryReturnedNoRows) => None,
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("read existing tile z={Z_TARGET}, x={x}, y={y}")
                })
            }
        };

        let job = IncrementJob { x, y, date_hours, png, existing };

        if !send_with_cancel(&job_tx, cancel, job)? {
            return Ok(());
        }

        count += 1;
        if count % 1024 == 0 {
            if total_tx.send(count).is_err() {
                return Ok(());
            }
        }
    }

    let _ = total_tx.send(count);
    Ok(())
}

/// Stream every tile from the source archive for seeding a new archive.
fn read_seed_jobs(
    source_db: &str,
    job_tx: Sender<SeedJob>,
    total_tx: Sender<u64>,
    cancel: &AtomicBool,
) -> Result<()> {
    let conn = open_db(source_db)?;

    let mut fetch = conn.prepare("SELECT z, x, y, data FROM tiles ORDER BY z, x, y")?;

    let mut rows = fetch.query([])?;
    let mut count = 0u64;

    while let Some(row) = rows.next()? {
        if cancel.load(Ordering::Relaxed) {
            return Ok(());
        }

        let z: i32 = row.get(0)?;
        let x: i32 = row.get(1)?;
        let y: i32 = row.get(2)?;
        let data: Vec<u8> = row.get(3)?;

        let job = SeedJob { z, x, y, data };

        if !send_with_cancel(&job_tx, cancel, job)? {
            return Ok(());
        }

        count += 1;
        if count % 1024 == 0 {
            if total_tx.send(count).is_err() {
                return Ok(());
            }
        }
    }

    let _ = total_tx.send(count);
    Ok(())
}

// ─── Writers ──────────────────────────────────────────────────────────────────

fn flush_increment_batch(
    conn: &mut Connection,
    batch: &mut Vec<IncrementResult>,
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
            stmt.execute(params![Z_TARGET, result.x, result.y, result.data])
                .with_context(|| {
                    format!("write tile z={Z_TARGET}, x={}, y={}", result.x, result.y)
                })?;
        }
    }

    tx.commit().context("commit tile write transaction")?;
    Ok(())
}

fn flush_seed_batch(
    conn: &mut Connection,
    batch: &mut Vec<SeedResult>,
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
            stmt.execute(params![result.z, result.x, result.y, result.data])
                .with_context(|| {
                    format!("write tile z={}, x={}, y={}", result.z, result.x, result.y)
                })?;
        }
    }

    tx.commit().context("commit tile write transaction")?;
    Ok(())
}

fn increment_writer_loop(
    db_path: &str,
    result_rx: Receiver<IncrementResult>,
    total_rx: Receiver<u64>,
) -> Result<()> {
    let mut conn = open_db(db_path)?;
    conn.pragma_update(None, "synchronous", "NORMAL")
        .context("set SQLite synchronous=NORMAL")?;

    let start = Instant::now();
    let mut last_report = start;
    let mut total_read = 0u64;
    let mut written = 0usize;

    let mut batch = Vec::with_capacity(WRITE_BATCH_SIZE);

    loop {
        while let Ok(count) = total_rx.try_recv() {
            total_read = count;
        }

        match result_rx.recv_timeout(Duration::from_secs(1)) {
            Ok(result) => {
                batch.push(result);
                written += 1;

                if batch.len() >= WRITE_BATCH_SIZE {
                    flush_increment_batch(&mut conn, &mut batch)?;
                }
            }
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                while let Ok(count) = total_rx.try_recv() {
                    total_read = count;
                }
                break;
            }
        }

        if last_report.elapsed() >= Duration::from_secs(10) {
            let elapsed = start.elapsed().as_secs_f64().max(0.001);
            eprintln!(
                "  {total_read} tiles read, {written} written ({:.0} tiles/s)",
                written as f64 / elapsed,
            );
            last_report = Instant::now();
        }
    }

    if !batch.is_empty() {
        flush_increment_batch(&mut conn, &mut batch)?;
    }

    let elapsed = start.elapsed().as_secs_f64().max(0.001);
    eprintln!(
        "  Done: {total_read} tiles read, {written} written ({:.0} tiles/s)",
        written as f64 / elapsed,
    );

    Ok(())
}

fn seed_writer_loop(
    db_path: &str,
    result_rx: Receiver<SeedResult>,
    total_rx: Receiver<u64>,
) -> Result<()> {
    let mut conn = open_db(db_path)?;
    conn.pragma_update(None, "synchronous", "NORMAL")
        .context("set SQLite synchronous=NORMAL")?;

    let start = Instant::now();
    let mut last_report = start;
    let mut total_read = 0u64;
    let mut written = 0usize;

    let mut batch = Vec::with_capacity(WRITE_BATCH_SIZE);

    loop {
        while let Ok(count) = total_rx.try_recv() {
            total_read = count;
        }

        match result_rx.recv_timeout(Duration::from_secs(1)) {
            Ok(result) => {
                batch.push(result);
                written += 1;

                if batch.len() >= WRITE_BATCH_SIZE {
                    flush_seed_batch(&mut conn, &mut batch)?;
                }
            }
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                while let Ok(count) = total_rx.try_recv() {
                    total_read = count;
                }
                break;
            }
        }

        if last_report.elapsed() >= Duration::from_secs(10) {
            let elapsed = start.elapsed().as_secs_f64().max(0.001);
            eprintln!(
                "  {total_read} tiles read, {written} seeded ({:.0} tiles/s)",
                written as f64 / elapsed,
            );
            last_report = Instant::now();
        }
    }

    if !batch.is_empty() {
        flush_seed_batch(&mut conn, &mut batch)?;
    }

    let elapsed = start.elapsed().as_secs_f64().max(0.001);
    eprintln!(
        "  Done: {total_read} tiles read, {written} seeded ({:.0} tiles/s)",
        written as f64 / elapsed,
    );

    Ok(())
}

// ─── Thread helpers ──────────────────────────────────────────────────────────

fn join_thread(thread: JoinHandle<Result<()>>, name: &str) -> Result<()> {
    match thread.join() {
        Ok(result) => result,
        Err(_) => Err(anyhow!("{name} thread panicked")),
    }
}

fn increment_worker_count() -> usize {
    let automatic = thread::available_parallelism()
        .map(|count| count.get().saturating_sub(1))
        .unwrap_or(1)
        .min(DEFAULT_MAX_WORKERS)
        .max(1);

    std::env::var("WIMAGE_INCREMENT_WORKERS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|&count| count > 0)
        .unwrap_or(automatic)
}

// ─── Pipeline runner ─────────────────────────────────────────────────────────

fn run_pipeline<J, Res, Reader, Writer>(
    target_db_path: &str,
    reader_fn: Reader,
    process: fn(J) -> Result<Option<Res>>,
    writer_loop: Writer,
) -> Result<()>
where
    J: Send + 'static,
    Res: Send + 'static,
    Reader: FnOnce(Sender<J>, Sender<u64>, Arc<AtomicBool>) -> Result<()> + Send + 'static,
    Writer: FnOnce(&str, Receiver<Res>, Receiver<u64>) -> Result<()> + Send + 'static,
{
    let worker_count = increment_worker_count();
    let channel_capacity = (worker_count * 2).max(4);

    let (job_tx, job_rx) = bounded::<J>(channel_capacity);
    let (result_tx, result_rx) = bounded::<Res>(channel_capacity);
    let (total_tx, total_rx) = bounded::<u64>(channel_capacity);

    let cancelled = Arc::new(AtomicBool::new(false));

    let writer_path = target_db_path.to_owned();
    let writer_cancel = Arc::clone(&cancelled);
    let writer = thread::spawn(move || {
        let result = writer_loop(&writer_path, result_rx, total_rx);
        if result.is_err() {
            writer_cancel.store(true, Ordering::Relaxed);
        }
        result
    });

    let processor = process;
    let mut workers = Vec::with_capacity(worker_count);

    for _ in 0..worker_count {
        let job_rx = job_rx.clone();
        let result_tx = result_tx.clone();
        let processor = processor;
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

    drop(job_rx);
    drop(result_tx);

    let reader_cancel = Arc::clone(&cancelled);
    let reader = thread::spawn(move || {
        let result = reader_fn(job_tx, total_tx, reader_cancel);
        if result.is_err() {
            cancelled.store(true, Ordering::Relaxed);
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

// ─── Steps ───────────────────────────────────────────────────────────────────

/// Seed a new archive from the latest archive's current images.
fn seed_new_archive(latest_db: &str, new_db: &str) -> Result<()> {
    eprintln!("Creating new archive {new_db} seeded from {latest_db} ...");
    create_empty_archive(new_db)?;
    enable_wal(new_db)?;

    let latest_path = latest_db.to_owned();
    let new_path = new_db.to_owned();

    run_pipeline(
        &new_path,
        move |job_tx, total_tx, cancel| {
            read_seed_jobs(&latest_path, job_tx, total_tx, &cancel)
        },
        process_seed,
        seed_writer_loop,
    )
}

/// Add every PNG tile from INCREMENT_DB into the target archive.
fn increment_archive(
    increment_db: &str,
    target_db: &str,
    date_hours: DateHours,
) -> Result<()> {
    eprintln!(
        "Adding tiles from {increment_db} to {target_db} at datehours={} ({}) ...",
        date_hours.0,
        date_hours.to_datetime(),
    );

    enable_wal(target_db)?;

    let increment_path = increment_db.to_owned();
    let target_path = target_db.to_owned();
    let reader_target = target_path.clone();

    run_pipeline(
        &target_path,
        move |job_tx, total_tx, cancel| {
            read_increment_jobs(
                &increment_path,
                &reader_target,
                date_hours,
                job_tx,
                total_tx,
                &cancel,
            )
        },
        process_increment,
        increment_writer_loop,
    )
}

pub fn add_version(archive_db: &str, date_hours: DateHours, original_file: &str) -> Result<()> {
    let conn = open_db(archive_db)?;
    conn.execute(
        "INSERT INTO versions (date, original_file)
         VALUES (?1, ?2)
         ON CONFLICT(date) DO UPDATE SET original_file = excluded.original_file",
        params![date_hours.0 as i64, original_file],
    )
    .with_context(|| format!("insert version date={}", date_hours.0))?;
    Ok(())
}

/// Rename the latest archive file to reflect an updated datehours in its name,
/// if the new datehours is later than the one currently embedded in the filename.
pub(crate) fn rename_archive_if_needed(
    archive_path: &Path,
    old_name: ArchiveName,
    new_date_hours: u32,
) -> Result<PathBuf> {
    if new_date_hours <= old_name.date_hours {
        return Ok(archive_path.to_path_buf());
    }

    let new_filename = archive_filename(old_name.week, new_date_hours);
    let parent = archive_path
        .parent()
        .ok_or_else(|| anyhow!("archive path has no parent: {}", archive_path.display()))?;

    let new_path = parent.join(&new_filename);

    fs::rename(archive_path, &new_path)
        .with_context(|| format!("rename archive to {new_filename}"))?;

    eprintln!("Renamed archive to {new_filename}");
    Ok(new_path)
}

// ─── Public API ──────────────────────────────────────────────────────────────

pub fn increment(archives_folder: &str, increment_db: &str) -> Result<(PathBuf, DateHours)> {
    let date_hours = increment_date_hours(increment_db)?;
    let inc_week = date_hours.week();

    let increment_filename = Path::new(increment_db)
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| anyhow!("increment path has no filename: {increment_db}"))?;

    let Some(latest_path) = find_latest_archive(archives_folder)? else {
        bail!("no archive found in {archives_folder}; an existing archive is required");
    };

    let latest_filename = latest_path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| anyhow!("latest archive path has no filename"))?;

    let latest_name = parse_archive_filename(latest_filename)?;

    let target_path = if inc_week != latest_name.week {
        // New week: seed a new archive from the latest one.
        let new_filename = archive_filename(inc_week, date_hours.0);
        let new_path = Path::new(archives_folder).join(&new_filename);
        let new_path_str = new_path
            .to_str()
            .ok_or_else(|| anyhow!("new archive path is not valid UTF-8"))?;

        seed_new_archive(
            latest_path
                .to_str()
                .ok_or_else(|| anyhow!("latest archive path is not valid UTF-8"))?,
            new_path_str,
        )?;

        new_path
    } else {
        latest_path.clone()
    };

    let target_db = target_path
        .to_str()
        .ok_or_else(|| anyhow!("target archive path is not valid UTF-8"))?;

    increment_archive(increment_db, target_db, date_hours)?;
    add_version(target_db, date_hours, increment_filename)?;

    let archive_path = if inc_week == latest_name.week {
        // The latest archive was reused; its filename may need a datehours bump.
        rename_archive_if_needed(&latest_path, latest_name, date_hours.0)?
    } else {
        // A new archive was created for the new week; report that one.
        target_path.clone()
    };

    Ok((archive_path, date_hours))
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use wimage::palette;

    const TILE_SIZE: usize = 1000;
    const TILE_PIXELS: usize = TILE_SIZE * TILE_SIZE;

    // ─── Helpers ─────────────────────────────────────────────────────────────

    fn make_tile(value: u8) -> PalettedImage {
        PalettedImage {
            width: TILE_SIZE,
            height: TILE_SIZE,
            indices: vec![value; TILE_PIXELS],
        }
    }

    fn history_with(img: PalettedImage, date: DateHours) -> Vec<u8> {
        let mut th = TileHistory { imgs: Default::default() };
        th.set(date, img).unwrap();
        th.to_bytes()
    }

    fn png_for(value: u8) -> Vec<u8> {
        make_tile(value).to_png().unwrap()
    }

    fn create_archive_db(path: &str) -> Result<()> {
        create_empty_archive(path)
    }

    fn create_increment_db(path: &str, tiles: &[(i32, i32, Vec<u8>)]) -> Result<()> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "CREATE TABLE tiles (
                x INTEGER,
                y INTEGER,
                data BLOB,
                PRIMARY KEY (x, y)
            );",
        )?;
        for (x, y, data) in tiles {
            conn.execute(
                "INSERT INTO tiles (x, y, data) VALUES (?1, ?2, ?3)",
                params![x, y, data],
            )?;
        }
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

    fn make_temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "wplace-inc-test-{}-{}",
            name,
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    // ─── Filename parsing ────────────────────────────────────────────────────

    #[test]
    fn parse_increment_datetime_rounds_to_hours() {
        let dt = parse_increment_datetime("inc_2026-06-03T22-11-00Z.db").unwrap();
        // 2026-06-03T22:11:00Z
        let expected = DateTime::parse_from_rfc3339("2026-06-03T22:11:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        assert_eq!(dt, expected);
    }

    #[test]
    fn increment_date_hours_matches_from_datetime() {
        let dh = increment_date_hours("inc_2026-06-03T22-11-00Z.db").unwrap();
        let dt = parse_increment_datetime("inc_2026-06-03T22-11-00Z.db").unwrap();
        assert_eq!(dh, DateHours::from_datetime(dt));
    }

    #[test]
    fn parse_archive_filename_extracts_week_and_datehours() {
        let name = parse_archive_filename("w71_12095.db").unwrap();
        assert_eq!(name.week, 71);
        assert_eq!(name.date_hours, 12095);
    }

    #[test]
    fn parse_archive_filename_rejects_bad_prefix() {
        assert!(parse_archive_filename("inc_2026-06-03.db").is_err());
        assert!(parse_archive_filename("71_12095.db").is_err());
    }

    #[test]
    fn find_latest_archive_picks_largest_week() {
        let dir = make_temp_dir("latest");
        for n in ["w31_5309.db", "w67_11423.db", "w72_12095.db"] {
            create_archive_db(dir.join(n).to_str().unwrap()).unwrap();
        }
        // Non-archive files must be ignored.
        fs::write(dir.join("inc_2026-06-03T22-11-00Z.db"), b"").unwrap();
        fs::write(dir.join("notes.txt"), b"").unwrap();

        let latest = find_latest_archive(dir.to_str().unwrap()).unwrap().unwrap();
        assert_eq!(
            latest.file_name().unwrap(),
            "w72_12095.db"
        );

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn find_latest_archive_empty_returns_none() {
        let dir = make_temp_dir("empty");
        assert!(find_latest_archive(dir.to_str().unwrap())
            .unwrap()
            .is_none());
        fs::remove_dir_all(&dir).ok();
    }

    // ─── process_increment ───────────────────────────────────────────────────

    #[test]
    fn process_increment_creates_new_history() {
        let png = png_for(palette::WHITE);
        let result = process_increment(IncrementJob {
            x: 5,
            y: 6,
            date_hours: DateHours(100),
            png,
            existing: None,
        })
        .unwrap()
        .unwrap();

        assert_eq!(result.x, 5);
        assert_eq!(result.y, 6);

        let th = TileHistory::from_bytes(&result.data).unwrap();
        let img = th.get(DateHours(100)).unwrap().unwrap();
        assert!(img.indices.iter().all(|&v| v == palette::WHITE));
    }

    #[test]
    fn process_increment_preserves_previous_versions() {
        let existing = history_with(make_tile(palette::BLACK), DateHours(50));
        let png = png_for(palette::WHITE);

        let result = process_increment(IncrementJob {
            x: 0,
            y: 0,
            date_hours: DateHours(100),
            png,
            existing: Some(existing),
        })
        .unwrap()
        .unwrap();

        let th = TileHistory::from_bytes(&result.data).unwrap();
        assert_eq!(th.list().len(), 2);
        assert!(th.get(DateHours(50)).unwrap().unwrap().indices.iter().all(|&v| v == palette::BLACK));
        assert!(th.get(DateHours(100)).unwrap().unwrap().indices.iter().all(|&v| v == palette::WHITE));
    }

    #[test]
    fn process_increment_identical_returns_none() {
        // Existing history already has the same image at the same date; setting it
        // again at a later date that is identical to the previous frame yields no diff.
        let existing = history_with(make_tile(palette::WHITE), DateHours(50));
        let png = png_for(palette::WHITE);

        let result = process_increment(IncrementJob {
            x: 0,
            y: 0,
            date_hours: DateHours(100),
            png,
            existing: Some(existing),
        })
        .unwrap();

        assert!(result.is_none());
    }

    // ─── process_seed ────────────────────────────────────────────────────────

    #[test]
    fn process_seed_takes_latest_image_at_datehours_zero() {
        // Source has two versions; seed must keep only the latest at DateHours(0).
        let mut th = TileHistory { imgs: Default::default() };
        th.set(DateHours(10), make_tile(palette::BLACK)).unwrap();
        th.set(DateHours(20), make_tile(palette::WHITE)).unwrap();
        let source_blob = th.to_bytes();

        let result = process_seed(SeedJob {
            z: 11,
            x: 1,
            y: 2,
            data: source_blob,
        })
        .unwrap()
        .unwrap();

        let seeded = TileHistory::from_bytes(&result.data).unwrap();
        let dates = seeded.list();
        assert_eq!(dates, vec![DateHours(0)]);
        let img = seeded.get(DateHours(0)).unwrap().unwrap();
        assert!(img.indices.iter().all(|&v| v == palette::WHITE));
    }

    #[test]
    fn process_seed_empty_history_none() {
        let empty = TileHistory { imgs: Default::default() }.to_bytes();
        let result = process_seed(SeedJob {
            z: 11,
            x: 0,
            y: 0,
            data: empty,
        });
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    // ─── rename_archive_if_needed ────────────────────────────────────────────

    #[test]
    fn rename_skips_when_datehours_not_greater() {
        let dir = make_temp_dir("rename-skip");
        let path = dir.join("w31_100.db");
        fs::write(&path, b"").unwrap();

        rename_archive_if_needed(&path, ArchiveName { week: 31, date_hours: 100 }, 100).unwrap();
        // unchanged
        assert!(path.exists());
        assert!(!dir.join("w31_200.db").exists());

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rename_updates_filename_when_datehours_greater() {
        let dir = make_temp_dir("rename-do");
        let path = dir.join("w31_100.db");
        fs::write(&path, b"").unwrap();

        rename_archive_if_needed(&path, ArchiveName { week: 31, date_hours: 100 }, 200).unwrap();
        assert!(!path.exists());
        assert!(dir.join("w31_200.db").exists());

        fs::remove_dir_all(&dir).ok();
    }

    // ─── End-to-end: same week ───────────────────────────────────────────────

    #[test]
    fn increment_same_week_updates_latest_archive() -> Result<()> {
        let dir = make_temp_dir("e2e-same");
        let archive_path = dir.join("w31_100.db");
        create_archive_db(archive_path.to_str().unwrap())?;

        // Seed the archive with one existing tile.
        {
            let conn = Connection::open(archive_path.to_str().unwrap())?;
            let blob = history_with(make_tile(palette::BLACK), DateHours(100));
            conn.execute(
                "INSERT INTO tiles (z, x, y, data) VALUES (11, 1, 2, ?1)",
                params![blob],
            )?;
        }

        // Build an increment db in the same week (week 31 = 31*168 = 5208 .. 5208+167).
        // Pick datehours 5209 inside week 31.
        let inc_dt = DateHours(5209).to_datetime();
        let inc_name = format!(
            "inc_{}.db",
            inc_dt.format("%Y-%m-%dT%H-%M-%SZ").to_string()
        );
        let inc_path = dir.join(&inc_name);
        let png = png_for(palette::WHITE);
        create_increment_db(inc_path.to_str().unwrap(), &[(1, 2, png.clone())])?;

        increment(dir.to_str().unwrap(), inc_path.to_str().unwrap())?;

        // The archive should have been renamed to w31_5209.db.
        assert!(!archive_path.exists(), "old archive file should be gone");
        let new_archive = dir.join("w31_5209.db");
        assert!(new_archive.exists(), "archive renamed to w31_5209.db");

        let conn = Connection::open(new_archive.to_str().unwrap())?;
        let data = read_tile(&conn, 11, 1, 2).expect("tile must exist");
        let th = TileHistory::from_bytes(&data).unwrap();
        assert_eq!(th.list().len(), 2);
        assert!(th.get(DateHours(100)).unwrap().unwrap().indices.iter().all(|&v| v == palette::BLACK));
        assert!(th.get(DateHours(5209)).unwrap().unwrap().indices.iter().all(|&v| v == palette::WHITE));

        let version: (i64, String) = conn.query_row(
            "SELECT date, original_file FROM versions WHERE date = ?1",
            params![5209i64],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        assert_eq!(version.0, 5209);
        assert_eq!(version.1, inc_name);

        fs::remove_dir_all(&dir).ok();
        Ok(())
    }

    // ─── End-to-end: new week ───────────────────────────────────────────────

    #[test]
    fn increment_new_week_creates_new_archive() -> Result<()> {
        let dir = make_temp_dir("e2e-new");
        let archive_path = dir.join("w31_5209.db");
        create_archive_db(archive_path.to_str().unwrap())?;

        // Two tiles at z=11 in the existing archive, with the latest being WHITE.
        {
            let conn = Connection::open(archive_path.to_str().unwrap())?;
            let mut th = TileHistory { imgs: Default::default() };
            th.set(DateHours(100), make_tile(palette::BLACK)).unwrap();
            th.set(DateHours(5209), make_tile(palette::WHITE)).unwrap();
            let blob = th.to_bytes();
            conn.execute(
                "INSERT INTO tiles (z, x, y, data) VALUES (11, 1, 2, ?1)",
                params![blob],
            )?;
            // A second tile with a single version.
            conn.execute(
                "INSERT INTO tiles (z, x, y, data) VALUES (11, 3, 4, ?1)",
                params![history_with(make_tile(7), DateHours(5209))],
            )?;
        }

        // Increment from week 32 (datehours 5376 = 168*32, week boundary).
        let inc_dt = DateHours(5376).to_datetime();
        let inc_name = format!(
            "inc_{}.db",
            inc_dt.format("%Y-%m-%dT%H-%M-%SZ").to_string()
        );
        let inc_path = dir.join(&inc_name);
        // Update tile (1,2) to RED in the new week.
        let png = png_for(7);
        create_increment_db(inc_path.to_str().unwrap(), &[(1, 2, png)])?;

        increment(dir.to_str().unwrap(), inc_path.to_str().unwrap())?;

        // New archive created, old one untouched.
        let new_archive = dir.join("w32_5376.db");
        assert!(new_archive.exists(), "new archive w32_5376.db created");
        assert!(archive_path.exists(), "old archive preserved");

        let conn = Connection::open(new_archive.to_str().unwrap())?;

        // Seeded base then updated: (1,2) has base WHITE at DateHours(0), diff at 5376.
        let blob12 = read_tile(&conn, 11, 1, 2).expect("seeded tile (1,2) must exist");
        let th12 = TileHistory::from_bytes(&blob12).unwrap();
        assert_eq!(th12.list(), vec![DateHours(0), DateHours(5376)]);
        assert!(th12.get(DateHours(0)).unwrap().unwrap().indices.iter().all(|&v| v == palette::WHITE));

        // Seeded base: (3,4) should be RED at DateHours(0).
        let blob34 = read_tile(&conn, 11, 3, 4).expect("seeded tile (3,4) must exist");
        let th34 = TileHistory::from_bytes(&blob34).unwrap();
        assert!(th34.get(DateHours(0)).unwrap().unwrap().indices.iter().all(|&v| v == 7));

        // At DateHours(5376) the increment's RED diff is applied on top of the base.
        let img = th12.get(DateHours(5376)).unwrap().unwrap();
        assert!(img.indices.iter().all(|&v| v == 7), "tile updated to RED");

        let version: i64 = conn.query_row(
            "SELECT date FROM versions WHERE date = ?1",
            params![5376i64],
            |r| r.get(0),
        )?;
        assert_eq!(version, 5376);

        fs::remove_dir_all(&dir).ok();
        Ok(())
    }

    // ─── End-to-end: ingest (increment + merge) on a new week ─────────────

    #[test]
    fn ingest_new_week_merges_new_archive() -> Result<()> {
        let dir = make_temp_dir("e2e-ingest-new");
        let old_path = dir.join("w85_14425.db");
        create_archive_db(old_path.to_str().unwrap())?;

        // Old week archive: one z=11 child and its z=9 parent, both BLACK at 14425.
        {
            let conn = Connection::open(old_path.to_str().unwrap())?;
            let blob = history_with(make_tile(palette::BLACK), DateHours(14425));
            conn.execute(
                "INSERT INTO tiles (z, x, y, data) VALUES (11, 0, 0, ?1)",
                params![blob.clone()],
            )?;
            conn.execute(
                "INSERT INTO tiles (z, x, y, data) VALUES (9, 0, 0, ?1)",
                params![blob],
            )?;
        }

        // New-week increment at 14449 (week 86) changing tile (0,0) to RED.
        let inc_dt = DateHours(14449).to_datetime();
        let inc_name = format!("inc_{}.db", inc_dt.format("%Y-%m-%dT%H-%M-%SZ").to_string());
        let inc_path = dir.join(&inc_name);
        create_increment_db(inc_path.to_str().unwrap(), &[(0, 0, png_for(7))])?;

        // Same flow as the `ingest` command: increment, then merge the returned path.
        let (archive_path, date_hours) =
            increment(dir.to_str().unwrap(), inc_path.to_str().unwrap())?;
        assert_eq!(date_hours, DateHours(14449));
        assert_eq!(
            archive_path.file_name().and_then(|f| f.to_str()),
            Some("w86_14449.db"),
            "increment() must return the new-week archive, not the old latest one"
        );

        crate::merge::merge(archive_path.to_str().unwrap(), date_hours)?;

        // The new archive's z=9 must have gained the 14449 merged version...
        let new_path = dir.join("w86_14449.db");
        let conn = Connection::open(new_path.to_str().unwrap())?;
        let blob = read_tile(&conn, 9, 0, 0).expect("seeded z=9 tile (0,0) must exist");
        let th = TileHistory::from_bytes(&blob).unwrap();
        assert_eq!(
            th.list(),
            vec![DateHours(0), DateHours(14449)],
            "z=9 of the new archive must contain the merged 14449 version"
        );

        // ...and the old archive must be untouched.
        drop(conn);
        let conn = Connection::open(old_path.to_str().unwrap())?;
        let blob = read_tile(&conn, 9, 0, 0).unwrap();
        let th = TileHistory::from_bytes(&blob).unwrap();
        assert_eq!(th.list(), vec![DateHours(14425)]);

        fs::remove_dir_all(&dir).ok();
        Ok(())
    }

    #[test]
    fn increment_no_existing_archive_errors() -> Result<()> {
        let dir = make_temp_dir("no-archive");
        let inc_path = dir.join("inc_2026-06-03T22-11-00Z.db");
        create_increment_db(inc_path.to_str().unwrap(), &[])?;
        let result = increment(dir.to_str().unwrap(), inc_path.to_str().unwrap());
        assert!(result.is_err());
        fs::remove_dir_all(&dir).ok();
        Ok(())
    }
}
