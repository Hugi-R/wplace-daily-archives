
use std::time::{Duration};
use anyhow::{Context, Result};
use rusqlite::{Connection};

// ─── SQLite helpers ──────────────────────────────────────────────────────────

pub fn open_db(path: &str) -> Result<Connection> {
    let conn = Connection::open(path)
        .with_context(|| format!("open database {path}"))?;

    conn.busy_timeout(Duration::from_secs(30))
        .context("set SQLite busy timeout")?;

    Ok(conn)
}

pub fn enable_wal(path: &str) -> Result<()> {
    let conn = Connection::open(path)
        .with_context(|| format!("open database {path}"))?;

    conn.pragma_update(None, "journal_mode", "WAL")
        .context("enable WAL mode")?;

    Ok(())
}

pub fn create_empty_archive(path: &str) -> Result<()> {
    let conn = open_db(path)?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS tiles (
            z INTEGER NOT NULL,
            x INTEGER NOT NULL,
            y INTEGER NOT NULL,
            data BLOB NOT NULL,
            PRIMARY KEY (z, x, y)
        );
        CREATE INDEX IF NOT EXISTS tiles_z_y_x_idx ON tiles (z, y, x);
        CREATE TABLE IF NOT EXISTS versions (
            date INTEGER PRIMARY KEY,
            original_file TEXT
        );",
    )
    .with_context(|| format!("create empty archive tables at {path}"))?;
    Ok(())
}