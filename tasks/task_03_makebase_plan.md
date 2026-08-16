# makebase Command Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement a `makebase --base BASE_DB --output ARCHIVE_DB [--datehours DATEHOURS]` command that converts a set of PNG tiles into a new archive SQLite db.

**Architecture:** New `pipeline/src/makebase.rs` module following the existing reader / worker / writer pipeline pattern used by `increment.rs` (crossbeam channels, bounded queues, batched SQLite transactions). Base PNGs are streamed from BASE_DB, workers convert each PNG into a fresh `TileHistory` at `datehours`, and a single writer batch-inserts them at z=11. `main.rs` gets a `Makebase` subcommand.

**Tech Stack:** Rust, `rusqlite` (bundled SQLite), `crossbeam-channel`, `clap`, `anyhow`, `tempfile` (dev), `wimage` crate (`PalettedImage`, `TileHistory`, `DateHours`).

## Global Constraints

(Violating any of these fails the task set.)

1. Only z=11 tiles are written. No lower zoom levels are merged; the separate `merge` command handles that.
2. `ARCHIVE_DB` may be any user-chosen path, but `makebase` must bail if that file already exists.
3. `--datehours` is a `u32` number of hours since `2025-01-01T00:00:00Z`, defaulting to `0`.
4. A `versions` row (`date`, `original_file`) is added ONLY when `datehours != 0`, with `original_file` = BASE_DB file name (name only, not the path).
5. No PNG geometry/size validation (mirrors `increment.rs`). Decode errors still abort the run.
6. `TileHistory` is always fresh (empty history), so the image is stored as a full base image.
7. Worker count defaults like `increment.rs` and honors `WIMAGE_MAKEBASE_WORKERS`.
8. Run from the workspace repo root (`/run/media/system/DataBtrfs/wplace/wplace-daily-archives`). The pipeline crate is `pipeline` (package `wpda-pipeline`).
9. Tests in `#[cfg(test)] mod tests` at the bottom of `pipeline/src/makebase.rs`. Tests only run once `mod makebase;` is declared in `main.rs`.

---

### Task 1: `process_job` — PNG → TileHistory worker

**Files:**
- Modify: `pipeline/src/main.rs` (add `mod makebase;`)
- Create/Replace: `pipeline/src/makebase.rs` (currently a doc comment only)
- Test: `#[cfg(test)] mod tests` in `pipeline/src/makebase.rs`

**Interfaces:**
- Consumes: `wimage::PalettedImage::from_png`, `wimage::tilehistory::{TileHistory, DateHours}`
- Produces:
  - `pub(crate) struct MakebaseJob { x: i32, y: i32, date_hours: wimage::tilehistory::DateHours, png: Vec<u8> }`
  - `pub(crate) struct MakebaseResult { x: i32, y: i32, data: Vec<u8> }`
  - `pub(crate) fn process_job(job: MakebaseJob) -> anyhow::Result<Option<MakebaseResult>>` — always returns `Ok(Some(...))`; the image is stored as a full image because the history is fresh.

- [ ] **Step 1: Register the module in `main.rs`**

In `pipeline/src/main.rs`, change the module block (lines 7-10) from:

```rust
mod merge;
mod validate;
mod increment;
mod common;
```

to:

```rust
mod merge;
mod validate;
mod increment;
mod makebase;
mod common;
```

- [ ] **Step 2: Write the failing tests (write `pipeline/src/makebase.rs`)**

Replace the entire current content of `pipeline/src/makebase.rs` with:

```rust
/// Implements the `makebase --base BASE_DB --output ARCHIVE_DB [--datehours DATEHOURS]` command.
/// Convert the base image PNGs from BASE_DB into a fresh ARCHIVE_DB as TileHistory blobs at z=11.
///
/// See `tasks/task_03_makebase.md` for the full spec.

#[cfg(test)]
mod tests {
    use wimage::palette;
    use wimage::tilehistory::{DateHours, TileHistory};
    use wimage::PalettedImage;

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
```

(The tests reference `process_job`, `MakebaseJob`, `MakebaseResult`, which do not exist yet.)

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test -p wpda-pipeline process_job`
Expected: FAIL (compile error — `MakebaseJob`, `MakebaseResult`, `process_job` not defined).

- [ ] **Step 4: Implement the worker**

Replace the entire content of `pipeline/src/makebase.rs` with:

```rust
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
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p wpda-pipeline process_job`
Expected: PASS (2 tests).

- [ ] **Step 6: Commit**

```bash
git add pipeline/src/makebase.rs pipeline/src/main.rs
git commit -m "feat(makebase): add PNG-to-TileHistory worker"
```

---

### Task 2: Pipeline (`read_jobs`, writer, `makebase()`) + end-to-end tests

**Files:**
- Modify: `pipeline/src/makebase.rs` (extend Task 1's file)
- Test: `#[cfg(test)] mod tests` in `pipeline/src/makebase.rs`

**Interfaces:**
- Consumes: `crate::common::{open_db, enable_wal, create_empty_archive}`, `crate::increment::add_version`, `MakebaseJob`, `MakebaseResult`, `process_job` from Task 1.
- Produces:
  - `pub fn makebase(base_db: &str, output_db: &str, date_hours: wimage::tilehistory::DateHours) -> anyhow::Result<()>`
- Writes to the archive: rows `(z=11, x, y, TileHistory blob)`; optionally a `versions` row.

- [ ] **Step 1: Write the failing end-to-end tests (append inside the `tests` module)**

Inside `#[cfg(test)] mod tests` in `pipeline/src/makebase.rs`, immediately before the closing `}` of the module, add:

```rust
    fn create_base_db(path: &str, tiles: &[(i32, i32, Vec<u8>)]) -> Result<()> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "CREATE TABLE tiles (
                x INTEGER NOT NULL,
                y INTEGER NOT NULL,
                data BLOB NOT NULL,
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

    #[test]
    fn makebase_creates_archive_from_pngs() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let base = tmp.path().join("base.db");
        let out = tmp.path().join("w0_0.db");

        create_base_db(
            base.to_str().unwrap(),
            &[(1, 2, png_for(palette::WHITE)), (3, 4, png_for(7))],
        )?;

        makebase(base.to_str().unwrap(), out.to_str().unwrap(), DateHours(0))?;

        let conn = Connection::open(out.to_str().unwrap())?;

        let t12 = read_tile(&conn, 11, 1, 2).expect("tile (11,1,2) must exist");
        let th12 = TileHistory::from_bytes(&t12).unwrap();
        assert_eq!(th12.list(), vec![DateHours(0)]);
        assert!(
            th12.get(DateHours(0))
                .unwrap()
                .unwrap()
                .indices
                .iter()
                .all(|&v| v == palette::WHITE)
        );

        let t34 = read_tile(&conn, 11, 3, 4).expect("tile (11,3,4) must exist");
        let th34 = TileHistory::from_bytes(&t34).unwrap();
        assert!(
            th34.get(DateHours(0))
                .unwrap()
                .unwrap()
                .indices
                .iter()
                .all(|&v| v == 7)
        );

        // No versions row when datehours == 0.
        let count: i64 =
            conn.query_row("SELECT COUNT(*) FROM versions", [], |r| r.get(0))?;
        assert_eq!(count, 0);

        Ok(())
    }

    #[test]
    fn makebase_adds_version_row_when_datehours_nonzero() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let base = tmp.path().join("base.db");
        let out = tmp.path().join("w3_500.db");

        create_base_db(base.to_str().unwrap(), &[(0, 0, png_for(palette::BLACK))])?;

        makebase(base.to_str().unwrap(), out.to_str().unwrap(), DateHours(500))?;

        let conn = Connection::open(out.to_str().unwrap())?;
        let (date, original_file): (i64, String) = conn.query_row(
            "SELECT date, original_file FROM versions WHERE date = ?1",
            params![500i64],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        assert_eq!(date, 500);
        assert_eq!(original_file, "base.db");

        Ok(())
    }

    #[test]
    fn makebase_empty_base_creates_empty_archive() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let base = tmp.path().join("base.db");
        let out = tmp.path().join("w0_0.db");

        create_base_db(base.to_str().unwrap(), &[])?;

        makebase(base.to_str().unwrap(), out.to_str().unwrap(), DateHours(0))?;

        let conn = Connection::open(out.to_str().unwrap())?;
        let count: i64 =
            conn.query_row("SELECT COUNT(*) FROM tiles", [], |r| r.get(0))?;
        assert_eq!(count, 0);
        Ok(())
    }

    #[test]
    fn makebase_existing_output_errors() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let base = tmp.path().join("base.db");
        let out = tmp.path().join("w0_0.db");

        create_base_db(base.to_str().unwrap(), &[])?;
        create_empty_archive(out.to_str().unwrap())?;

        let result = makebase(base.to_str().unwrap(), out.to_str().unwrap(), DateHours(0));
        assert!(result.is_err());
        Ok(())
    }
```

(The tests reference `makebase`, `Connection`, `params`, `create_empty_archive` — none in scope yet.)

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p wpda-pipeline makebase -- --nocapture`
Expected: FAIL (compile error — `makebase` undefined, `Connection`/`params`/`create_empty_archive` not in scope).

- [ ] **Step 3: Implement the full pipeline**

Replace every line of `pipeline/src/makebase.rs` above the `// ─── Tests ───` marker with the following (keep the module doc comment at the very top; the whole non-test half of the file now becomes):

```rust
use std::io::Cursor;
use std::path::Path;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use crossbeam_channel::{bounded, Receiver, SendTimeoutError, Sender};
use rusqlite::{params, Connection};

use wimage::tilehistory::{DateHours, TileHistory};
use wimage::PalettedImage;

use crate::common::{create_empty_archive, enable_wal, open_db};
use crate::increment::add_version;

const Z_TARGET: i32 = 11;
const WRITE_BATCH_SIZE: usize = 256;
const DEFAULT_MAX_WORKERS: usize = 16;

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

/// Stream every PNG tile from BASE_DB.
fn read_jobs(
    base_db: &str,
    date_hours: DateHours,
    job_tx: Sender<MakebaseJob>,
    total_tx: Sender<u64>,
    cancel: &AtomicBool,
) -> Result<()> {
    let conn = open_db(base_db)?;

    let mut fetch = conn.prepare("SELECT x, y, data FROM tiles ORDER BY x, y")?;

    let mut rows = fetch.query([])?;
    let mut count = 0u64;

    while let Some(row) = rows.next()? {
        if cancel.load(Ordering::Relaxed) {
            return Ok(());
        }

        let x: i32 = row.get(0)?;
        let y: i32 = row.get(1)?;
        let png: Vec<u8> = row.get(2)?;

        let job = MakebaseJob { x, y, date_hours, png };

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

// ─── Writer ──────────────────────────────────────────────────────────────────

fn flush_batch(
    conn: &mut Connection,
    batch: &mut Vec<MakebaseResult>,
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

fn writer_loop(
    db_path: &str,
    result_rx: Receiver<MakebaseResult>,
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
                    flush_batch(&mut conn, &mut batch)?;
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
        flush_batch(&mut conn, &mut batch)?;
    }

    let elapsed = start.elapsed().as_secs_f64().max(0.001);
    eprintln!(
        "  Done: {total_read} tiles read, {written} written ({:.0} tiles/s)",
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

fn makebase_worker_count() -> usize {
    let automatic = thread::available_parallelism()
        .map(|count| count.get().saturating_sub(1))
        .unwrap_or(1)
        .min(DEFAULT_MAX_WORKERS)
        .max(1);

    std::env::var("WIMAGE_MAKEBASE_WORKERS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|&count| count > 0)
        .unwrap_or(automatic)
}

// ─── Pipeline runner ─────────────────────────────────────────────────────────

fn run_pipeline<J, Res, Reader, Writer>(
    output_db: &str,
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
    let worker_count = makebase_worker_count();
    let channel_capacity = (worker_count * 2).max(4);

    let (job_tx, job_rx) = bounded::<J>(channel_capacity);
    let (result_tx, result_rx) = bounded::<Res>(channel_capacity);
    let (total_tx, total_rx) = bounded::<u64>(channel_capacity);

    let cancelled = Arc::new(AtomicBool::new(false));

    let writer_path = output_db.to_owned();
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

// ─── Public API ──────────────────────────────────────────────────────────────

pub fn makebase(base_db: &str, output_db: &str, date_hours: DateHours) -> Result<()> {
    if Path::new(output_db).exists() {
        bail!("output archive already exists: {output_db}");
    }

    eprintln!(
        "Creating base archive {output_db} from {base_db} at datehours={} ({}) ...",
        date_hours.0,
        date_hours.to_datetime(),
    );

    create_empty_archive(output_db)?;
    enable_wal(output_db)?;

    let base_path = base_db.to_owned();
    let output_path = output_db.to_owned();

    run_pipeline(
        &output_path,
        move |job_tx, total_tx, cancel| {
            read_jobs(&base_path, date_hours, job_tx, total_tx, &cancel)
        },
        process_job,
        writer_loop,
    )?;

    if date_hours != DateHours(0) {
        let base_filename = Path::new(base_db)
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| anyhow!("base db path has no filename: {base_db}"))?;
        add_version(output_db, date_hours, base_filename)?;
    }

    Ok(())
}
```

Keep the `// ─── Tests ───` marker and the test module exactly as they are.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p wpda-pipeline`
Expected: PASS (2 worker tests + 4 end-to-end tests; the existing merge/increment/validate tests still pass).

- [ ] **Step 5: Commit**

```bash
git add pipeline/src/makebase.rs
git commit -m "feat(makebase): add reader/worker/writer pipeline and makebase()"
```

---

### Task 3: Wire the `Makebase` CLI subcommand

**Files:**
- Modify: `pipeline/src/main.rs`

**Interfaces:**
- Consumes: `makebase::makebase(base_db: &str, output_db: &str, date_hours: DateHours) -> Result<()>` from Task 2.
- Produces: the CLI surface `wplace-daily-archives makebase --base BASE_DB --output ARCHIVE_DB [--datehours DATEHOURS]`.

- [ ] **Step 1: Add the `Makebase` subcommand variant**

In the `Command` enum in `pipeline/src/main.rs`, after the `Ingest` variant's closing brace (line 51) and before the enum's final `}` (line 52), add:

```rust
    /// Create a new base archive from PNG tiles.
    Makebase {
        /// Path to the SQLite base db (PNG tiles at z=11)
        #[arg(long)]
        base: String,
        /// Path to the new archive SQLite db to create
        #[arg(long)]
        output: String,
        /// Datehour: hours since 2025-01-01T00:00:00Z (default 0)
        #[arg(long, default_value_t = 0)]
        datehours: u32,
    },
```

- [ ] **Step 2: Add the match arm**

In `fn main`, after the `Command::Ingest { ... }` arm (ends at line 96) and before the match's closing `};` (line 97), add:

```rust
        Command::Makebase { base, output, datehours } => {
            let date_hours = DateHours(datehours);
            eprintln!(
                "Creating base archive {output} from {base} at datehour={} ({})",
                date_hours.0,
                date_hours.to_datetime(),
            );
            makebase::makebase(&base, &output, date_hours).with_context(|| "makebase failed")
        }
```

- [ ] **Step 3: Build and run all tests**

Run: `cargo build -p wpda-pipeline`
Expected: builds with no errors or warnings.

Run: `cargo test -p wpda-pipeline`
Expected: PASS (all tests).

- [ ] **Step 4: Verify the CLI help and a real smoke run**

Run: `cargo run -p wpda-pipeline -- makebase --help`
Expected: shows `makebase` usage with `--base`, `--output`, `--datehours`.

Smoke test with a real 1000x1000 PNG tile from the wimage testdata (uses `python3` + `sqlite3` module):

```bash
SMOKE=$(mktemp -d)
python3 - "$SMOKE" <<'PY'
import sqlite3, sys, pathlib
smoke = pathlib.Path(sys.argv[1])
png = pathlib.Path("/run/media/system/DataBtrfs/wplace/wplace-image/wimage/src/testdata/00.png")
conn = sqlite3.connect(smoke / "base.db")
conn.execute("CREATE TABLE tiles (x INTEGER NOT NULL, y INTEGER NOT NULL, data BLOB NOT NULL, PRIMARY KEY (x, y))")
conn.execute("INSERT INTO tiles (x, y, data) VALUES (1, 2, ?)", (png.read_bytes(),))
conn.commit(); conn.close()
PY
cargo run -p wpda-pipeline -- makebase --base "$SMOKE/base.db" --output "$SMOKE/w0_0.db" --datehours 0
python3 - "$SMOKE" <<'PY'
import sqlite3, sys, pathlib
smoke = pathlib.Path(sys.argv[1])
conn = sqlite3.connect(smoke / "w0_0.db")
n = conn.execute("SELECT COUNT(*) FROM tiles WHERE z = 11").fetchone()[0]
assert n == 1, f"expected 1 tile, got {n}"
assert conn.execute("SELECT COUNT(*) FROM versions").fetchone()[0] == 0
print("smoke test OK")
PY
rm -rf "$SMOKE"
```

Expected: prints “Creating base archive … at datehour=0 (2025-01-01T00:00:00Z) …”, then “smoke test OK”. If `python3` is unavailable, skip the smoke test; the `cargo test` suite already covers this path.

- [ ] **Step 5: Commit**

```bash
git add pipeline/src/main.rs
git commit -m "feat(makebase): wire makebase CLI subcommand"
```

---

## Final verification

Run from the repo root:

```bash
cargo build -p wpda-pipeline && cargo test -p wpda-pipeline
```

Expected: build succeeds, all tests pass.

```bash
cargo run -p wpda-pipeline -- makebase --help
```

Expected: the `makebase` subcommand is listed with `--base`, `--output`, `--datehours`.