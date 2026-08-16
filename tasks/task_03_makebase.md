Implements the `makebase --base BASE_DB --output ARCHIVE_DB [--datehours DATEHOURS]` command.
Convert the images from BASE_DB and add them to a new ARCHIVE_DB.

The `wimage` crate already implements everything needed to convert/manipulate the images (`PalettedImage`, `TileHistory`, `DateHours`, ...).

## Input
### BASE_DB
SQLite db containing the base images for the archive.

The sqlite db has these tables:
```sql
CREATE TABLE IF NOT EXISTS tiles (
    x INTEGER NOT NULL,
    y INTEGER NOT NULL,
    data BLOB NOT NULL,
);
```
The data blob is a PNG image. The z level is implicitly 11.

### DATEHOURS
The datehours to set the base images to. Given as a u32 number of hours since
2025-01-01T00:00:00Z. Default to 0.

## Output
### ARCHIVE_DB
A new SQLite db to be created with the base images, using the archive schema from
`common.rs` (`create_empty_archive`):

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
The data blob is a `TileHistory` from `wimage`.

Only the z=11 level is written. Lower zoom levels are built later by the separate
`merge` command, like the `increment` → `merge` flow. `ARCHIVE_DB` may be any
user-chosen path; the user is responsible for naming it `w<week>_<datehours>.db`
so that `increment` can later discover it.

## Processing
For every PNG tile in BASE_DB, store it in ARCHIVE_DB at (z=11, x, y) as a
TileHistory whose only version is the image at DATEHOURS.

For every PNG:
- Read the PNG blob and convert it to Paletted (`PalettedImage.from_png`).
- Create a fresh TileHistory and set the PalettedImage at DATEHOURS (`TileHistory.set`).
  Since the history is new, the image is stored as a full (base) image.
- Serialize the TileHistory (`TileHistory.to_bytes`) and write it to ARCHIVE_DB.

If `DATEHOURS != 0`, also add a row to the `versions` table:
`date=DATEHOURS`, `original_file=BASE_DB file name (name only, not the path)`.

## Error handling
- Bail if ARCHIVE_DB already exists; makebase creates a new archive and must not
  silently overwrite an existing one.
- No PNG size/geometry validation (mirrors `increment.rs`). PNG decode errors and
  other worker errors propagate and abort the run.
- An empty BASE_DB produces an empty (but valid) ARCHIVE_DB.

## Pipeline structure
Follow the reader / worker / writer pattern already used by `increment.rs`
(crossbeam channels, bounded queues, batched transactions, `WIMAGE_MAKEBASE_WORKERS`
env override, defaulting like `increment_worker_count`). Each command module has
its own copy of this pattern.

- Reader: stream `(x, y, data)` rows from BASE_DB.
- Workers: decode PNG, build TileHistory, serialize.
- Writer: batch-insert rows at (z=11, x, y) into ARCHIVE_DB.

## Testing
Follow the test style of `increment.rs` (unit tests on the worker function plus
end-to-end tests through real SQLite with `tempfile`):

- Worker: PNG → TileHistory with a single full image at DATEHOURS (0 and non-zero).
- End-to-end: create a BASE_DB with a few PNGs, run makebase, assert the archive
  has the expected z=11 tiles and that `versions` is populated only when
  `DATEHOURS != 0`.
- End-to-end: empty BASE_DB creates an empty valid archive.
- End-to-end: existing ARCHIVE_DB path errors out.
