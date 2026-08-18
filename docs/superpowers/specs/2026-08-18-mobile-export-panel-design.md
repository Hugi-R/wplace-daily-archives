# Mobile export panel design

**Date:** 2026-08-18
**Status:** Approved

## Goal

Make the export panel in `frontend/index.html` usable on mobile portrait screens. Today the panel is a fixed 280px-wide full-height drawer docked to the right edge; on a phone in portrait it covers most of the screen and hides the map. On portrait screens the panel should instead appear as a scrollable sheet docked to the top of the viewport, with a tighter layout so the map stays visible below.

## Background / constraints (explored)

- `#export-panel` (frontend/index.html:237) is `position: fixed; top: 0; right: 0; height: 100%; width: 280px;` with `overflow-y: auto` and `display: none` until `.open` is added. Open/close is handled purely in JS (`openExportPanel`/`closeExportPanel`, lines 874/919) by toggling `.open` on the panel and `export-open` on `body`.
- While open, `body.export-open` hides all other chrome (`#extra`, zoom buttons, transparency toggle, `#version-select`). Those rules are unaffected by this change.
- The file uses inline `@media (max-width: 600px)` and `@media (max-width: 500px)` blocks already; adding an inline `@media (orientation: portrait)` block matches convention.
- No JS reads the panel's dimensions or position, so a pure CSS override is safe.

## Scope decisions

- Pure CSS approach: one `@media (orientation: portrait)` block overrides the panel geometry and tightens the layout. No JS changes.
- Trigger is `orientation: portrait` so the top sheet applies to all portrait screens (phones and tablets). Landscape screens keep the current right-side drawer.
- Top sheet is scrollable with `max-height: 65vh`; the map remains visible and interactive below it (the bounds preview canvas still works).

## Design

### Portrait media query

One `@media (orientation: portrait)` block, placed after the existing `#export-panel` rules (before the closing `</style>`).

### Panel repositioning

Override on `#export-panel`:

- `top: 0; left: 0; right: 0;` (was `top: 0; right: 0`)
- `height: auto; width: auto;` (was `height: 100%; width: 280px`)
- `max-height: 65vh;` — keeps the map visible below; combined with the existing `overflow-y: auto`, long content scrolls inside the sheet.
- `border-left: none; border-bottom: 1px solid var(--border-color);` — docked to the top edge instead of the right edge.
- `z-index: 200`, background, box-shadow, box-sizing unchanged.
- `#export-close` stays positioned absolute top-right; it now sits against the top sheet's corner, no change needed.

### Tighter layout

Overrides inside the same media query:

- Panel `padding: 8px 12px;` (from 12px), `font-size: 13px;` (from 14px).
- `h2` font-size 14px (from 16px), `margin-bottom: 6px;` (from 10px).
- `section` `margin-bottom: 6px;` (from 12px).
- `.export-bounds` becomes a single row: `grid-template-columns: repeat(4, 1fr);` with `font-size` reduced so the four `Min/Max X/Y` number inputs fit side by side.
- Inputs/selects keep `width: 100%;` and their existing box-sizing.

### Unchanged

- Open/close JS, `body.export-open` chrome-hiding rules, bounds/z/date logic, export flow, filenames.
- All landscape layouts (right-side drawer as today).

## Testing & verification

- Build via `build_server.sh`, serve with `DATA_PATH=tmp`.
- Manual E2E checklist:
  1. DevTools portrait mode (e.g. 390×844 and a tablet ~768×1024): open export → panel appears as a top sheet spanning the width, max ~65vh, map visible below, content scrolls.
  2. Close (×) → panel disappears, chrome restored, map version restored.
  3. Landscape (or desktop): panel still docks to the right edge with the original layout.
  4. Bounds preview rectangle still tracks the map (map interactive below the sheet).
  5. Tile counts / z / date selectors still behave as before.
  6. Regression: PNG and APNG export still work, share-URL round-trip unaffected.

## Out of scope

- Changing the export logic, caps, or behavior.
- Landscape/desktop layout changes.
- JS-driven responsive layout (CSS media query covers live orientation changes natively).
