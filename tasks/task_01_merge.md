# Task
Implement a `merge -t DATEHOUR INPUTDB` command.
The merge operation take images on a x,y,z grid from INPUTDB and merge them into a single image.

The `wimage` crate already implements everything needed to manipulate the images (`PalettedImage`, `TileHistory`, `downscale_mode_weighted_2x2`, ...).

Follow TDD best practice.
Start by creating synthetic test data and test cases with expected outputs.
Then write the code function by function and test they actually perform their expected work.

## Merge
For a Z level, take the 2x2 grid from Z-1 and merge them, forming a single image.

| Z levels | Processing |
| 11 | original, already exist, skip |
| 10-0 | 2x2 merge from z-1 |
Level z-1 need to be finished before starting level z.

### Processing
#### Single Reader
- Read z-1 TileHistory from sqlite (2x2 grid). Note the tiles may not exist. If all tiles are missing skip the job. If at least one tile exists treat the missing ones as empty.
- Read the z TileHistory from sqlite, its content will be updated by the job. Note the tile may not exist/be empty.
- Reads can be batched.
- Send jobs to crossbeam channel.

#### Many Workers
- read job channel.
- decode+decompress TileHistory image for T. (wimage)
- merge images. (wimage)
- update result TileHistory with the new image for T. (wimage)
- send to result channel.

#### Single Writer
- read result channel.
- prepare batch transaction and wait for timer/enough results.
- run transaction.

#### Code skeleton for arbitrary job
```rust
use rusqlite::{Connection, params};
use crossbeam_channel::{bounded, Sender, Receiver};
use rayon::prelude::*;
use std::thread;

struct Job { id: i64, blob: Vec<u8> }
struct Result { id: i64, blob: Vec<u8> }

fn run(db_path: &str) -> anyhow::Result<()> {
    let (job_tx, job_rx) = bounded::<Job>(2000);
    let (res_tx, res_rx) = bounded::<Result>(2000);

    // Reader thread
    let reader_path = db_path.to_string();
    let reader = thread::spawn(move || {
        let conn = Connection::open(&reader_path).unwrap();
        conn.pragma_update(None, "journal_mode", "WAL").unwrap();
        let mut last_id = 0i64;
        loop {
            let mut stmt = conn.prepare(
                "SELECT id, data FROM jobs WHERE id > ?1 ORDER BY id LIMIT 5000"
            ).unwrap();
            let rows: Vec<Job> = stmt.query_map(params![last_id], |r| {
                Ok(Job { id: r.get(0)?, blob: r.get(1)? })
            }).unwrap().filter_map(Result::ok).collect();
            if rows.is_empty() { break; }
            last_id = rows.last().unwrap().id;
            for j in rows { job_tx.send(j).unwrap(); }
        }
        // job_tx dropped here -> closes channel
    });

    // Worker pool (rayon over the receiver via a bridging loop, or scoped threads)
    let n_workers = num_cpus::get();
    let mut handles = vec![];
    for _ in 0..n_workers {
        let job_rx = job_rx.clone();
        let res_tx = res_tx.clone();
        handles.push(thread::spawn(move || {
            for job in job_rx {
                let decompressed = zstd::decode_all(&job.blob[..]).unwrap();
                let processed = process(decompressed); // CPU-heavy work
                let compressed = zstd::encode_all(&processed[..], 3).unwrap();
                res_tx.send(Result { id: job.id, blob: compressed }).unwrap();
            }
        }));
    }
    drop(res_tx); // original sender; workers hold their own clones

    // Writer thread: batched commits
    let writer_path = db_path.to_string();
    let writer = thread::spawn(move || {
        let mut conn = Connection::open(&writer_path).unwrap();
        conn.pragma_update(None, "synchronous", "NORMAL").unwrap();
        let mut batch = Vec::with_capacity(2000);
        let flush = |conn: &mut Connection, batch: &mut Vec<Result>| {
            let tx = conn.transaction().unwrap();
            {
                let mut stmt = tx.prepare_cached(
                    "UPDATE jobs SET result = ?1 WHERE id = ?2"
                ).unwrap();
                for r in batch.drain(..) {
                    stmt.execute(params![r.blob, r.id]).unwrap();
                }
            }
            tx.commit().unwrap();
        };
        for r in res_rx {
            batch.push(r);
            if batch.len() >= 2000 { flush(&mut conn, &mut batch); }
        }
        if !batch.is_empty() { flush(&mut conn, &mut batch); }
    });

    reader.join().unwrap();
    for h in handles { h.join().unwrap(); }
    writer.join().unwrap();
    Ok(())
}

fn process(data: Vec<u8>) -> Vec<u8> { /* your CPU-heavy logic */ data }
```

### Input
- SQLite db of tilehistory for z=11
- the datehour time T to process

The sqlite db has these tables:
```sql
CREATE TABLE IF NOT EXISTS tiles (
    z INTEGER NOT NULL,
    x INTEGER NOT NULL,
    y INTEGER NOT NULL,
    data BLOB NOT NULL,
    PRIMARY KEY (z, x, y)
);

CREATE TABLE IF NOT EXISTS versions (
    date INTEGER PRIMARY KEY,
    original_file TEXT
);
```
The data blob is a `TileHistory` from `wimage`

### Output
- the same db, but with z<=10 populated for datehour time T.