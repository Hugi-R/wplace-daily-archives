# SEO robots.txt + sitemap.xml Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Serve `/robots.txt` and `/sitemap.xml` from the tileserver, generated once at startup from `Lang::ALL` + `SITE_BASE`, per spec `docs/superpowers/specs/2026-08-22-seo-robots-sitemap-design.md`.

**Architecture:** Two pure builder functions next to `build_index` produce the file bodies; they are stored as `Bytes` on `TileServer` at startup and served through the existing `cached_response` helper (content-crc32 ETag + `Cache-Control: public, max-age=3600`) via two new axum routes.

**Tech Stack:** Rust, axum 0.8, tower (tests use `tower::ServiceExt::oneshot`), chrono. No new dependencies.

All commands run from the repository root. The crate is `wpda-tileserver`; run tests with `cargo test -p wpda-tileserver`.

---

### Task 1: robots.txt builder

**Files:**
- Modify: `tileserver/src/main.rs` — add builder function directly after the `build_index` function (it ends with `Ok((pages, latest_version))` / closing brace), add unit test inside the existing `#[cfg(test)] mod tests`.

- [ ] **Step 1: Write the failing test**

Add inside `mod tests` in `tileserver/src/main.rs` (next to the other builder tests like `render_index_replaces_all_placeholders`):

```rust
    #[test]
    fn build_robots_txt_blocks_data_endpoints_and_declares_sitemap() {
        let body = build_robots_txt();
        assert!(body.starts_with("User-agent: *\n"), "{body}");
        assert!(body.contains("Disallow: /tiles/\n"), "{body}");
        assert!(body.contains("Disallow: /diff/\n"), "{body}");
        assert!(
            body.contains(&format!("Sitemap: {SITE_BASE}/sitemap.xml")),
            "{body}"
        );
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p wpda-tileserver build_robots_txt -- --nocapture`
Expected: compile error — `cannot find function \`build_robots_txt\` in this scope`

- [ ] **Step 3: Write minimal implementation**

Directly after the `build_index` function in `tileserver/src/main.rs`:

```rust
/// Builds the robots.txt body: block heavy binary data endpoints, declare
/// the sitemap. `/assets/` stays crawlable so search engines can render.
fn build_robots_txt() -> String {
    format!(
        "User-agent: *\n\
         Disallow: /tiles/\n\
         Disallow: /diff/\n\
         \n\
         Sitemap: {SITE_BASE}/sitemap.xml\n"
    )
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p wpda-tileserver build_robots_txt -- --nocapture`
Expected: PASS (1 test)

- [ ] **Step 5: Commit**

```bash
git add tileserver/src/main.rs
git commit -m "feat(tileserver): add robots.txt body builder"
```

---

### Task 2: sitemap.xml builder

**Files:**
- Modify: `tileserver/src/main.rs` — add builder function directly after `build_robots_txt` from Task 1, add unit tests inside `mod tests`.

- [ ] **Step 1: Write the failing tests**

Add inside `mod tests`:

```rust
    #[test]
    fn build_sitemap_xml_lists_all_languages_with_alternates() {
        let xml = build_sitemap_xml(0);
        assert!(
            xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"),
            "{xml}"
        );
        assert!(
            xml.contains("xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\""),
            "{xml}"
        );
        assert!(
            xml.contains("xmlns:xhtml=\"http://www.w3.org/1999/xhtml\""),
            "{xml}"
        );
        // One entry per language page; root "/" excluded (it is a redirect).
        assert_eq!(xml.matches("<url>").count(), Lang::ALL.len(), "{xml}");
        for lang in Lang::ALL {
            assert!(
                xml.contains(&format!("<loc>{SITE_BASE}/{}/</loc>", lang.path())),
                "{xml}"
            );
            // Every language appears as an alternate once per entry.
            assert_eq!(
                xml.matches(&format!("hreflang=\"{}\"", lang.code())).count(),
                Lang::ALL.len(),
                "{xml}"
            );
        }
        assert_eq!(
            xml.matches("hreflang=\"x-default\"").count(),
            Lang::ALL.len(),
            "{xml}"
        );
        assert!(
            xml.contains(&format!(
                "hreflang=\"x-default\" href=\"{SITE_BASE}/{}/\"",
                Lang::En.path()
            )),
            "{xml}"
        );
        assert_eq!(
            xml.matches("<changefreq>daily</changefreq>").count(),
            Lang::ALL.len(),
            "{xml}"
        );
    }

    #[test]
    fn build_sitemap_xml_lastmod_from_latest_epoch_hour() {
        let xml = build_sitemap_xml(0);
        assert!(
            xml.contains("<lastmod>2025-01-01T00:00:00+00:00</lastmod>"),
            "{xml}"
        );
    }
```

Note on fixtures: epoch hour `0` is 2025-01-01T00 UTC (`wplace_epoch()`), so `epoch_hour_to_date(0)` returns `"2025-01-01T00"`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p wpda-tileserver build_sitemap_xml -- --nocapture`
Expected: compile error — `cannot find function \`build_sitemap_xml\` in this scope`

- [ ] **Step 3: Write minimal implementation**

Directly after `build_robots_txt`:

```rust
/// Builds the sitemap body: one entry per language page with the full
/// hreflang alternate set (plus x-default), lastmod derived from the latest
/// archive version, daily changefreq (new snapshots land daily). The root
/// URL is excluded on purpose: it is an Accept-Language redirect, not
/// canonical content.
fn build_sitemap_xml(latest_epoch_hour: u32) -> String {
    let lastmod = format!("{}:00:00+00:00", epoch_hour_to_date(latest_epoch_hour));
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\"\n\
         xmlns:xhtml=\"http://www.w3.org/1999/xhtml\">\n",
    );
    for lang in Lang::ALL {
        out.push_str("  <url>\n");
        out.push_str(&format!("    <loc>{SITE_BASE}/{}/</loc>\n", lang.path()));
        for alt in Lang::ALL {
            out.push_str(&format!(
                "    <xhtml:link rel=\"alternate\" hreflang=\"{}\" href=\"{SITE_BASE}/{}/\"/>\n",
                alt.code(),
                alt.path()
            ));
        }
        out.push_str(&format!(
            "    <xhtml:link rel=\"alternate\" hreflang=\"x-default\" href=\"{SITE_BASE}/{}/\"/>\n",
            Lang::En.path()
        ));
        out.push_str(&format!("    <lastmod>{lastmod}</lastmod>\n"));
        out.push_str("    <changefreq>daily</changefreq>\n");
        out.push_str("  </url>\n");
    }
    out.push_str("</urlset>\n");
    out
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p wpda-tileserver build_sitemap_xml -- --nocapture`
Expected: PASS (2 tests)

- [ ] **Step 5: Commit**

```bash
git add tileserver/src/main.rs
git commit -m "feat(tileserver): add sitemap.xml body builder"
```

---

### Task 3: Wire builders into server state and routing

**Files:**
- Modify: `tileserver/src/main.rs`:
  - `struct TileServer` (fields) and `impl TileServer::new` (populate them)
  - handler functions placed directly after `serve_favicon`
  - route registrations in `build_router`
  - router tests added inside `mod tests`

- [ ] **Step 1: Write the failing router tests**

Add inside `mod tests` (they follow the same pattern as `lang_path_serves_localized_page`; `router_fixture`, `Body`, `Request`, `StatusCode`, `to_bytes` are already available there):

```rust
    #[tokio::test]
    async fn robots_txt_served_as_text() {
        let tmp = router_fixture();
        let ts = Arc::new(TileServer::new(tmp.path().to_path_buf()).unwrap());
        let app = build_router(ts);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/robots.txt")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get("content-type").unwrap(),
            "text/plain; charset=utf-8"
        );
        let body = String::from_utf8(
            to_bytes(resp.into_body(), 64 * 1024).await.unwrap().to_vec(),
        )
        .unwrap();
        assert!(body.starts_with("User-agent: *"), "{body}");
        assert!(
            body.contains(&format!("Sitemap: {SITE_BASE}/sitemap.xml")),
            "{body}"
        );
    }

    #[tokio::test]
    async fn sitemap_xml_served_as_xml_with_etag_revalidation() {
        let tmp = router_fixture();
        let ts = Arc::new(TileServer::new(tmp.path().to_path_buf()).unwrap());
        let app = build_router(ts);
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/sitemap.xml")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers().get("content-type").unwrap(), "application/xml");
        let etag = resp
            .headers()
            .get("etag")
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        let body = String::from_utf8(
            to_bytes(resp.into_body(), 64 * 1024).await.unwrap().to_vec(),
        )
        .unwrap();
        for lang in Lang::ALL {
            assert!(
                body.contains(&format!("<loc>{SITE_BASE}/{}/</loc>", lang.path())),
                "{body}"
            );
        }
        let revalidated = app
            .oneshot(
                Request::builder()
                    .uri("/sitemap.xml")
                    .header("if-none-match", etag)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(revalidated.status(), StatusCode::NOT_MODIFIED);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p wpda-tileserver robots_txt_served -- --nocapture && cargo test -p wpda-tileserver sitemap_xml_served -- --nocapture`
Expected: compile errors — `no field \`robots_txt\` on type \`TileServer\`` (and `sitemap_xml`). This is the red state.

- [ ] **Step 3: Add struct fields, populate them, add handlers and routes**

3a. In `struct TileServer` (after the `favicon: Bytes,` field):

```rust
    robots_txt: Bytes,
    sitemap_xml: Bytes,
```

3b. In `impl TileServer { fn new(...) }`, directly after the `index_html` collection block (the lines ending `.collect();`) — `dates` is guaranteed non-empty here because `build_index(...)?` above already errors on empty:

```rust
        let robots_txt = Bytes::from(build_robots_txt());
        let sitemap_xml = Bytes::from(build_sitemap_xml(
            *dates.last().expect("build_index validated non-empty dates"),
        ));
```

and extend the `Ok(Self { ... })` literal (e.g. right after `favicon,`) with:

```rust
            robots_txt,
            sitemap_xml,
```

3c. Handlers, directly after `serve_favicon`:

```rust
async fn serve_robots(State(ts): State<Arc<TileServer>>, headers: HeaderMap) -> Response {
    let if_none_match = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok());
    cached_response(
        StatusCode::OK,
        "text/plain; charset=utf-8",
        Duration::from_secs(3600),
        ts.robots_txt.clone(),
        if_none_match,
    )
}

async fn serve_sitemap(State(ts): State<Arc<TileServer>>, headers: HeaderMap) -> Response {
    let if_none_match = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok());
    cached_response(
        StatusCode::OK,
        "application/xml",
        Duration::from_secs(3600),
        ts.sitemap_xml.clone(),
        if_none_match,
    )
}
```

3d. In `build_router`, directly after the `/favicon.ico` route line:

```rust
        .route("/robots.txt", get(serve_robots))
        .route("/sitemap.xml", get(serve_sitemap))
```

Static segments take precedence over the existing `/{lang}` param route, so no conflict is possible.

- [ ] **Step 4: Run the full suite to verify everything passes**

Run: `cargo test -p wpda-tileserver`
Expected: all tests PASS, including the two new router tests and all pre-existing ones.

- [ ] **Step 5: Commit**

```bash
git add tileserver/src/main.rs
git commit -m "feat(tileserver): serve generated robots.txt and sitemap.xml"
```

---

## Verification checklist (end of execution)

- `cargo test -p wpda-tileserver` green.
- Manual smoke check (optional but cheap): `DATA_PATH=<a fixture dir> PORT=8099 cargo run -p wpda-tileserver` then `curl -i http://localhost:8099/robots.txt` and `curl -i http://localhost:8099/sitemap.xml` — expect 200 with the expected bodies and `Cache-Control: public, max-age=3600`.
