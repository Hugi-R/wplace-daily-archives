# GIF export: user-configurable frame delay and date stamp

Date: 2026-08-29

## Problem

`wasm_video` in `frontend/src/lib.rs` hardcodes `tilehistory::gif_from_history(history, 200, false)`.
Users cannot control the GIF frame delay (2nd positional arg, ms) or the per-frame
date stamp in the corner (3rd positional arg, bool). The wimage function already
supports both: when `show_date` is set, each frame is stamped with the UTC calendar
date (`YYYY-MM-DD`) of the version it represents.

## Goal

Expose `frame_delay_ms` and `show_date` through `wasm_video` and wire them to two
new controls in the export panel, shown only for Animated GIF exports.

## Non-goals

- No changes to the Static PNG path.
- No changes to the tileserver HTTP API or diff endpoints.
- No APNG rename/refactor of the `exportParams.type === 'apng'` value (label is
  already "Animated GIF"; renaming the value is churn with no benefit).

## Design

### WASM (`frontend/src/lib.rs`)

Append two parameters to `wasm_video`:

```rust
pub async fn wasm_video(
    base_url: &str, z: i64, x1: i64, y1: i64, x2: i64, y2: i64,
    from: u32, to: u32, delay: u16, date: bool,
) -> Result<Vec<u8>, JsValue>
```

Line 155 becomes `tilehistory::gif_from_history(history, delay, date)`.

Rationale: matches the existing positional style; the only JS caller is
`frontend/index.html`; the tileserver template test asserts only the prefix
`wasm_video("/diff",`, so it stays green.

### HTML frontend (`frontend/index.html`)

Two controls inside the existing `#export-date-section` (visible only when the
Animated GIF radio is selected — same show/hide logic as start/end dates):

- `<input type="number" id="export-delay">` — frame delay in ms.
  Default `200`, min `10`, max `10000`, step `10`. GIF stores the delay in
  centiseconds (`frame_delay_ms / 10` inside wimage), so values outside the
  range or non-multiples of 10 would silently round; the input constraints
  make that moot. The run handler additionally clamps with the existing
  `clampInt`-style logic before passing the value.
- `<input type="checkbox" id="export-date-cb">` — show date in corner.
  Default unchecked (today's behavior).

Export run handler passes both through:

```js
const delay = Math.min(10000, Math.max(10, parseInt(document.getElementById('export-delay').value, 10) || 200));
const showDate = document.getElementById('export-date-cb').checked;
const data = await wasmModule.wasm_video("/diff", z, x1, y1, x2, y2, from, to, delay, showDate);
```

The delay/date controls need no recompute of `export-info` (they affect
neither tile count nor caps), so no listener wiring beyond defaults is required.

### i18n (`tileserver/i18n/{en,es,pt-BR,ru,ko,ja}.json`)

New keys, translated per language:

- `"frame_delay"`: label for the delay input, e.g. en "Frame delay (ms)".
- `"gif_show_date"`: label for the checkbox, e.g. en "Show date in corner".

Labels use the existing `{{t:...}}` template markers so the i18n render
pipeline picks them up.

## Data flow

Export panel (GIF selected) → user sets delay / date checkbox → run handler
reads + clamps → `wasm_video(…, delay, date)` → `gif_from_history(history,
delay, date)` → GIF bytes → `saveFile(..., 'image/gif')`. No new state in
`exportParams` (values are read only at run time, like `from`/`to`).

## Error handling

No new error paths. Invalid/empty delay input falls back to the clamped
default (200) via the run-handler clamp. Out-of-range values are clamped.

## Testing

- `cargo build` + `clippy` for the tileserver workspace (i18n JSON changes).
- Tileserver i18n/template render tests (assert markers gone, prefix strings
  intact, `window.I18N` populated — must still pass with the two new keys).
- `wasm-pack build --target web --no-default-features` for the frontend to
  regenerate `pkg/` with the new `wasm_video` signature.
- Manual E2E: export a small GIF with default settings; then delay 1000 ms
  and date on; confirm timing and corner date stamp in the saved GIF.
