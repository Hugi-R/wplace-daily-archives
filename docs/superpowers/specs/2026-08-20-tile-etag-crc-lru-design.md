# ETag Revalidation via In-Memory Crc LRU

Date: 2026-08-20

## Goal

Remove the full-blob SQLite read that ETag revalidation currently performs in
`tileserver/src/main.rs`. Today a `304 Not Modified` for `/tiles/...` or
`/diff/all/...` still fetches the entire tile blob (avg ~43KB, SQLite
overflow pages) just to recompute the crc32 ETag. Replace that with a bounded
in-memory LRU of `(coordinates+dates) -> crc32` so hot-tile revalidation is a
pure memory lookup. Also tighten the coordinate types, which currently travel
the whole handler stack as `i64`.

## Current state

- `parse_tile_coords` returns `(i64, i64, i64)` (main.rs:563), validated to
  `z∈[0,11]`, `0≤x,y<2^z`. At max z=11 tiles are 2048×2048, so x,y∈[0,2047].
- `get_tile(&self, z: i64, x: i64, y: i64, version: u32)` (main.rs:143) reads
  `SELECT data FROM tiles WHERE z=? AND x=? AND y=?` (TILE_QUERY, main.rs:41).
- `get_all_diffs(&self, z: i64, x: i64, y: i64, from: u32, to: u32)` (main.rs:198)
  concatenates all week blobs in [from,to] for a tile.
- `serve_tile` (main.rs:625) / `serve_all_diff` (main.rs:~676) extract
  `If-None-Match`, then DB-read the full body, then `cached_response`
  (main.rs:601) crc32s the body and returns 304 or 200.
- Week DBs are opened `SQLITE_OPEN_READ_ONLY` at startup and never reloaded
  mid-process; the `DatabaseManager` holds `HashMap<version, SqlitePool>` for
  the life of the process. Tile blobs are therefore immutable while the
  server runs.

## Design

### 1. Tighten coordinate types

- `parse_tile_coords` returns `Result<(u8, u16, u16), &'static str>`.
  `z` parses to `u8`; `x`,`y` parse to `u16`. Validation unchanged
  (`z<=11`, `x,y < 1<<z` — cast `1u16 << z`).
- Ripple (all in `tileserver/src/main.rs`):
  - `get_tile(self, z: u8, x: u16, y: u16, version: u32)`
  - `get_all_diffs(self, z: u8, x: u16, y: u16, from: u32, to: u32)`
  - call sites in `serve_tile` and `serve_all_diff` (they receive u8/u16
    straight from the parser; `spawn_blocking` closures just pass them on).
- `rusqlite` binds u8/u16 natively via `ToSql`; `TILE_QUERY` string unchanged.
- Test helpers: the `for z in [9i64, 0i64]` loop (main.rs:1046-1047) becomes
  `[9u8, 0u8]`. `create_week_db`'s own coordinate params narrow to the same
  types (u8/u16 bind natively to SQLite INTEGER; pure SQL-side insertion,
  behavior unchanged) so the loop's `z: u8` passes through without a cast.

### 2. CrcCache component

```rust
#[derive(Hash, PartialEq, Eq)]
struct TileCrcKey { version: u32, z: u8, x: u16, y: u16 }

#[derive(Hash, PartialEq, Eq)]
struct DiffCrcKey { z: u8, x: u16, y: u16, from: u32, to: u32 }

struct CrcCache<K> {
    inner: std::sync::Mutex<lru::LruCache<K, u32>>,
}
```

Concurrency note: a single global `Mutex` serializes the (microsecond-scale)
cache lookup/insert only — DB reads and network I/O stay fully concurrent.
The lock's cost is real but small: `lru::LruCache::get` needs `&mut self`
(promotion mutates the recency list), so even read-heavy revalidations ping
the same cache line. Sharded mutexes or a concurrent cache (`moka`) would
reduce contention, but for this server's scale the simple lock wins on
maintainability; a comment on `CrcCache` documents the known drawback and
deliberate choice to stay simple.

- Dependency: add `lru = "0.16"` to `tileserver/Cargo.toml` (not currently in
  the lockfile; small crate, no native deps).
- `get(&self, key: &K) -> Option<u32>` and
  `insert(&self, key: K, crc: u32)`.
- Bound: `lru::LruCache::new(NonZeroUsize::new(500_000))` per cache instance
  (~25MB worst case across both, typical far lower).
- Two instances on `TileServer`: `tile_crcs: CrcCache<TileCrcKey>` and
  `diff_crcs: CrcCache<DiffCrcKey>`. `TileServer` is already shared via
  `Arc<TileServer>`, so the caches live directly on the struct.
- Correctness by construction: crcs are only stored from real computed bodies;
  week blobs are immutable for the process lifetime, so a stored crc can never
  go stale; a restart empties the cache. No invalidation needed.

### 3. Handler integration

Both handlers gain a fast path **before** the DB read:

1. Parse coords (now u8/u16).
2. Build the cache key.
3. `if let Some(crc) = cache.get(&key)` → compute `etag = format!("\"{:08x}\"", crc)`;
   if it equals the request's `If-None-Match` → return `304` immediately with
   the standard headers, **no DB hit**.
4. Otherwise: existing flow (spawn_blocking DB read → `cached_response`). On
   the `200` branch, after/while computing crc, `cache.insert(key, crc)`.

`cached_response` itself is unchanged; it still computes crc from data on the
compute path and emits 304/200 as today. The LRU is only an earlier short-cut
in the handlers.

Both handlers already extract `If-None-Match` before the DB read; the fast
path reuses that value.

To keep the 304 headers identical to the 200 headers, the fast path builds
the same header set (`Content-Type`, `Cache-Control: public, max-age=3600`,
`ETag`) that `cached_response` emits — factored into the existing
`cached_response` via a small helper or inline header tuple, keeping them in
sync.

## Concerns / tradeoffs

1. A cache miss costs the same as today (full blob read). Hit rate on hot
   tiles is the win; cold/many-version tiles still pay the read.
2. Two 500K-entry LRUs cap memory ~25MB. If memory is tighter, a single
   shared instance with an enum key is the fallback.
3. The `If-None-Match` value travels as `Option<&str>` borrowed from the
   request headers; the fast path must hold it without fighting the borrow
   checker (clone the header value if needed before the async/sync boundary).
4. This only optimizes the server's own validation path. CDN/browser caches
   still use the HTTP semantics as before.

## Testing

- Unit test the `CrcCache` (insert/get, eviction of LRU tail beyond bound,
  no-panic under concurrent get/insert).
- Handler tests (extend `mod tests`, router `oneshot`):
  - `/tiles/...`: first request populates cache → second request with matching
    `If-None-Match` returns `304`.
  - `/diff/all/...`: same round trip → `304` on echoed etag.
  - 200 path still returns body + etag (regression).
  - Prove the fast path avoids the DB: after a first 200 (cache populated),
    delete the row from the DB file and re-request with echoing `If-None-Match`
    — a `304` proves no DB read (the row is gone). If the fast path queried
    the DB it would 404.
- u8/u16 ripple covered by the existing suite still compiling/passing.

## Out of scope

- Adding a `crc` column to the SQLite `tiles` table (pipeline schema +
  rebuild) — explicitly deferred; LRU chosen over it.
- Streaming `get_all_diffs`; `make_latest_image`; bitmap-instead-of-PNG.
- Persisting the crc cache across restarts.