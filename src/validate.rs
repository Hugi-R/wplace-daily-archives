use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use crossbeam_channel::{bounded, Receiver, RecvTimeoutError, SendTimeoutError, Sender};
use rusqlite::{params, Connection};

use wimage::tilehistory::{DateHours, TileHistory};

const WRITE_BATCH_SIZE: usize = 256;
const DEFAULT_MAX_WORKERS: usize = 16;

// ─── Types ───────────────────────────────────────────────────────────────────

struct ValidateJob {
    z: i32,
    x: i32,
    y: i32,
    data: Vec<u8>,
    versions: Vec<DateHours>,
}

struct ValidateResult {
    z: i32,
    x: i32,
    y: i32,
    data: Vec<u8>,
}

// ─── SQLite helpers ──────────────────────────────────────────────────────────

fn open_db(path: &str) -> Result<Connection> {
    let conn = Connection::open(path)
        .with_context(|| format!("open database {path}"))?;

    conn.busy_timeout(Duration::from_secs(120))
        .context("set SQLite busy timeout")?;

    Ok(conn)
}

fn enable_wal(path: &str) -> Result<()> {
    let conn = Connection::open(path)
        .with_context(|| format!("open database {path}"))?;

    conn.pragma_update(None, "journal_mode", "WAL")
        .context("enable WAL mode")?;

    Ok(())
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

/// Streams every tile from the database, sending jobs and periodic total counts.
fn read_jobs(
    db_path: &str,
    job_tx: Sender<ValidateJob>,
    total_tx: Sender<u64>,
    cancel: &AtomicBool,
) -> Result<()> {
    let conn = open_db(db_path)?;

    // Read versions
    let mut fetch = conn.prepare(
        "SELECT date FROM versions ORDER BY date ASC",
    )?;
    let mut versions = Vec::new();
    let mut rows = fetch.query([])?;
    while let Some(row) = rows.next()? {
        let date: i32 = row.get(0)?;
        versions.push(DateHours(date as u32));
    }
    // If there's more than 7 dates, crash, because we want a DB to be a week of history
    if versions.len() > 7 {
        bail!("database has more than 7 versions, which is not supported");
    }

    // Read tiles
    let mut fetch = conn.prepare(
        "SELECT z, x, y, data FROM tiles",
    )?;

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

        if !send_with_cancel(
            &job_tx,
            cancel,
            ValidateJob { z, x, y, data, versions: versions.clone() },
        )? {
            return Ok(());
        }

        count += 1;

        // Report progress periodically.
        if count % 1024 == 0 {
            if total_tx.send(count).is_err() {
                return Ok(());
            }
        }
    }

    // Send final count so the writer knows the total.
    let _ = total_tx.send(count);

    Ok(())
}

// ─── Worker ──────────────────────────────────────────────────────────────────

fn process_job(job: ValidateJob) -> Result<Option<ValidateResult>> {
    let ValidateJob { z, x, y, data, versions } = job;

    let mut history = TileHistory::from_bytes(&data)
        .with_context(|| format!("decode tile z={}, x={}, y={}", z, x, y))?;

    let was_valid = history.validate_and_fix(Some(versions), true)
        .with_context(|| format!("validate tile z={}, x={}, y={}", z, x, y))?;

    if was_valid {
        Ok(None)
    } else {
        Ok(Some(ValidateResult {
            z,
            x,
            y,
            data: history.to_bytes(),
        }))
    }
}

// ─── Writer ──────────────────────────────────────────────────────────────────

fn flush_batch(
    conn: &mut Connection,
    batch: &mut Vec<ValidateResult>,
) -> Result<()> {
    let tx = conn.transaction().context("start write transaction")?;

    {
        let mut stmt = tx.prepare_cached(
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

fn writer_loop(
    db_path: &str,
    result_rx: Receiver<ValidateResult>,
    total_rx: Receiver<u64>,
) -> Result<()> {
    let mut conn = open_db(db_path)?;

    conn.pragma_update(None, "synchronous", "NORMAL")
        .context("set SQLite synchronous=NORMAL")?;

    let start = Instant::now();
    let mut last_report = start;
    let mut total_checked = 0u64;
    let mut written = 0usize;

    let mut batch = Vec::with_capacity(WRITE_BATCH_SIZE);

    loop {
        // Drain total count updates.
        while let Ok(count) = total_rx.try_recv() {
            total_checked = count;
        }

        match result_rx.recv_timeout(Duration::from_secs(1)) {
            Ok(result) => {
                if result.data.is_empty() {
                    // Delete the tile if the history is empty after validation.
                    // No batching, this is very rare.
                    conn.execute(
                        "DELETE FROM tiles WHERE z = ?1 AND x = ?2 AND y = ?3",
                        params![result.z, result.x, result.y],
                    )
                    .with_context(|| {
                        format!(
                            "delete tile z={}, x={}, y={}",
                            result.z, result.x, result.y
                        )
                    })?;
                }

                batch.push(result);
                written += 1;

                if batch.len() >= WRITE_BATCH_SIZE {
                    flush_batch(&mut conn, &mut batch)?;
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                // No new results; check for total updates and report.
            }
            Err(RecvTimeoutError::Disconnected) => {
                // Get final total count.
                while let Ok(count) = total_rx.try_recv() {
                    total_checked = count;
                }
                break;
            }
        }

        if last_report.elapsed() >= Duration::from_secs(10) {
            let elapsed = start.elapsed().as_secs_f64().max(0.001);
            eprintln!(
                "  {total_checked} tiles checked, {written} fixed ({:.0} tiles/s)",
                total_checked as f64 / elapsed,
            );
            last_report = Instant::now();
        }
    }

    if !batch.is_empty() {
        flush_batch(&mut conn, &mut batch)?;
    }

    let elapsed = start.elapsed().as_secs_f64().max(0.001);
    eprintln!(
        "  Done: {total_checked} tiles checked, {written} fixed ({:.0} tiles/s)",
        total_checked as f64 / elapsed,
    );

    Ok(())
}

// ─── Thread helpers ──────────────────────────────────────────────────────────

fn join_thread(
    thread: JoinHandle<Result<()>>,
    name: &str,
) -> Result<()> {
    match thread.join() {
        Ok(result) => result,
        Err(_) => Err(anyhow!("{name} thread panicked")),
    }
}

// ─── Pipeline ────────────────────────────────────────────────────────────────

fn validate_worker_count() -> usize {
    let automatic = thread::available_parallelism()
        .map(|count| count.get().saturating_sub(1))
        .unwrap_or(1)
        .min(DEFAULT_MAX_WORKERS)
        .max(1);

    std::env::var("WIMAGE_VALIDATE_WORKERS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|&count| count > 0)
        .unwrap_or(automatic)
}

pub fn validate(db_path: &str) -> Result<()> {
    let worker_count = validate_worker_count();
    let channel_capacity = (worker_count * 2).max(4);

    let (job_tx, job_rx) = bounded::<ValidateJob>(channel_capacity);
    let (result_tx, result_rx) = bounded::<ValidateResult>(channel_capacity);
    let (total_tx, total_rx) = bounded::<u64>(channel_capacity);

    // Enable WAL once from the main thread to avoid lock contention
    // between reader and writer trying to set it concurrently.
    enable_wal(db_path)?;

    let cancelled = Arc::new(AtomicBool::new(false));

    // Writer thread
    let writer_path = db_path.to_owned();
    let writer_cancel = Arc::clone(&cancelled);
    let writer = thread::spawn(move || {
        let result = writer_loop(&writer_path, result_rx, total_rx);
        if result.is_err() {
            writer_cancel.store(true, Ordering::Relaxed);
        }
        result
    });

    // Worker threads
    let processor = Arc::new(process_job);
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

    // Drop main senders so workers see EOF when the reader finishes.
    drop(job_rx);
    drop(result_tx);

    // Reader thread
    let reader_path = db_path.to_owned();
    let reader_cancel = Arc::clone(&cancelled);
    let reader = thread::spawn(move || {
        let result = read_jobs(&reader_path, job_tx, total_tx, &reader_cancel);
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
