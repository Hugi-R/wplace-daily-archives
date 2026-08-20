# Tile Caching: TTL 1h and content-crc32 ETags

Date: 2026-08-20

## Goal

Review and update how tiles are cached across `frontend/index.html` and
`tileserver/src/main.rs`:

- TTL for cached tile content becomes 1 hour.
- ETags become content-based, computed with crc32.

## Current state

### tileserver/src/main.rs

- `serve_tile` (`/tiles/{version}/{z}/{x}/{y}`):
  - `Cache-Control: public, max-age=86400` (1 day).
  - ETag = `"{version}-{z}/{x}/{y}"` — derived from coordinates, not content,
    so it does not change when the underlying tile blob changes.
- `serve_all_diff` (`/diff/all/{z}/{x}/{y}`):
  - `Cache-Control: public, max-age=3600` (already 1 hour).
  - ETag = `"alldiff-{z}/{x}/{y}-from{from}-to{to}"` — range-derived, not content.
- Index pages, `/assets/*`, `/preview.png`, `/favicon.ico` have no cache headers.

### frontend/index.html

- CacheStorage cache `merged-tiles`, keyed `tile-{version}-{z}-{x}-{y}`. No TTL
  (Cache API has no native expiry), no server ETag stored or revalidated.
- Download handlers re-read the same cache and alert on miss.

## Design

### tileserver/src/main.rs

Add a shared response helper used by all static/single-body responses:

```rust
fn cached_response(
    status: StatusCode,
    mime: &'static str,
    max_age: Duration,
    data: Bytes,
    if_none_match: Option<&str>,
) -> Response
```

- Computes `etag = format!("\"{:08x}\"", crc32fast::hash(&data))`.
- Sets `Content-Type`, `Cache-Control: public, max-age=3600`, and `ETag`.
- Returns `304` with headers when `if_none_match == etag`, otherwise
  `status` + headers + body.

Applied to:

- `serve_tile`: DB read -> cached_response; TTL 86400 -> 3600. Note: 304 now
  costs a DB read (etag requires the content) — accepted tradeoff.
- `serve_all_diff`: build body -> cached_response; TTL stays 3600.
- `serve_index_*`: HTML gets `max-age=3600` + content ETag. Safe because the
  index pages are only rebuilt at startup (new weeks require a restart).
- `serve_asset`: assets get `max-age=3600` + content ETag.
- `serve_preview` / `serve_favicon`: same — `max-age=3600` + content ETag.

`serve_tile` and `serve_all_diff` extract `If-None-Match` from the request
headers and pass it to the helper. Redirects and 4xx/5xx error responses stay
uncacheable.

### tileserver/Cargo.toml

Add `crc32fast = "1"` (already a transitive dep in the lockfile via `png`;
promote to direct).

### frontend/index.html

- Add `const TILE_CACHE_TTL_MS = 60 * 60 * 1000;`.
- On put: store a metadata entry `meta-tile-{version}-{z}-{x}-{y}` carrying the
  storage timestamp.
- On match: if metadata missing or age >= 1h -> `cache.delete` the tile entry
  and treat as a cache miss (refetch + re-decode).
- Download handlers unchanged: they already alert when the cache misses.

## Concerns / noted tradeoffs

1. Content ETags mean revalidation requires the DB read; accepted for this
   scale.
2. HTML and asset caching becomes correct-by-construction (single helper) and
   covered by the same TTL policy.
3. crc32 is not a cryptographic hash; fine for ETag change-detection but not a
   content-authenticity guarantee.

## Testing

- Extend/add `#[tokio::test]` handler tests (tower `oneshot`) to assert:
  - 200 responses carry `Cache-Control: public, max-age=3600`.
  - A matching `If-None-Match` returns `304` with the same headers.
  - A mismatched/absent `If-None-Match` returns `200` with the body.
- Frontend TTL is a small JS logic change; verify by code review and manual
  browser check.

## Out of scope

- Streamed diffs (existing follow-up note).
- `make_latest_image` TODO.
- Bitmap-instead-of-PNG transfer idea (README).