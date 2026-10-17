# Settings-pane tooltips (mobile-friendly ? icons) — Design

## Background

`frontend/index.html` settings pane (`#settings-panel`) exposes five settings
(basemap, layer swap, supersampling, tile-grid overlay, denoise) as bare
labels + controls with no explanation. Existing tooltips in the app use native
`title=` attributes, which do not work on mobile touch devices. The user
provided per-setting tooltip copy and requires the wording to match the
displayed option labels in every language.

## Goal

Add a small `?` info button next to each of the five settings labels.
Clicking it toggles a styled, server-translated tooltip. Identical behavior
on desktop and mobile: click/tap toggles, light-dismiss (outside-click, Esc,
re-click) closes. Hover does not open (explicitly deferred).

## Architecture (approved: native Popover API, click-only)

No new module. All changes in `frontend/index.html` (+ i18n keys). Native
Popover API (`popover` attribute + `popovertarget` buttons) provides
top-layer rendering (immune to the panel's `overflow-y: auto` clipping) and
native light-dismiss for free. Browser baseline is fine: the app already
requires WebGL2 / MapLibre GL; Popover API needs Chrome 114+, Safari 17+,
Firefox 125+.

### Markup

Per setting, a flex header row + hidden popover div; existing control ids,
handlers, and checkbox markup stay untouched:

```html
<div class="setting-head">
  <label for="settings-basemap">{{t:basemap}}</label>
  <button type="button" class="help-btn" popovertarget="tip-basemap"
          aria-label="{{t:help}}: {{t:basemap}}">?</button>
</div>
<select id="settings-basemap">…</select>
<div id="tip-basemap" class="help-tip" popover>{{t:basemap_hint}}</div>
```

- Five buttons + five popover divs (ids: `tip-basemap`, `tip-layerswap`,
  `tip-supersampling`, `tip-tile-overlay`, `tip-denoise`).
- The two checkboxes keep their exact current
  `<label><input type="checkbox" …> text</label>` form, wrapped in the same
  `.setting-head` row with the `?` button.
- Tooltip text is rendered server-side via `{{t:…}}`; no client-side
  translation lookup needed.

### CSS (inline `<style>` block, near existing `#settings-panel` rules)

- `.setting-head` — `display:flex; align-items:center; gap:6px;` and carries
  the old label margin (`8px 0 2px`); `#settings-panel label` loses its
  block/margin styling for these rows.
- `.help-btn` — ~18px circular ghost button, `line-height:1; padding:0`,
  styled from existing `:root` vars (`--border-color`, `--button-text`).
- `.help-tip` — small card (background `var(--panel-bg)`, border, radius,
  `var(--shadow-lg)`, padding, `font-size:13px`, `max-width:240px`);
  resets UA popover margins (`margin:0`). Top-layer rendering means the
  panel's `max-height:60vh; overflow-y:auto` can never clip it.

### JS (inline script, near existing settings-panel wiring, ~15 lines)

On each tooltip's `toggle` event, when it opens (`:popover-open`): position
it below the `?` button (above if insufficient space below), left-aligned
with the button, clamped to the viewport with a small margin. No custom
dismiss logic — outside-click, Esc, and re-click are native.

Edge case: closing the settings panel (gear re-click or ✕) while a tooltip
is open must also close the tooltip — the existing panel-close paths call
`hidePopover()` on any open `.help-tip`. (Esc is handled natively: the first
Esc closes the open popover, not the panel.)

### Data flow

None — tooltips are static explanatory text; no interaction with
`WplaceMapState`, `setWplaceVersion`, or URL state.

## Error handling

None needed: no async, no failure modes; a missing popover target would
simply no-op.

## i18n

Six new keys in `tileserver/i18n/en.json` (reference) + `es/ja/ko/pt-BR/ru/tr`
(tileserver fails hard on any key-set mismatch):

| Key | English text |
|---|---|
| `help` | Help |
| `basemap_hint` | The background map rendering. Vector is the default Wplace map; Raster is a faster option. |
| `layerswap_hint` | Controls how version changes are rendered. Swap produces no visual artefact but is slow; Reload is immediate but produces a blank flash; Overlay draws the new tiles on top, then removes the old ones. |
| `supersampling_hint` | How many images are used to render a tile — the more, the sharper, but at a huge cost in bandwidth and computing. |
| `show_tile_overlay_hint` | Displays an overlay with the tile grid and coordinates. |
| `denoise_hint` | Removes lone pixels in the final image; best results when combined with supersampling ×4. |

Grammar lightly fixed from the user's draft ("produces", "immediate",
"pixels"). **Translation rule:** each language's hint reuses the exact same
terms already shown in that language's option labels — e.g. the `es`
layer-swap hint must use the same Spanish words as `layerswap_swap`,
`layerswap_reload`, and `layerswap_overlay` in `es.json`.

## Testing

No JS unit tests apply (pure DOM/CSS). Verification:

- Tileserver startup enforces i18n key parity across all seven files (fails
  hard on mismatch) — starting the tileserver validates the i18n addition.
- Manual browser check: `?` click toggles tooltip; outside-click, Esc, and
  re-click dismiss; tooltip positioned near its button and clamped at
  viewport edges; closing the panel (gear/✕) with a tooltip open closes the
  tooltip too; settings controls still function; mobile viewport (devtools)
  tap behavior identical; no console errors.
