# makeweek.sh: automatic week listing and building from the HF bucket

Date: 2026-09-01
Status: approved

## Problem

`makeweek.sh` hardcodes the week number, the base `full_*.db` and the list of
daily `inc_*.db` files. Every week, the operator must manually edit the file
lists by browsing the HF bucket. The current hardcoded list for week 83 is
also incomplete (it omits `inc_2026-08-05T01-35-33Z.db`).

## Goal

Replace the hardcoded lists with discovery from the HF bucket:

- `./makeweek.sh` (no arguments) lists every week that can be composed from
  bucket data, with the exact files composing each week.
- `./makeweek.sh <N>` takes only a week number, resolves the files for week
  N from the bucket, downloads them, and builds the week exactly like the
  current script does.

Non-goals: parallel downloads, resumable builds, caching of bucket listings,
JSON output, automatic scheduling.

## Background: week and file semantics

- `DateHours` (wimage) counts hours since the epoch `2025-01-01T00:00:00Z`.
  `week() = datehours / 168` (floor), so weeks run Wednesday 00:00 UTC to the
  next Wednesday 00:00 UTC. Example: week 83 spans 2026-08-05T00:00Z to
  2026-08-12T00:00Z (datehours 13944..14111).
- The bucket `hf://buckets/Hugi-R/wplace-archives` has two folders, `full/`
  and `incremental/`, each containing one file per daily snapshot with
  identical timestamped names (`full_<ts>.db` is the complete snapshot,
  `inc_<ts>.db` is the delta against the previous day). At design time:
  89 snapshots from 2026-06-03T22-11-00Z to 2026-08-31T01-55-56Z, with
  2026-06-05 missing.
- A week N is composed of:
  - **base**: the latest `full_*.db` whose `datehours < 168*N` (the last
    snapshot taken before the week starts);
  - **incs**: every `inc_*.db` whose `datehours` is in `[168*N, 168*(N+1))`,
    sorted ascending.
- `makebase` is called with the default `--datehours 0`, which stores the
  base image at DateHours 0 and inserts **no** row in the `versions` table
  (makebase.rs:567 verifies this). Each ingested inc adds one `versions` row
  at its absolute datehours. A complete week therefore has exactly 7
  versions (7 incs), which satisfies validate.rs's ">7 versions" limit.
- `increment`/`ingest` renames the archive after each ingest to
  `w<week>_<latest datehours>.db` (e.g. `w83_14089.db`), matching the
  tileserver layout `weeks/w<version>_<any>.db`.

## CLI

```
makeweek.sh            # list mode
makeweek.sh <N>        # build week N
```

- `<N>` must be a non-negative integer; anything else prints usage and exits 2.
- Unknown week (no snapshot in `[168*N, 168*(N+1))`) prints an error and exits 1.

## Week selection rule

Shared by list and build mode. For every snapshot filename, parse the
timestamp `YYYY-MM-DDTHH-MM-SSZ`, convert to epoch seconds with
`date -u -d` (dashes in the time part replaced by colons), then compute
`datehours = (epoch - 1735689600) / 3600` (floor) and `week = datehours / 168`.
Week 83 must resolve to base `full_2026-08-04T01-36-27Z.db` plus incs
2026-08-05 through 2026-08-11 (7 files) — this is the acceptance check.

## List mode output

One block per week, ascending, only weeks having at least one snapshot:

```
week 83   2026-08-05 -> 2026-08-12   [complete]
  base: full_2026-08-04T01-36-27Z.db
  incs: inc_2026-08-05T01-35-33Z.db
        inc_2026-08-06T01-36-50Z.db
        inc_2026-08-07T01-42-25Z.db
        inc_2026-08-08T01-36-49Z.db
        inc_2026-08-09T01-40-00Z.db
        inc_2026-08-10T01-38-45Z.db
        inc_2026-08-11T01-40-12Z.db
```

Every inc file is enumerated (no ellipsis in real output). Status tags:

- `[complete]` when the week has a base and exactly 7 incs;
- `[incomplete: k/7 incs]` otherwise;
- `[no base]` when no base exists (week 74, the first week in the bucket).

A week with a base and 0 incs cannot occur from current bucket data but would
be tagged `[incomplete: 0/7 incs]` and refused in build mode. The range shows
the week's start and end dates (UTC).

## Build mode flow

1. Check `./target/release/wpda-pipeline` exists; otherwise error with the
   hint `cargo build --release -p wpda-pipeline` and exit 1.
2. Resolve week N files. Exit 1 if there is no base or no inc, with an
   explanatory message. Warn (non-fatal) when the inc count is below 7.
3. Abort if `$WORKDIR/w<N>_*.db` already exists (a leftover from a previous
   run would be picked by `find_latest_archive` and poison the build).
4. `makebase --base <base> --output "$WORKDIR/w<N>_0.db"` then
   `merge -t 0` on it.
5. For each inc ascending: `hf buckets cp` it to `$DOWNLOADFOLDER`, run
   `ingest --archives "$WORKDIR" --increment <file>`, then `rm` the file
   (same sequential download-process-delete pattern as the current script).
6. Print the resulting archive in `$WORKDIR` (the file renamed by the last
   ingest, e.g. `w83_14089.db`).

Error handling: `set -euo pipefail`; any pipeline failure aborts the script.

## Configuration

Environment-overridable, defaults preserved from the current script:

- `DOWNLOADFOLDER` (default `~/Téléchargements`)
- `WORKDIR` (default `/run/media/system/DataBtrfs/wplace/ramfs/<N>`,
  created with `mkdir -p`; ramfs speed note kept as a comment)

## Implementation

- Pure bash (approach A). No new dependencies beyond `uvx hf` already used.
- One `uvx hf buckets list <prefix> -q` call per folder per invocation, output
  captured to temp files; the `full/` / `incremental/` path prefix is stripped.
- Helper functions: `snapshot_datehours <filename>`, `select_week <N>`
  (echoes base and incs), `list_weeks`, `build_week <N>`.

## Verification

- No bash test harness in this repo. Manual verification:
  - `./makeweek.sh` prints weeks 74..86 with the counts observed in the
    bucket (74: no base, 6 incs; 75..85: complete; 86: 6/7 incs).
  - Week 83 selection matches base `full_2026-08-04T01-36-27Z.db` + 7 incs
    2026-08-05..2026-08-11.
  - `./makeweek.sh 83` in a scratch `WORKDIR` with the real pipeline binary:
    confirm downloads, makebase/merge/ingest steps, renamed final archive
    `w83_14089.db`, and `versions` table containing the 7 inc datehours.
