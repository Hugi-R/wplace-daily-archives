Implements the `increment --archives ARCHIVES_FOLDER --increment INCREMENT_DB` command.
Convert the images from INCREMENT_DB and add them to the latest archive in ARCHIVES_FOLDER.

The `wimage` crate already implements everything needed to convert/manipulate the images (`PalettedImage`, `TileHistory`, `DateHours`, ...).

## Input
### ARCHIVES_FOLDER
Folder containing SQLite .db file like so:
- w71_12095.db
- w72_12262.db
- w73_12335.db
Named `w<week>_<datehours>.db`, With `week` the weeks number since 2025, and `datehours` the hours number since 2025-01-01T00:00:00Z.

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
The data blob is a `TileHistory` from `wimage`.
Images are tiled in x,y,z pyramid for displaying on a map. Where z represent the zoom level.
- 0 <= z <= 11
- 0 <= x <= 2048 (2^11)
- 0 <= y <= 2048 (2^11)

### INCREMENT_DB
A SQLite db containing png images of tiles that need to be updated. The z level implicitely 11.
The filename indicate the time of the increment: `inc_2026-06-03T22-11-00Z.db`.

The sqlite db has this table:
```sql
CREATE TABLE tiles (
    -- z is 11
    x INTEGER,
    y INTEGER,
    data BLOB,
    PRIMARY KEY (x, y)
);
```
The data blob is a PNG image.

## Output
Update the latest archive in ARCHIVES_FOLDER, or, if the week changed, create a new one.

## Processing
For every image in INCREMENT_DB, add it to its corresponding TileHistory of the latest archive in ARCHIVES_FOLDER. If INCREMENT_DB is from a new (different) week than the latest archive, a new archive is created.

### New archive creation
Special step to prepare a new archive when INCREMENT_DB week number `W` is different than the latest week number `W-1`.

Create a new DB `wW_DATEHOURS.db` withe empty tables `tiles` and `versions`. DATEHOURS is calculated from the name of the INCREMENT_DB. 

For ALL existing tiles (any x,y,z) in the latest archive:
- Read the TileHistory blob.
- Get the latest image from that TileHistory (`TileHistory.from_byte(blob)` then `TileHistory.get(DateHours.max())`).
- Create a new TileHistory, and set that image as DateHours 0 (`TileHistory.set`).
- Write the blob (`TileHistory.to_bytes`) of that new TileHistory to the new DB.

This will setup the base, full images, of that new archive (all addition we then do to the TileHistory will be increment diff).

### Usual increment step
For every image in INCREMENT_DB (usually ~60_000 PNGs, ~4GB ), add it to its corresponding TileHistory, at the datehours calculated from the timestamp of the INCREMENT_DB filename.

Add the datehours and filename the the `versions` table.

For every PNG:
- Read PNG blob and convert it to Paletted (`PalettedImage.from_png`)
- Read TileHistory blob, deserialize and set the PalletedImage at the datehours.
- Serialize the TileHistory and write to the latest archive.

If necessary, rename the latest archive to update the datehours in its name.
