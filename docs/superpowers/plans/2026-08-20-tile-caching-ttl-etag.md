# Tile Caching TTL and ETag Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Unify tile caching so every cached resource uses a 1-hour `Cache-Control` TTL and a content-derived crc32 ETag, across the tileserver and the frontend CacheStorage.

**Architecture:** One Rust helper (`cached_response`) computes a crc32-of-body ETag, applies `public, max-age=3600`, and emits `304 Not Modified` when `If-None-Match` matches. Every byte-returning handler (tiles, all-diffs, index HTML, assets, preview, favicon) funnels through it. The frontend CacheStorage layer gains an explicit timestamp-based 1h expiry since the Cache API has no native TTL.

**Tech Stack:** Rust (axum 0.8, tower/`oneshot` tests), `crc32fast`, vanilla JS CacheStorage in `frontend/index.html`.

## Global Constraints

- All cached resources use `Cache-Control: public, max-age=3600`.
- ETag format is `"{:08x}"` (quoted 8-hex-digit) from `crc32fast::hash(body)`.
- Redirects and 4xx/5xx error responses must NOT pass through `cached_response` (stay uncacheable).
- Workspace root Cargo.toml; run tests with `cargo test -p wpda-tileserver` from the workspace root.
- Tests live inline in `tileserver/src/main.rs` inside `mod tests` (existing pattern).
- Commit style follows repo: `feat(tileserver): ...` / `feat(frontend): ...`.

---

### Task 1: `cached_response` helper with crc32 ETag + 1h Cache-Control

**Files:**
- Modify: `tileserver/Cargo.toml` (add `crc32fast`)
- Modify: `tileserver/src/main.rs` (add helper + unit tests; tests in `mod tests` at bottom of file)

**Interfaces:**
- Consumes: existing `etag_header(&str) -> (HeaderName, HeaderValue)` (main.rs:596).
- Produces: `fn cached_response(status: StatusCode, mime: &'static str, max_age: Duration, data: Bytes, if_none_match: Option<&str>) -> Response` — the single entry point Tasks 2-3 rewire handlers to.

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `tileserver/src/main.rs` (after the last existing `#[tokio::test]`, before the closing `}` of `mod tests`):

```rust
#[test]
fn cached_response_sets_1h_cache_control_and_content_etag() {
    let resp = cached_response(
        StatusCode::OK,
        "application/octet-stream",
        Duration::from_secs(3600),
        Bytes::from_static(b"hello"),
        None,
    );
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers().get(header::CACHE_CONTROL).unwrap(),
        "public, max-age=3600"
    );
    assert_eq!(
        resp.headers().get(header::ETAG).unwrap(),
        &format!("\"{:08x}\"", crc32fast::hash(b"hello"))
    );
}

#[test]
fn cached_response_matching_if_none_match_returns_304() {
    let body = Bytes::from_static(b"hello");
    let etag = format!("\"{:08x}\"", crc32fast::hash(&body));
    let resp = cached_response(
        StatusCode::OK,
        "application/octet-stream",
        Duration::from_secs(3600),
        body,
        Some(&etag),
    );
    assert_eq!(resp.status(), StatusCode::NOT_MODIFIED);
    assert_eq!(resp.headers().get(header::ETAG).unwrap(), &etag);
}

#[test]
fn cached_response_mismatched_if_none_match_returns_full_body() {
    let resp = cached_response(
        StatusCode::OK,
        "application/octet-stream",
        Duration::from_secs(3600),
        Bytes::from_static(b"hello"),
        Some("\"deadbeef\""),
    );
    assert_eq!(resp.status(), StatusCode::OK);
}

#[test]
fn crc32fast_matches_standard_check_vector() {
    assert_eq!(crc32fast::hash(b"123456789"), 0xCBF4_3926);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p wpda-tileserver cached_response` and `cargo test -p wpda-tileserver crc32fast_matches`
Expected: FAIL — `cannot find function \`cached_response\` in this scope` and `cannot find function \`crc32fast\``.

- [ ] **Step 3: Add the dependency**

In `tileserver/Cargo.toml` under `[dependencies]`, add after the `chrono` line:

```toml
crc32fast = "1"
```

- [ ] **Step 4: Implement the helper**

In `tileserver/src/main.rs`, immediately after the existing `if_none_match` function (main.rs:609), add:

```rust
/// Builds a response with a content-crc32 ETag and a 1h-friendly Cache-Control.
/// Returns `304 Not Modified` when `if_none_match` equals the computed ETag.
fn cached_response(
    status: StatusCode,
    mime: &'static str,
    max_age: Duration,
    data: Bytes,
    if_none_match: Option<&str>,
) -> Response {
    let etag = format!("\"{:08x}\"", crc32fast::hash(&data));
    let out_headers = [
        (header::CONTENT_TYPE, HeaderValue::from_static(mime)),
        (
            header::CACHE_CONTROL,
            HeaderValue::from_str(&format!("public, max-age={}", max_age.as_secs()))
                .expect("valid cache-control header"),
        ),
        etag_header(&etag),
    ];
    if if_none_match == Some(etag.as_str()) {
        return (StatusCode::NOT_MODIFIED, out_headers).into_response();
    }
    (status, out_headers, data).into_response()
}
```

- [ ] **Step 5: Run all tests to verify they pass**

Run: `cargo test -p wpda-tileserver`
Expected: PASS, including the four new tests.

- [ ] **Step 6: Commit**

```bash
git add tileserver/Cargo.toml tileserver/src/main.rs
git commit -m "feat(tileserver): content crc32 etag and 1h cache-control helper"
```

---

### Task 2: Rewire tile and all-diff endpoints through `cached_response`

**Files:**
- Modify: `tileserver/src/main.rs` — `serve_tile` (main.rs:612), `serve_all_diff` (main.rs:676), delete `if_none_match` (main.rs:603)
- Test: `mod tests` in `tileserver/src/main.rs`

**Interfaces:**
- Consumes: `cached_response` from Task 1.
- Produces: tile & diff endpoints with `Cache-Control: public, max-age=3600` and crc32 content ETags; 304 when `If-None-Match` matches.

- [ ] **Step 1: Write the failing handler tests**

Add to `mod tests` in `tileserver/src/main.rs`:

```rust
#[tokio::test]
async fn tile_endpoint_has_1h_cache_control_and_content_etag() {
    let tmp = router_fixture();
    let ts = Arc::new(TileServer::new(tmp.path().to_path_buf()).unwrap());
    let app = build_router(ts);
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/tiles/0/9/0/0.zst")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers().get("cache-control").unwrap(),
        "public, max-age=3600"
    );
    assert_eq!(
        resp.headers().get("etag").unwrap(),
        &format!("\"{:08x}\"", crc32fast::hash(&entry(0, &[0xAA])))
    );
}

#[tokio::test]
async fn tile_endpoint_returns_304_on_matching_if_none_match() {
    let tmp = router_fixture();
    let ts = Arc::new(TileServer::new(tmp.path().to_path_buf()).unwrap());
    let app = build_router(ts);
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/tiles/0/9/0/0.zst")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let etag = resp.headers().get("etag").unwrap().clone();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/tiles/0/9/0/0.zst")
                .header("if-none-match", &etag)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_MODIFIED);
    assert_eq!(resp.headers().get("etag").unwrap(), &etag);
}

#[tokio::test]
async fn diff_endpoint_returns_304_and_1h_cache_control() {
    let tmp = router_fixture();
    let ts = Arc::new(TileServer::new(tmp.path().to_path_buf()).unwrap());
    let app = build_router(ts);
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/diff/all/9/0/0.zst?from=0&to=4294967295")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers().get("cache-control").unwrap(),
        "public, max-age=3600"
    );
    let etag = resp.headers().get("etag").unwrap().clone();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/diff/all/9/0/0.zst?from=0&to=4294967295")
                .header("if-none-match", &etag)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_MODIFIED);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p wpda-tileserver tile_endpoint` and `cargo test -p wpda-tileserver diff_endpoint_returns`
Expected: FAIL — tile test asserts `max-age=3600` but the endpoint still sends `max-age=86400`; the 304 tests fail because no `If-None-Match` handling exists for content ETags.

- [ ] **Step 3: Rewire `serve_tile`**

Replace the body of `serve_tile` in `tileserver/src/main.rs` (the section from `let etag = format!("\"{}-{}\"", version, tile_key(z, x, y));` through the end of the function, main.rs:632-667) with:

```rust
    let if_none_match = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok());

    let state = ts.clone();
    let result = tokio::task::spawn_blocking(move || state.db.get_tile(z, x, y, version)).await;

    match result {
        Ok(Ok(data)) => cached_response(
            StatusCode::OK,
            "application/octet-stream",
            Duration::from_secs(3600),
            Bytes::from(data),
            if_none_match,
        ),
        Ok(Err(TileError::TileNotFound)) => text_error(StatusCode::NOT_FOUND, "tile not found"),
        Ok(Err(TileError::VersionNotFound(v))) => {
            text_error(StatusCode::NOT_FOUND, &format!("version {v} not found"))
        }
        Ok(Err(e)) => {
            error!("Database query error: {e}");
            text_error(StatusCode::INTERNAL_SERVER_ERROR, "Database error")
        }
        Err(e) => {
            error!("Blocking task failed: {e}");
            text_error(StatusCode::INTERNAL_SERVER_ERROR, "Internal error")
        }
    }
```

- [ ] **Step 4: Rewire `serve_all_diff`**

Replace this section of `serve_all_diff` in `tileserver/src/main.rs` (from `let etag = format!("\"alldiff-...` at main.rs:713 through the end of the function at main.rs:742) with:

```rust
    let if_none_match = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok());

    let state = ts.clone();
    match tokio::task::spawn_blocking(move || state.db.get_all_diffs(z, x, y, from, to)).await {
        // An empty diff means either the tile does not exist at this z (z=10 is
        // intentionally not stored, and /diff/all is open to any z) or no frame in
        // the requested range changed this tile; both are "nothing to render".
        Ok(body) if body.is_empty() => text_error(StatusCode::NOT_FOUND, "diff not found"),
        Ok(body) => cached_response(
            StatusCode::OK,
            "application/octet-stream",
            Duration::from_secs(3600),
            Bytes::from(body),
            if_none_match,
        ),
        Err(e) => {
            error!("Blocking task failed: {e}");
            text_error(StatusCode::INTERNAL_SERVER_ERROR, "Internal error")
        }
    }
```

- [ ] **Step 5: Delete the now-unused `if_none_match` function**

Remove the function `fn if_none_match(headers: &HeaderMap, etag: &str) -> bool` (main.rs:603-609). Keep `etag_header`.

- [ ] **Step 6: Run all tests to verify they pass**

Run: `cargo test -p wpda-tileserver`
Expected: PASS — new handler tests pass and all pre-existing tests (`diff_endpoint_serves_lower_zoom_levels`, `diff_endpoint_unstored_z_is_404`, etc.) still pass.

- [ ] **Step 7: Commit**

```bash
git add tileserver/src/main.rs
git commit -m "feat(tileserver): 1h ttl and crc32 etags for tile and diff endpoints"
```

---

### Task 3: Rewire index, assets, preview, and favicon through `cached_response`

**Files:**
- Modify: `tileserver/src/main.rs` — `serve_index_en/ja/es`, `serve_index_lang` (main.rs:744-761), `serve_preview` (main.rs:800), `serve_favicon` (main.rs:809), `serve_asset` (main.rs:821)
- Test: `mod tests` in `tileserver/src/main.rs`

**Interfaces:**
- Consumes: `cached_response` from Task 1.
- Produces: cached HTML/asset/preview/favicon responses with the same 1h TTL + crc32 ETag policy.

- [ ] **Step 1: Write the failing handler tests**

Add to `mod tests` in `tileserver/src/main.rs`:

```rust
#[tokio::test]
async fn index_page_has_1h_cache_control_and_etag() {
    let tmp = router_fixture();
    let ts = Arc::new(TileServer::new(tmp.path().to_path_buf()).unwrap());
    let app = build_router(ts);
    let resp = app
        .oneshot(Request::builder().uri("/en/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers().get("cache-control").unwrap(),
        "public, max-age=3600"
    );
    assert!(resp.headers().contains_key("etag"));
    assert_eq!(
        resp.headers().get("content-type").unwrap(),
        "text/html; charset=utf-8"
    );
}

#[tokio::test]
async fn asset_has_1h_cache_control_and_revalidates() {
    let tmp = router_fixture();
    let body = "console.log('x')";
    std::fs::write(tmp.path().join("assets").join("app.js"), body).unwrap();
    let ts = Arc::new(TileServer::new(tmp.path().to_path_buf()).unwrap());
    let app = build_router(ts);
    let resp = app
        .clone()
        .oneshot(Request::builder().uri("/assets/app.js").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers().get("cache-control").unwrap(),
        "public, max-age=3600"
    );
    assert_eq!(
        resp.headers().get("etag").unwrap(),
        &format!("\"{:08x}\"", crc32fast::hash(body.as_bytes()))
    );
    let etag = resp.headers().get("etag").unwrap().clone();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/assets/app.js")
                .header("if-none-match", &etag)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_MODIFIED);
}

#[tokio::test]
async fn preview_and_favicon_have_1h_cache_control() {
    let tmp = router_fixture();
    let ts = Arc::new(TileServer::new(tmp.path().to_path_buf()).unwrap());
    let app = build_router(ts);
    for uri in ["/preview.png", "/favicon.ico"] {
        let resp = app
            .clone()
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get("cache-control").unwrap(),
            "public, max-age=3600",
            "{uri}"
        );
        assert!(resp.headers().contains_key("etag"), "{uri}");
    }
}
```

Note: in the test fixture `make_latest_image()` returns `Err`, so `preview_image` and `favicon` are empty `Bytes` — the status is still `200 OK`, which is what the test asserts.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p wpda-tileserver index_page_has` and `cargo test -p wpda-tileserver asset_has` and `cargo test -p wpda-tileserver preview_and`
Expected: FAIL — handlers send no `Cache-Control`/`ETag` for index/assets/preview/favicon today.

- [ ] **Step 3: Rewire `serve_index_*`**

Replace `serve_index_en`, `serve_index_ja`, `serve_index_es`, and `serve_index_lang` (main.rs:744-761) with:

```rust
async fn serve_index_en(State(ts): State<Arc<TileServer>>, headers: HeaderMap) -> Response {
    serve_index_lang(ts, Lang::En, headers)
}

async fn serve_index_ja(State(ts): State<Arc<TileServer>>, headers: HeaderMap) -> Response {
    serve_index_lang(ts, Lang::Ja, headers)
}

async fn serve_index_es(State(ts): State<Arc<TileServer>>, headers: HeaderMap) -> Response {
    serve_index_lang(ts, Lang::Es, headers)
}

fn serve_index_lang(ts: Arc<TileServer>, lang: Lang, headers: HeaderMap) -> Response {
    let if_none_match = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok());
    match ts.index_html.get(&lang) {
        Some(html) => cached_response(
            StatusCode::OK,
            "text/html; charset=utf-8",
            Duration::from_secs(3600),
            html.clone(),
            if_none_match,
        ),
        None => text_error(StatusCode::NOT_FOUND, "404 page not found"),
    }
}
```

- [ ] **Step 4: Rewire `serve_preview` and `serve_favicon`**

Replace `serve_preview` and `serve_favicon` (main.rs:800-819) with:

```rust
async fn serve_preview(State(ts): State<Arc<TileServer>>, headers: HeaderMap) -> Response {
    let if_none_match = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok());
    cached_response(
        StatusCode::OK,
        "image/png",
        Duration::from_secs(3600),
        ts.preview_image.clone(),
        if_none_match,
    )
}

async fn serve_favicon(State(ts): State<Arc<TileServer>>, headers: HeaderMap) -> Response {
    let if_none_match = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok());
    cached_response(
        StatusCode::OK,
        "image/x-icon",
        Duration::from_secs(3600),
        ts.favicon.clone(),
        if_none_match,
    )
}
```

- [ ] **Step 5: Rewire `serve_asset`**

Replace `serve_asset` (main.rs:821-837) with:

```rust
async fn serve_asset(
    State(ts): State<Arc<TileServer>>,
    AxumPath(filename): AxumPath<String>,
    headers: HeaderMap,
) -> Response {
    let if_none_match = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok());
    match ts.assets.get(&filename) {
        Some(asset) => cached_response(
            StatusCode::OK,
            asset.mime,
            Duration::from_secs(3600),
            asset.data.clone(),
            if_none_match,
        ),
        None => text_error(StatusCode::NOT_FOUND, "404 page not found"),
    }
}
```

- [ ] **Step 6: Run all tests to verify they pass**

Run: `cargo test -p wpda-tileserver`
Expected: PASS — new handler tests and all pre-existing tests (`lang_path_serves_localized_page`, `unknown_lang_path_is_404`, etc.) pass.

- [ ] **Step 7: Commit**

```bash
git add tileserver/src/main.rs
git commit -m "feat(tileserver): 1h ttl and crc32 etags for html, assets, preview, favicon"
```

---

### Task 4: Frontend CacheStorage 1h TTL

**Files:**
- Modify: `frontend/index.html` — `merged` protocol handler (index.html:710-740)

**Interfaces:**
- Consumes: the existing CacheStorage cache `"merged-tiles"` and `tile-{version}-{z}-{x}-{y}` keys.
- Produces: stale (>1h) cache entries are deleted and treated as misses; fresh entries still short-circuit `getTile`. Download handlers (index.html:896-910) keep working unchanged — they alert on a miss, which is the existing behavior.

- [ ] **Step 1: Add the TTL constant and expiry logic**

In `frontend/index.html`, replace the body of the `maplibregl.addProtocol('merged', ...)` callback (index.html:711-740) with:

```js
    maplibregl.addProtocol('merged', async (params, abortController) => {
      const { version, z, x, y } = parseTileUrl(params.url);

      // abort if z=10 as we don't store that level
      if (z === 10) {
        const err = new Error("z=10 tiles are not stored");
        err.status = 404;
        throw err;
      }

      const TILE_CACHE_TTL_MS = 60 * 60 * 1000; // 1 hour
      const cache = typeof caches !== 'undefined' ? await caches.open("merged-tiles") : null;
      const cacheKey = `tile-${version}-${z}-${x}-${y}`;
      const metaKey = `meta-${cacheKey}`;

      if (cache) {
        const [cachedResponse, metaResponse] = await Promise.all([
          cache.match(cacheKey),
          cache.match(metaKey)
        ]);
        if (cachedResponse && metaResponse) {
          const meta = await metaResponse.json();
          if (Date.now() - meta.timestamp < TILE_CACHE_TTL_MS) {
            const buffer = await cachedResponse.arrayBuffer();
            return {data: buffer};
          }
          await cache.delete(cacheKey);
          await cache.delete(metaKey);
        }
      }

      const img = await getTile(version, z, x, y, abortController.signal);
      if (cache) {
        // Cache the response for future requests
        const response = new Response(img);
        await cache.put(cacheKey, response);
        await cache.put(metaKey, new Response(JSON.stringify({ timestamp: Date.now() }), {
          headers: { 'Content-Type': 'application/json' }
        }));
      }
      return {data: img};
    });
```

- [ ] **Step 2: Verify the edit and run the server test suite**

Run: `cargo test -p wpda-tileserver`
Expected: PASS. The i18n/template render tests read `frontend/index.html` and only assert markers + `wasm_screenshot("/tiles",` + `wasm_video("/diff",` substrings, which are untouched; the change introduces no template markers, so rendering still succeeds.
Also manually grep to confirm no leftover old code path:

Run: `grep -n "cache.put\|cache.delete\|TILE_CACHE_TTL_MS" frontend/index.html`
Expected: the three new usages at the protocol handler; no other `cache.put` outside the handler.

- [ ] **Step 3: Commit**

```bash
git add frontend/index.html
git commit -m "feat(frontend): 1h ttl for cache storage tiles"
```

---

## Self-Review

**Spec coverage:**
- crc32 content ETags (spec §Design tileserver) → Task 1 helper + Tasks 2-3 handlers.
- 1h TTL everywhere in scope (spec §Design) → `cached_response` max-age 3600 used by all handlers; frontend TTL via `TILE_CACHE_TTL_MS` (Task 4).
- Frontend hard-expiry timestamp approach (approved Q1) → Task 4 metadata + delete-on-stale.
- Both endpoints crc32 (approved Q2) → Task 2 covers `/tiles` and `/diff/all`.
- Concern #2 brought into scope (approved): HTML + asset cache headers → Task 3.
- Testing section (spec §Testing) → 200 headers + 304 expectations asserted in Tasks 2-3; frontend verified by grep + template tests (Task 4).
- Redirects/errors uncacheable → `cached_response` only called for 200 paths; `text_error` and `redirect_found` untouched.

**Placeholder scan:** No TBD/TODO; every code step has exact code, every verify step has an exact command with expected output.

**Type consistency:** `cached_response(status, mime, max_age: Duration, data: Bytes, if_none_match: Option<&str>) -> Response` is defined once in Task 1 and all Task 2-3 call sites use matching argument order and types (`Duration::from_secs(3600)`, `Bytes::from(...)`, `Option<&str>`). Local variable `if_none_match` shadows nothing after Task 2 removes the old function. ETag format constant (`"{:08x}"`, content-hash based) matches the asserted `format!("\"{:08x}\"", crc32fast::hash(...))` in all tests.