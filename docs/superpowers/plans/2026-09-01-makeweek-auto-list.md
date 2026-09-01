# makeweek.sh Automatic Week Listing and Building Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the hardcoded file lists in `makeweek.sh` with automatic discovery from the HF bucket: no arguments lists every composable week with its composing files; a single week-number argument downloads and builds that week.

**Architecture:** Single pure-bash script. It lists both bucket folders (`full/`, `incremental/`) with `uvx hf buckets list -q`, converts each filename timestamp to "datehours" (floored hours since the 2025-01-01T00:00Z epoch), and groups snapshots into weeks with `week = datehours / 168` (same formula as wimage's `DateHours::week()`). Week N = base (latest `full_*.db` before `168·N`) + every `inc_*.db` inside `[168·N, 168·(N+1))`. Build mode runs the existing pipeline sequence: `makebase` (default `--datehours 0`) → `merge -t 0` → per-inc `ingest`, downloading and deleting one file at a time.

**Tech Stack:** Bash (GNU coreutils `date`), `uvx hf buckets` (already used by the current script), `./target/release/wpda-pipeline` (existing binary).

**Spec:** `docs/superpowers/specs/2026-09-01-makeweek-auto-list-design.md`

## Global Constraints

- Pure bash; no new dependencies beyond the `uvx hf` calls already used.
- Epoch constant: `EPOCH_SECONDS=1735689600` (= 2025-01-01T00:00:00Z). `datehours = (epoch − EPOCH_SECONDS) / 3600` floored; `week = datehours / 168` floored.
- Week N composition: base = latest `full_*.db` with `datehours < 168·N` (may be none); incs = every `inc_*.db` with `168·N ≤ datehours < 168·(N+1)`, ascending.
- `makebase` must keep the default `--datehours 0`: makebase then inserts no `versions` row, so base + 7 incs = 7 versions and `wpda-pipeline validate`'s ">7 versions" limit holds.
- Acceptance mapping (verified against live bucket data): week 83 = base `full_2026-08-04T01-36-27Z.db` + incs `2026-08-05` … `2026-08-11` (7 files).
- Defaults preserved and env-overridable: `DOWNLOADFOLDER` (default `$HOME/Téléchargements`), `WORKDIR` (default `/run/media/system/DataBtrfs/wplace/ramfs/<N>`).
- `set -euo pipefail`; sequential `hf buckets cp` → pipeline → `rm` download pattern (one file on disk at a time).
- Bucket: `hf://buckets/Hugi-R/wplace-archives`, folders `full/` and `incremental/`; listing lines look like `full/full_2026-08-04T01-36-27Z.db`.
- List and build must be verified against live bucket data; expected outputs in this plan were produced from the real bucket on 2026-09-01 (89 snapshots, weeks 74–86). If the bucket gains snapshots (e.g. week 86's 7th inc or week 87), counts change accordingly — recompute expectations from `./makeweek.sh` output, not from stale assumptions.

---

### Task 1: makeweek.sh lists weeks from the bucket

**Files:**
- Modify: `makeweek.sh` (full rewrite; currently a hardcoded one-week build script)

**Interfaces:**
- Consumes: `uvx hf buckets list <bucket-folder> -q` (network), GNU `date`.
- Produces: `snapshot_datehours <filename>` (prints datehours), `fetch_listings` (fills global arrays `FULL_FILES`/`FULL_DH`/`INC_FILES`/`INC_DH`), `select_week <w>` (sets `WEEK_BASE`, array `WEEK_INC_FILES`), `list_weeks`, `usage`, `main`. Task 2 reuses all of these unchanged.

- [ ] **Step 1: Write the new makeweek.sh (list mode only)**

Replace the entire content of `makeweek.sh` with:

```bash
#!/bin/bash
#
# List the weekly archives composable from the HF bucket.
#
# Usage:
#   makeweek.sh    List the weeks composable from the HF bucket.
#
# A week N is composed of the latest full_*.db taken before the week starts
# (the base) plus every inc_*.db inside the week. Weeks run Wednesday 00:00Z
# to Wednesday 00:00Z (DateHours::week() = hours since 2025-01-01T00:00Z / 168).

set -euo pipefail

BUCKET="hf://buckets/Hugi-R/wplace-archives"
EPOCH_SECONDS=1735689600 # 2025-01-01T00:00:00Z, the DateHours epoch

FULL_FILES=() FULL_DH=() # full/ snapshots and their datehours
INC_FILES=() INC_DH=()   # incremental/ snapshots and their datehours
WEEK_BASE=""             # set by select_week: base full_*.db of the week
WEEK_INC_FILES=()        # set by select_week: inc_*.db of the week

usage() {
    cat >&2 <<'EOF'
Usage: makeweek.sh          list the weeks composable from the HF bucket
EOF
}

# snapshot_datehours <filename>
# Print the datehours (hours since 2025-01-01T00:00:00Z, floored) encoded in a
# snapshot filename like "full_2026-08-04T01-36-27Z.db".
snapshot_datehours() {
    local ts="${1#*_}" # strip "full_" or "inc_"
    ts="${ts%.db}"
    local d="${ts%%T*}" # 2026-08-04
    local t="${ts#*T}"  # 01-36-27Z
    t="${t%Z}"          # 01-36-27
    t="${t//-/:}"       # 01:36:27
    local epoch
    epoch=$(date -u -d "$d $t" +%s)
    echo $(( (epoch - EPOCH_SECONDS) / 3600 ))
}

# fetch_listings
# Fill FULL_FILES/FULL_DH and INC_FILES/INC_DH from the bucket listing.
fetch_listings() {
    local tmp_full tmp_inc
    tmp_full=$(mktemp)
    tmp_inc=$(mktemp)
    trap "rm -f '$tmp_full' '$tmp_inc'" EXIT

    uvx hf buckets list "$BUCKET/full" -q > "$tmp_full"
    uvx hf buckets list "$BUCKET/incremental" -q > "$tmp_inc"

    local f
    while IFS= read -r f; do
        [[ -n "$f" ]] || continue
        f="${f#full/}"
        FULL_FILES+=("$f")
        FULL_DH+=("$(snapshot_datehours "$f")")
    done < <(sort -u "$tmp_full")

    while IFS= read -r f; do
        [[ -n "$f" ]] || continue
        f="${f#incremental/}"
        INC_FILES+=("$f")
        INC_DH+=("$(snapshot_datehours "$f")")
    done < <(sort -u "$tmp_inc")

    if (( ${#FULL_FILES[@]} == 0 || ${#INC_FILES[@]} == 0 )); then
        echo "error: empty bucket listing from $BUCKET" >&2
        exit 1
    fi
}

# select_week <week>
# Set WEEK_BASE to the latest full_*.db taken before the week starts (empty if
# none) and WEEK_INC_FILES to every inc_*.db inside the week, in order.
# Week <w> covers datehours [168*w, 168*(w+1)), same as DateHours::week().
select_week() {
    local w="$1"
    local start=$(( w * 168 ))
    local end=$(( (w + 1) * 168 ))
    local i dh best_dh=-1

    WEEK_BASE=""
    for i in "${!FULL_FILES[@]}"; do
        dh=${FULL_DH[i]}
        if (( dh < start && dh > best_dh )); then
            WEEK_BASE=${FULL_FILES[i]}
            best_dh=$dh
        fi
    done

    WEEK_INC_FILES=()
    for i in "${!INC_FILES[@]}"; do
        dh=${INC_DH[i]}
        if (( dh >= start && dh < end )); then
            WEEK_INC_FILES+=("${INC_FILES[i]}")
        fi
    done
}

# list_weeks
# Print one block per week (ascending) that has at least one snapshot, with
# its base and every composing inc file.
list_weeks() {
    local -A seen=()
    local w i
    for i in "${!INC_FILES[@]}"; do
        seen[$(( INC_DH[i] / 168 ))]=1
    done

    local -a weeks=()
    mapfile -t weeks < <(printf '%s\n' "${!seen[@]}" | sort -n)

    for w in "${weeks[@]}"; do
        select_week "$w"
        local start_s=$(( EPOCH_SECONDS + w * 168 * 3600 ))
        local end_s=$(( EPOCH_SECONDS + (w + 1) * 168 * 3600 ))
        local range
        range="$(date -u -d "@${start_s}" +%Y-%m-%d) -> $(date -u -d "@${end_s}" +%Y-%m-%d)"
        local status
        if [[ -z "$WEEK_BASE" ]]; then
            status="[no base]"
        elif (( ${#WEEK_INC_FILES[@]} == 7 )); then
            status="[complete]"
        else
            status="[incomplete: ${#WEEK_INC_FILES[@]}/7 incs]"
        fi
        printf 'week %-3d %s   %s\n' "$w" "$range" "$status"
        if [[ -n "$WEEK_BASE" ]]; then
            printf '  base: %s\n' "$WEEK_BASE"
        else
            printf '  base: (none)\n'
        fi
        if (( ${#WEEK_INC_FILES[@]} > 0 )); then
            local inc
            for inc in "${WEEK_INC_FILES[@]}"; do
                if [[ "$inc" == "${WEEK_INC_FILES[0]}" ]]; then
                    printf '  incs: %s\n' "$inc"
                else
                    printf '        %s\n' "$inc"
                fi
            done
        else
            printf '  incs: (none)\n'
        fi
    done
}

main() {
    if (( $# == 0 )); then
        fetch_listings
        list_weeks
    else
        usage
        exit 2
    fi
}

main "$@"
```

- [ ] **Step 2: Syntax check and executable bit**

Run: `chmod +x makeweek.sh && bash -n makeweek.sh && echo OK`
Expected: `OK` (no syntax errors)

- [ ] **Step 3: Verify list output against live bucket**

Run: `./makeweek.sh; echo "EXIT=$?"`
Expected: exit 0, exactly 13 `week` blocks (74–86), 11 `[complete]`, and these three blocks printed verbatim (the others follow the same pattern; week 83 is the spec's acceptance check):

```
week 74  2026-06-03 -> 2026-06-10   [no base]
  base: (none)
  incs: inc_2026-06-03T22-11-00Z.db
        inc_2026-06-04T17-32-25Z.db
        inc_2026-06-06T08-50-35Z.db
        inc_2026-06-07T00-51-15Z.db
        inc_2026-06-08T00-52-23Z.db
        inc_2026-06-09T00-53-54Z.db
```

```
week 83  2026-08-05 -> 2026-08-12   [complete]
  base: full_2026-08-04T01-36-27Z.db
  incs: inc_2026-08-05T01-35-33Z.db
        inc_2026-08-06T01-36-50Z.db
        inc_2026-08-07T01-42-25Z.db
        inc_2026-08-08T01-36-49Z.db
        inc_2026-08-09T01-40-00Z.db
        inc_2026-08-10T01-38-45Z.db
        inc_2026-08-11T01-40-12Z.db
```

```
week 86  2026-08-26 -> 2026-09-02   [incomplete: 6/7 incs]
  base: full_2026-08-25T01-50-32Z.db
  incs: inc_2026-08-26T01-52-24Z.db
        inc_2026-08-27T01-51-02Z.db
        inc_2026-08-28T01-53-46Z.db
        inc_2026-08-29T01-55-33Z.db
        inc_2026-08-30T01-56-47Z.db
        inc_2026-08-31T01-55-56Z.db
```

Structural checks (run each, expected value shown):

- `./makeweek.sh | grep -c '^week '` → `13`
- `./makeweek.sh | grep -c '\[complete\]'` → `11`
- `./makeweek.sh | grep -c '^  base: full_'` → `12`

(If the bucket gained new snapshots since 2026-09-01, recompute these three numbers from the new output before proceeding — the block contents above may legitimately differ for the newest week.)

- [ ] **Step 4: Commit**

```bash
git add makeweek.sh
git commit -m "feat: makeweek.sh lists weeks composable from the HF bucket"
```

---

### Task 2: makeweek.sh builds a week from its number

**Files:**
- Modify: `makeweek.sh` (header comment, constants, `usage`, new `build_week`, `main`)

**Interfaces:**
- Consumes: everything from Task 1 (`select_week`, `WEEK_BASE`, `WEEK_INC_FILES`, `snapshot_datehours`, `fetch_listings`).
- Produces: `build_week <week>` — runs the full pipeline; final archive lands in `$WORKDIR` as `w<N>_<last inc datehours>.db` (renamed by `ingest`).

- [ ] **Step 1: Update the header comment**

Replace:

```bash
#
# List the weekly archives composable from the HF bucket.
#
# Usage:
#   makeweek.sh    List the weeks composable from the HF bucket.
#
```

with:

```bash
#
# Build a weekly archive DB for the tileserver from the HF bucket archives.
#
# Usage:
#   makeweek.sh          List the weeks composable from the HF bucket.
#   makeweek.sh <week>   Download the files composing <week> and build it.
#
```

- [ ] **Step 2: Add the build constants**

Replace:

```bash
BUCKET="hf://buckets/Hugi-R/wplace-archives"
EPOCH_SECONDS=1735689600 # 2025-01-01T00:00:00Z, the DateHours epoch
```

with:

```bash
BUCKET="hf://buckets/Hugi-R/wplace-archives"
PIPELINE="./target/release/wpda-pipeline"
EPOCH_SECONDS=1735689600 # 2025-01-01T00:00:00Z, the DateHours epoch
DOWNLOADFOLDER="${DOWNLOADFOLDER:-$HOME/Téléchargements}"
```

- [ ] **Step 3: Extend usage**

Replace:

```bash
usage() {
    cat >&2 <<'EOF'
Usage: makeweek.sh          list the weeks composable from the HF bucket
EOF
}
```

with:

```bash
usage() {
    cat >&2 <<'EOF'
Usage: makeweek.sh          list the weeks composable from the HF bucket
       makeweek.sh <week>   download and build the given week
EOF
}
```

- [ ] **Step 4: Add build_week (insert between list_weeks and main)**

```bash
# build_week <week>
# Download the files composing <week> and run the pipeline on them.
build_week() {
    local n=$(( 10#$1 ))

    select_week "$n"
    if [[ -z "$WEEK_BASE" ]]; then
        echo "error: week $n has no base snapshot in the bucket" >&2
        exit 1
    fi
    if (( ${#WEEK_INC_FILES[@]} == 0 )); then
        echo "error: week $n has no incremental snapshots in the bucket" >&2
        exit 1
    fi
    if (( ${#WEEK_INC_FILES[@]} < 7 )); then
        echo "warning: week $n is incomplete: ${#WEEK_INC_FILES[@]}/7 incremental snapshots" >&2
    fi

    if [[ ! -x "$PIPELINE" ]]; then
        echo "error: $PIPELINE not found. Build it first: cargo build --release -p wpda-pipeline" >&2
        exit 1
    fi

    local workdir="${WORKDIR:-/run/media/system/DataBtrfs/wplace/ramfs/${n}}" # ramfs for speed, need ~20GB
    mkdir -p "$workdir"
    if compgen -G "$workdir/w${n}_*.db" > /dev/null; then
        echo "error: archives for week $n already exist in $workdir; remove them first" >&2
        exit 1
    fi

    echo "Working in $workdir"
    echo "Making week $n from base $WEEK_BASE"
    uvx hf buckets cp "$BUCKET/full/$WEEK_BASE" "$DOWNLOADFOLDER"
    "$PIPELINE" makebase --base "$DOWNLOADFOLDER/$WEEK_BASE" --output "$workdir/w${n}_0.db"
    "$PIPELINE" merge -t 0 "$workdir/w${n}_0.db"
    rm "$DOWNLOADFOLDER/$WEEK_BASE"

    local inc
    for inc in "${WEEK_INC_FILES[@]}"; do
        echo "Ingesting incremental $inc"
        uvx hf buckets cp "$BUCKET/incremental/$inc" "$DOWNLOADFOLDER"
        "$PIPELINE" ingest --archives "$workdir" --increment "$DOWNLOADFOLDER/$inc"
        rm "$DOWNLOADFOLDER/$inc"
    done

    printf 'Result: %s\n' "$workdir"/w"$n"_*.db
    echo "Done"
}
```

Notes:
- `10#$1` forces base-10 so `makeweek.sh 083` does not trip bash's octal interpretation.
- Selection resolves before the `$PIPELINE` check (per spec) so argument/selection errors are deterministic before anything environment-dependent.
- The stale-archive abort guards against a leftover `w<N>_*.db` that `find_latest_archive` (inside `ingest`) would otherwise pick up.

- [ ] **Step 5: Replace main**

Replace:

```bash
main() {
    if (( $# == 0 )); then
        fetch_listings
        list_weeks
    else
        usage
        exit 2
    fi
}
```

with:

```bash
main() {
    if (( $# == 0 )); then
        fetch_listings
        list_weeks
    elif (( $# == 1 )); then
        if ! [[ "$1" =~ ^[0-9]+$ ]]; then
            usage
            exit 2
        fi
        fetch_listings
        build_week "$1"
    else
        usage
        exit 2
    fi
}
```

- [ ] **Step 6: Syntax check**

Run: `bash -n makeweek.sh && echo OK`
Expected: `OK`

- [ ] **Step 7: Regression — list mode unchanged**

Run: `./makeweek.sh | grep -c '^week '`
Expected: `13` (recompute if the bucket changed since Task 1, same rule as Task 1 Step 3)

- [ ] **Step 8: Argument validation (no downloads happen)**

Run: `./makeweek.sh x 2>&1; echo "EXIT=$?"`
Expected:

```
Usage: makeweek.sh          list the weeks composable from the HF bucket
       makeweek.sh <week>   download and build the given week
EXIT=2
```

Run: `./makeweek.sh 1 2 2>&1; echo "EXIT=$?"`
Expected: the same usage block, `EXIT=2`

- [ ] **Step 9: Selection error paths (exit before any download)**

Run: `./makeweek.sh 74 2>&1; echo "EXIT=$?"`
Expected (week 74 has incs but nothing before it in the bucket):

```
error: week 74 has no base snapshot in the bucket
EXIT=1
```

Run: `./makeweek.sh 99 2>&1; echo "EXIT=$?"`
Expected (a future week resolves the latest full as base but has no incs):

```
error: week 99 has no incremental snapshots in the bucket
EXIT=1
```

- [ ] **Step 10: Commit**

```bash
git add makeweek.sh
git commit -m "feat: makeweek.sh builds a week from its number"
```

---

### Task 3: README touch-up + real acceptance build (user-run)

**Files:**
- Modify: `README.md` (one line in "Building the archives")

**Interfaces:**
- Consumes: the completed `makeweek.sh` from Task 2.

- [ ] **Step 1: Update the makeweek.sh mention in README**

In `README.md`, replace the line:

```markdown
`makeweek.sh` automates this full base + incremental workflow for a week number.
```

with:

```markdown
`makeweek.sh` automates this workflow: run it without arguments to list the
weeks composable from the bucket, or `makeweek.sh <week>` to download its
files and build it.
```

- [ ] **Step 2: Commit**

```bash
git add README.md
git commit -m "docs: makeweek.sh bucket-driven usage in README"
```

- [ ] **Step 3: Real acceptance build of week 83 (LONG: hours, ~20GB ramfs — run interactively, not from an agent)**

Run:

```bash
WORKDIR=/run/media/system/DataBtrfs/wplace/ramfs/83-test ./makeweek.sh 83
```

Expected flow, in order:

1. `warning:` line must NOT appear (week 83 is complete, 7/7 incs).
2. `Working in /run/media/system/DataBtrfs/wplace/ramfs/83-test`
3. `Making week 83 from base full_2026-08-04T01-36-27Z.db`
4. Seven `Ingesting incremental inc_2026-08-05…` through `inc_2026-08-11…` lines (including `inc_2026-08-05T01-35-33Z.db`, the file the old script missed).
5. `Result: /run/media/system/DataBtrfs/wplace/ramfs/83-test/w83_14089.db` (ingest renames the archive to the last inc's datehours; 14089 = 2026-08-11T01:40).
6. `Done`

Then verify the versions table (7 rows at the inc datehours, no row for the base):

Run: `sqlite3 /run/media/system/DataBtrfs/wplace/ramfs/83-test/w83_14089.db "SELECT date FROM versions ORDER BY date"`
Expected:

```
13945
13969
13993
14017
14041
14065
14089
```

Optional but recommended (long): `./target/release/wpda-pipeline validate /run/media/system/DataBtrfs/wplace/ramfs/83-test/w83_14089.db` — expected: completes without the ">7 versions" error.

Cleanup (frees the ramfs): `rm -rf /run/media/system/DataBtrfs/wplace/ramfs/83-test`

- [ ] **Step 4: Report**

Report acceptance results to the user; do not commit any DB artifacts (`.gitignore` already excludes the ramfs path; nothing in this task should touch the repo besides the README commit from Step 2).
