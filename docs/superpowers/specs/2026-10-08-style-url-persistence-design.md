# Map style settings URL persistence design

Date: 2026-10-08
Status: approved (pending spec review)
Scope: `frontend/assets/map-styles.js`, `frontend/assets/map-styles.test.js`, `frontend/index.html`

## Context

`updateUrlWithMapView()` in `frontend/index.html` persists `lat`/`lng`/`zoom`/`version`
via `history.replaceState` on moveend, version-slider input, and export flows. It
never writes `basemap`/`layerswap`/`supersampling`, and `applySettings()` never
touches the URL — so pane changes are lost on reload/share. The read side
(`initWplaceStateFromUrl`) already understands all three params.

## Decisions (user-confirmed)

- Scope: the 3 style selects only. Transparency toggle stays session-only.
- Approach: extend the existing URL writer (single choke point).
- **Default values are NOT persisted**: defaults are omitted, and stale params are
  deleted when a setting returns to default.

## Changes

### 1. Pure helper in `frontend/assets/map-styles.js`

```js
export function syncStyleParams(params, state = WplaceMapState) {
  if (state.basemapType !== 'vector') params.set('basemap', state.basemapType);
  else params.delete('basemap');
  if (state.strategy !== 'swap') params.set('layerswap', state.strategy);
  else params.delete('layerswap');
  if (state.supersampling !== 2) params.set('supersampling', String(state.supersampling));
  else params.delete('supersampling');
  return params;
}
```

Defaults (`vector`/`swap`/`2`) match `resetWplaceState`/`initWplaceStateFromUrl`.
Pure over `URLSearchParams`, so unit-testable in node with no DOM.

### 2. Write path in `frontend/index.html`

- `updateUrlWithMapView`: call `syncStyleParams(params, WplaceMapState)` after the
  existing `params.set` calls, before `history.replaceState`. Import it alongside
  the other `map-styles.js` imports.
- `applySettings`: call `updateUrlWithMapView(map)` (in try/catch, like every other
  call site) after updating state. `WplaceMapState` fields are set synchronously
  before the async `setWplaceVersion` resolves, so the URL reflects the new state
  immediately.

### 3. Read side

Unchanged. Absent params already parse to defaults, so omit-defaults round-trips
exactly: `?layerswap=overlay` → reload → overlay preselected; back to swap →
param deleted → reload → swap default.

## Tests (`frontend/assets/map-styles.test.js`)

New `describe('syncStyleParams')` suite:

1. Omits all three params for default state.
2. Writes non-defaults (`basemap=raster`, `layerswap=overlay`, `supersampling=4`).
3. Deletes stale params when state returns to default.
4. Preserves unrelated params (e.g. `lat`, `version`).
5. Round-trip: `syncStyleParams` output parses back via `initWplaceStateFromUrl` to the same state.

Plus `npm test` green and manual checks: change setting → param appears; reset to
default → param disappears; reload/share → state restored.

## Non-goals

- No transparency persistence, no `pushState`/history entries (stays `replaceState`),
  no changes to view/version params or the load-time parsing.
