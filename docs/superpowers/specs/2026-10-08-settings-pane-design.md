# Settings pane (compact floating card) — Design

## Background

`frontend/index.html` exposes map controls ad hoc: the art-opacity button at
`#transparency-toggle` (bottom-left), the tile-overlay checkbox at
`#overlay-toggle` inside `#extra`, and no UI for `WplaceMapState` fields in
`frontend/assets/map-styles.js` (`basemapType`, `strategy`, `supersampling`).
A full-height side pane (export-panel style) was rejected: too much coverage
on mobile, where part of the map must stay visible.

## Goal

Add a settings pane opened from a gear button below the opacity button. The
pane exposes `basemapType`, layerswap `strategy`, and `supersampling`, and
hosts the moved "show tile overlay" toggle. Compact and mobile-friendly.

## Architecture (approved: compact floating card)

No new module. All changes in `frontend/index.html` (+ i18n keys), reusing
the existing `map-styles.js` API. No change to overlay-canvas or
worker/tile logic.

### Button + toolbar

- `#map-toolbar` (absolute, `bottom:10px; left:10px`) is a single vertical
  flex column holding, top to bottom: the `.zoom-button-container` (+/−
  box), `#toggle-transparency` (existing, unchanged handler), and the new
  `#toggle-settings` below it. No per-element absolute offsets, so the
  stack can grow without overlapping.
- `#toggle-settings` reuses the existing button styling, 22px gear SVG icon,
  `title="{{t:settings_title}}"`, `aria-label` for a11y.

### Pane

- New `#settings-panel`: small floating card to the right of the toolbar,
  bottom-aligned with it (`position:absolute; bottom:10px; left:56px;
  width:~250px; max-width:calc(100vw - 76px); max-height:60vh;
  overflow-y:auto`), styled like `#encart` (`var(--panel-bg)`, radius,
  shadow, border).
- Hidden by default; toggles on gear click; closes on gear re-click, ✕
  button, and `Escape`. Independent from `#export-panel` (both may coexist;
  no `body.export-open` interaction).
- Contents (compact label + control rows):
  - Basemap: `<select id="settings-basemap">` (`vector` / `raster`),
    initialised from `WplaceMapState.basemapType`.
  - Layer swap: `<select id="settings-layerswap">` (`swap` / `reload`),
    initialised from `WplaceMapState.strategy`.
  - Supersampling: `<select id="settings-supersampling">` (`1/2/4/8`),
    initialised from `WplaceMapState.supersampling`.
  - Tile overlay: checkbox `id="toggle-tile-overlay"` moved verbatim from
    `#extra #overlay-toggle` (which is removed); existing
    `setOverlayVisibility` / `drawTileOverlay` wiring moves with it
    (overlay canvas is created on map load, so the checkbox element must
    exist before that; keep the same id so no logic change is needed).

### Data flow

- Each of the three selects calls the existing
  `setWplaceVersion(map, wplaceVersion, { basemapType, strategy,
  supersampling })`, which already handles `reload` vs `swap` rebuilds and
  preserves opacity via `WplaceMapState.isTransparent`. No URL-param writes
  in v1 (initial values still come from `initWplaceStateFromUrl`).
- Exact CSS values may be revised later (explicitly deferred).

## Error handling

- `setWplaceVersion` promise rejection is swallowed (same as version-slider
  handler: `.catch(() => {})`); overlay toggle is local-only, no failure
  mode. Invalid select values fall back to current state (never thrown to
  UI).

## i18n

New keys in `tileserver/i18n/en.json` (reference) + `es/ja/ko/pt-BR/ru/tr`:
`settings`, `settings_title`, `close_settings_panel`, `basemap`,
`basemap_vector`, `basemap_raster`, `layerswap`, `layerswap_swap`,
`layerswap_reload`, `supersampling`. Existing `show_tile_overlay` key is
reused for the moved checkbox.

## Testing

Manual: gear opens/closes pane (click, ✕, Escape); each select visibly
changes the map without console errors; overlay checkbox still toggles and
redraws the canvas; desktop + narrow-viewport (mobile) check that most of
the map stays visible; `cargo test` / tileserver suite unaffected (i18n-only
addition).
