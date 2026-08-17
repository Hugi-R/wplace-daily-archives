# Export UI design (screenshot / video)

**Date:** 2026-08-18
**Status:** Approved

## Goal

Replace the two separate `Screenshot` and `Video` buttons in `frontend/index.html` with a single `Export` button opening a docked panel. The panel exports the map area as a static PNG or an Animated PNG (APNG), with user-adjustable tile bounds (min/max x/y), a target z level, and — for APNG — a start/end date range. A live preview is shown on the map.

## Background / constraints (explored)

- Tiles are stored as `TileHistory` blobs at z ∈ {0..9, 11} in SQLite; **z=10 is not stored** (merge skips it; merges z=11 → z=9 directly). Every stored z level uses 1000×1000 px tiles, so no resize logic is needed for export.
- `frontend/src/lib.rs` WASM functions:
  - `wasm_screenshot(base_url, version, x1, y1, x2, y2)` — hardcodes `z=11` in the tile fetch URL.
  - `wasm_video(base_url, x1, y1, x2, y2, from, to)` — hardcodes `z=11` in the `/diff/all` fetch URL.
- `tileserver/src/main.rs` `/diff/all/{z}/{x}/{y}.zst` rejects any z ≠ 11 (line ~554); `/tiles/{version}/{z}/{x}/{y}.zst` already supports z 0..11.
- Versions are `DateHours` (u32, hours since 2025-01-01T00:00:00Z). `WPLACE_VERSIONS` (injected server-side into `index.html`) lists `{version, date}` pairs.
- Caps today: PNG ≤ 400 tiles, APNG ≤ 64 tiles. Kept.
- Real week DBs exist under `tmp/weeks/` for manual verification.

## Scope decisions

- One `Export` button replaces the existing `Screenshot` and `Video` buttons.
- Export panel is a docked non-modal panel on the right edge; the map stays interactive underneath.
- While the panel is open, all other chrome is hidden (only the map + panel remain). Restored on close.
- No re-implementation of PNG/APNG encoding in JS; reuse the Rust WASM path.
- APNG supported at all z (0..9, 11); requires the tileserver change below.

## Design

### Section 1 — Export panel (frontend/index.html)

- One `Export` button (`#export-btn`) in `#extra`, replacing the two old buttons.
- Docked, non-modal panel: fixed, `top: 0; right: 0`, full height, scrollable; styled with the existing panel/surface CSS variables. No backdrop; map stays interactive.
- Contents:
  - **Type toggle**: radio buttons `PNG` / `APNG` (default PNG).
  - **Target z**: `<select>` with z = 0..9, 11 (10 excluded). Default = `floor(map zoom)` clamped into the valid set, else 11.
  - **Bounds**: number inputs Min X, Max X, Min Y, Max Y. Integers clamped to `[0, 2^z-1]`; `min ≤ max` enforced.
  - **Live info line**: `W×H tiles = N`, output `AxB px` (each tile 1000×1000). Updates on every change.
  - **Date section (APNG only)**: `Start` and `End` dropdowns listing `WPLACE_VERSIONS` (label = date, value = version). `Start ≤ End` enforced (swap). Caption `Last frame: <end date>`.
  - **Actions**: `Export` + `Cancel` (× in corner / implicit via closing).
- Behavior:
  - PNG: date section hidden; exported frame uses the currently selected map version (`wplaceVersion`).
  - APNG: date section shown; map temporarily switches to the end-date version (see Section 2).
  - Changing z re-derives min/max x/y from the currently selected geographic rectangle (same ground area).
  - Export disabled + red inline hint when tile count exceeds the cap (PNG 400, APNG 64) or inputs invalid. Cancel restores everything.
- While the panel is open, a `body.export-open` class hides `#extra`, the zoom button stack, the transparency toggle, and the `#version-select` top bar. Restored on close.

### Section 2 — Map preview (selection + APNG end-frame)

- `#export-overlay-canvas` appended to the map container while the panel is open (absolute, `pointer-events: none`, above the existing tile-overlay canvas). Draws the rectangle for the current bounds using `tile2lng`/`tile2lat` + `map.project(...)`: `west = tile2lng(minX,z)`, `east = tile2lng(maxX+1,z)`, `north = tile2lat(minY,z)`, `south = tile2lat(maxY+1,z)`. Semi-transparent fill + strong stroke (reuse `--tile-overlay-*` CSS vars); small corner label `N tiles · AxB px`.
- Redraw on map `move`/`zoom`/`resize` and on bounds/z/type changes while open. Guard against duplicate canvases on style reloads (`setStyle` re-fires `load`). On close, remove `#export-overlay-canvas`, remove event listeners, and reset `body.export-open`.
- APNG end-frame preview: when APNG is selected and End changes, the map switches to the end-date version (`map.setStyle(getMapStyle(endVersion))`), tracking the previous version. On close, the original version, slider position, and URL are restored.
- PNG: map keeps showing the current version; the export uses that same version.

### Section 3 — WASM changes (frontend/src/lib.rs)

- `wasm_screenshot(base_url, z, version, x1, y1, x2, y2)` — tile fetch URL becomes `{base}/{version.week()}/{z}/{x}/{y}.zst`.
- `wasm_video(base_url, z, x1, y1, x2, y2, from, to)` — fetch URL becomes `{base}/all/{z}/{x}/{y}.zst?from=…&to=…`.
- Stitching/`init_img`/1000×1000 unchanged.
- JS call sites updated accordingly.

### Section 4 — Tileserver change (tileserver/src/main.rs)

- Remove the `z != 11` rejection in `serve_all_diff`. `parse_tile_coords` already bounds z to 0..11; `get_all_diffs` already parametrizes z in the query.
- z=10 naturally 404s (nothing stored); frontend selector excludes it anyway.
- Diff-concatenation semantics unchanged. **Caveat:** lower-z histories were produced via increment+merge and have never been exercised through this endpoint — verify against `tmp/weeks` (z=9, z=0).

### Section 5 — JS wiring, limits, export flow (frontend/index.html)

- On open, compute initial state: z = floored map zoom clamped into {0..9, 11}; bounds = viewport tiles at that z; Start = current `wplaceVersion`; End = latest `WPLACE_VERSIONS`.
- Every input change recomputes `tileCount` and px size; clamp/validate bounds; disable Export + red hint when over cap or invalid.
- Export click:
  - PNG → `wasm_screenshot("tiles", z, version, x1, y1, x2, y2)` with `version = parseInt(wplaceVersion)`.
  - APNG → `wasm_video("diff", z, x1, y1, x2, y2, from, to)`.
  - Button shows `Exporting…` and is disabled during run. Wasm progress logs rendered in the panel status area; then download triggered; on error, red text in the panel status area (+ `console.error`).
- Filenames: PNG `w{version}-z{z}-{x1}-{y1}-{x2}-{y2}.png`; APNG `w{from}-{to}-z{z}-{x1}-{y1}-{x2}-{y2}.png`.
- Remove the old `#user-messages` container from HTML; `log_user_message` appends into the panel status area instead.

### Section 6 — Testing & verification

- Build via `build_server.sh` (release build + `wasm-pack build --target web --no-default-features` + copy artifacts into `tmp/`).
- Serve `wpda-tileserver` with `DATA_PATH=tmp`.
- Manual E2E checklist:
  1. Open panel → chrome hides, selection rectangle matches viewport.
  2. Change z → bounds preserve ground area; z=10 absent.
  3. Over-cap / invalid → Export disabled + red hint; under-cap → enabled; counts correct.
  4. PNG export at z=11 and a low z (e.g. 0) → downloads, opens, correct.
  5. APNG: Start/End → map shows end frame, caption shows last-frame date; export at z=11, z=9, z=0 with `tmp/weeks` data → APNG opens and animates.
  6. Close panel → chrome restored, map version/slider/URL restored.
  7. Server: `curl /diff/all/{9,0}/{x}/{y}.zst?from=…&to=…` returns 200 + decodable body (also covered by step 5).
  8. Regression: map rendering, share-URL round-trip, right-click download tile, version slider.

## Out of scope

- Adjustable export size caps, APNG frame delay/count settings, and pure-JS composition were considered and rejected.
- z=10 export (no data stored).