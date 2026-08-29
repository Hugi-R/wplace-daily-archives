# GIF Export Delay + Date Stamp Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Expose `gif_from_history`'s frame delay (ms) and corner date-stamp flag through `wasm_video` and give the user two new controls in the Animated GIF export panel.

**Architecture:** Append `delay: u16` and `date: bool` to the existing positional `wasm_video` signature and forward them to `tilehistory::gif_from_history(history, delay, date)`. The export panel gets a number input (frame delay) and a checkbox (date stamp) inside the existing `#export-date-section`, which already shows/hides with the Animated GIF radio. Two new i18n keys are added to all 6 language files (the tileserver template renderer errors on unknown keys, and a test asserts identical key sets across languages).

**Tech Stack:** Rust + wasm-bindgen (frontend WASM, built with wasm-pack), vanilla JS single-file frontend (`frontend/index.html`), axum tileserver that renders `frontend/index.html` as an i18n template from `tileserver/i18n/*.json`.

## Global Constraints

- `frontend/index.html` must keep the literal substring `wasm_video("/diff",` — `tileserver/src/main.rs:1237` asserts it.
- All 6 i18n files (`en`, `es`, `pt-BR`, `ru`, `ko`, `ja`) must have identical key sets; each language must keep 40..=100 keys (adding 2 keys → 59) — `tileserver/src/main.rs:1188-1200`.
- Every `{{t:key}}` marker in `frontend/index.html` must exist in every language file — `render_index` returns `unknown i18n key` error otherwise (`tileserver/src/main.rs:494`).
- GIF stores frame delay in centiseconds: wimage computes `frame_delay_ms / 10` (wimage `tilehistory.rs:424`). UI range: 10–10000 ms, step 10. Defaults: 200 ms, date off (today's hardcoded behavior).
- New wasm signature (exact): `wasm_video(base_url: &str, z: i64, x1: i64, y1: i64, x2: i64, y2: i64, from: u32, to: u32, delay: u16, date: bool) -> Result<Vec<u8>, JsValue>`.
- WASM build command (from `frontend/`): `~/.cargo/bin/wasm-pack build --target web --no-default-features`. `frontend/pkg/` is gitignored — never commit it.
- No changes to `exportParams.type` values; the radio value stays `'apng'` (label already reads "Animated GIF").
- wimage dependency is pinned by rev `3bcdd0759b306be7ea000dfd38aa8978e5a60943` — do not update it.

---

### Task 1: Export panel controls + i18n keys

**Files:**
- Modify: `frontend/index.html:411-417` (markup), `frontend/index.html:1510-1524` (run handler wiring comes in Task 2 — do NOT touch the JS in this task)
- Modify: `tileserver/i18n/en.json`, `tileserver/i18n/es.json`, `tileserver/i18n/pt-BR.json`, `tileserver/i18n/ru.json`, `tileserver/i18n/ko.json`, `tileserver/i18n/ja.json`

**Interfaces:**
- Consumes: existing template markers `{{t:...}}` rendered by the tileserver.
- Produces: DOM elements `#export-delay` (number input, default 200) and `#export-date-cb` (checkbox, default unchecked); i18n keys `frame_delay`, `gif_show_date`. Task 2's JS reads these IDs/keys.

- [ ] **Step 1: Add the two controls to the date section markup (red)**

In `frontend/index.html`, replace lines 411-417:

```html
    <section id="export-date-section">
      <label for="export-start">{{t:start_date}}</label>
      <select id="export-start"></select>
      <label for="export-end">{{t:end_date}}</label>
      <select id="export-end"></select>
      <div id="export-last-frame"></div>
    </section>
```

with:

```html
    <section id="export-date-section">
      <label for="export-start">{{t:start_date}}</label>
      <select id="export-start"></select>
      <label for="export-end">{{t:end_date}}</label>
      <select id="export-end"></select>
      <label for="export-delay">{{t:frame_delay}}</label>
      <input type="number" id="export-delay" value="200" min="10" max="10000" step="10">
      <label><input type="checkbox" id="export-date-cb"> {{t:gif_show_date}}</label>
      <div id="export-last-frame"></div>
    </section>
```

The section already has `#export-date-section { display: none; }` (line 307) and is shown/hidden by the existing type-change listener (line 1264), so the new controls inherit GIF-only visibility with zero extra JS.

- [ ] **Step 2: Run the tileserver template tests to verify they fail**

Run: `cargo test real_`
Expected: FAIL — `real_template_renders_all_languages_without_leftovers` errors with `unknown i18n key '{{t:frame_delay}}'`.

- [ ] **Step 3: Add translations to all 6 language files**

Insert both keys immediately after the `"export_frames"` line in each file (key sets must stay identical across files).

`tileserver/i18n/en.json`:

```json
  "frame_delay": "Frame delay (ms)",
  "gif_show_date": "Show date in corner",
```

`tileserver/i18n/es.json`:

```json
  "frame_delay": "Retardo de fotograma (ms)",
  "gif_show_date": "Mostrar fecha en la esquina",
```

`tileserver/i18n/pt-BR.json`:

```json
  "frame_delay": "Atraso de quadro (ms)",
  "gif_show_date": "Mostrar data no canto",
```

`tileserver/i18n/ru.json`:

```json
  "frame_delay": "Задержка кадра (мс)",
  "gif_show_date": "Показывать дату в углу",
```

`tileserver/i18n/ko.json`:

```json
  "frame_delay": "프레임 지연 (ms)",
  "gif_show_date": "모서리에 날짜 표시",
```

`tileserver/i18n/ja.json`:

```json
  "frame_delay": "フレーム間隔 (ms)",
  "gif_show_date": "隅に日付を表示",
```

- [ ] **Step 4: Run the tileserver tests to verify they pass**

Run: `cargo test real_`
Expected: PASS — both `real_translation_files_have_identical_key_sets` and `real_template_renders_all_languages_without_leftovers`.

- [ ] **Step 5: Commit**

```bash
git add frontend/index.html tileserver/i18n/
git commit -m "feat(frontend): GIF export controls for frame delay and date stamp"
```

---

### Task 2: WASM signature + JS wiring

**Files:**
- Modify: `frontend/src/lib.rs:113` (signature), `frontend/src/lib.rs:155` (call site)
- Modify: `frontend/index.html:1510-1524` (run handler)

**Interfaces:**
- Consumes: `#export-delay` and `#export-date-cb` from Task 1; wimage `gif_from_history(history: HashMap<(u16, u16), TileHistory>, frame_delay_ms: u16, show_date: bool) -> anyhow::Result<Vec<u8>>`.
- Produces: new exported wasm signature `wasm_video(base_url, z, x1, y1, x2, y2, from, to, delay, date)` (u16 → JS `number`, bool → JS `boolean` in the generated `frontend/pkg/wimage_wasm.d.ts`).

- [ ] **Step 1: Update `wasm_video` in `frontend/src/lib.rs`**

Replace line 113:

```rust
pub async fn wasm_video(base_url: &str, z: i64, x1: i64, y1: i64, x2: i64, y2: i64, from: u32, to: u32) -> Result<Vec<u8>, JsValue> {
```

with:

```rust
pub async fn wasm_video(base_url: &str, z: i64, x1: i64, y1: i64, x2: i64, y2: i64, from: u32, to: u32, delay: u16, date: bool) -> Result<Vec<u8>, JsValue> {
```

Replace line 155:

```rust
    let img = tilehistory::gif_from_history(history, 200, false)
```

with:

```rust
    let img = tilehistory::gif_from_history(history, delay, date)
```

- [ ] **Step 2: Wire the JS run handler in `frontend/index.html`**

In the `export-run-btn` click handler, replace (lines 1513-1516):

```js
        exportParams.from = document.getElementById('export-start').value;
        exportParams.to = document.getElementById('export-end').value;
        const from = parseInt(exportParams.from, 10);
        const to = parseInt(exportParams.to, 10);
```

with:

```js
        exportParams.from = document.getElementById('export-start').value;
        exportParams.to = document.getElementById('export-end').value;
        const from = parseInt(exportParams.from, 10);
        const to = parseInt(exportParams.to, 10);
        const delay = Math.min(10000, Math.max(10, parseInt(document.getElementById('export-delay').value, 10) || 200));
        const showDate = document.getElementById('export-date-cb').checked;
```

Then replace (line 1523):

```js
            const data = await wasmModule.wasm_video("/diff", z, x1, y1, x2, y2, from, to);
```

with:

```js
            const data = await wasmModule.wasm_video("/diff", z, x1, y1, x2, y2, from, to, delay, showDate);
```

(The `|| 200` handles an emptied input; the clamp enforces the 10–10000 ms range regardless of what the DOM min/max allow. `wasm_video("/diff",` prefix is preserved for the tileserver test.)

- [ ] **Step 3: Rebuild the WASM package**

Run: `cd frontend && ~/.cargo/bin/wasm-pack build --target web --no-default-features`
Expected: build succeeds; `frontend/pkg/` regenerated.

- [ ] **Step 4: Verify the generated signature**

Run: `grep -n "wasm_video" frontend/pkg/wimage_wasm.d.ts`
Expected: `export function wasm_video(base_url: string, z: bigint, x1: bigint, y1: bigint, x2: bigint, y2: bigint, from: number, to: number, delay: number, date: boolean): Promise<Uint8Array>;`

- [ ] **Step 5: Run the full tileserver test suite**

Run: `cargo test`
Expected: PASS (all tests, including the template render assertion on `wasm_video("/diff",`).

- [ ] **Step 6: Commit**

```bash
git add frontend/src/lib.rs frontend/index.html
git commit -m "feat: expose GIF frame delay and date stamp through wasm_video"
```

---

### Task 3: Manual E2E verification

**Files:**
- None (verification only). Requires a running server with data: `./build_server.sh && ./tmp/wpda-tileserver` (or the project's usual run setup).

- [ ] **Step 1: Export a GIF with defaults**

Open the site, open the export panel, pick "Animated GIF", small area (1 tile), keep delay=200 and date unchecked, run. Expected: GIF downloads (`w{from}-{to}-z…gif`), animates at ~200 ms/frame, no date stamp in the corner.

- [ ] **Step 2: Export with date stamp on and slow delay**

Set delay to 1000, check "Show date in corner", run. Expected: GIF frames advance ~1 s apart and each frame carries a white `YYYY-MM-DD` date in the corner.

- [ ] **Step 3: Verify clamping**

Clear the delay input and export. Expected: no crash — the handler falls back to 200 ms.

No commit (nothing changed; this task is the acceptance gate).

---

## Self-Review

- **Spec coverage:** WASM signature + forwarding (Task 2 Step 1), panel controls in `#export-date-section` with defaults 200/off and 10–10000 range (Task 1 Step 1), run-handler clamp + pass-through (Task 2 Step 2), i18n keys in all 6 languages (Task 1 Step 3), template test constraint (Global Constraints + Task 1 Step 2/4), wasm rebuild + d.ts check (Task 2 Steps 3-4), manual E2E (Task 3). No gaps.
- **Placeholder scan:** none — every step has exact code, paths, commands, and expected results.
- **Type consistency:** `delay`/`showDate` JS names match Task 2 usage; DOM IDs `export-delay`/`export-date-cb` consistent between Tasks 1 and 2; i18n keys `frame_delay`/`gif_show_date` consistent between markup and JSON; Rust param names `delay`/`date` forwarded verbatim to `gif_from_history`.
