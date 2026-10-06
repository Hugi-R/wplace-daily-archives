# Port layer-swap fix to map-styles.js with state object — Design

## Background

Commit `db4897f` (`fix/slider-flash` branch, "keep previous art visible while changing
version") added hidden-layer swapping to the old monolithic `frontend/index.html`:
load the new version in a hidden layer at opacity 0, swap it in on `sourcedata`,
avoiding the white flash of `map.setStyle()`. The current `map-overhaul` branch
(`0346e5f`, vector basemap + `frontend/assets/map-styles.js`) does not contain
that fix — `setMapVersion()` is a plain `map.setStyle()` and flashes.

## Goal

Port `db4897f` to `frontend/assets/map-styles.js`, make old (full reload) vs new
(hidden-layer swap) switchable at runtime, and refactor the module around a state
object. Add a committed JS test harness starting at `map-styles.js`.

## Architecture (Option A, approved)

All version-swap logic moves from inline `index.html` into `map-styles.js`.
`index.html` keeps thin calls only.

### State object (singleton, exported from `map-styles.js`)

```js
export const WplaceMapState = {
  version: null,            // current wplace version string
  basemapType: 'vector',    // 'vector' | 'raster'
  strategy: 'swap',         // 'swap' (new, no flash) | 'reload' (old setStyle)
  currentLayerId: 'wplace', // visible wplace layer id
  pendingLayerId: null,     // loading layer id or null
  layerCount: 0,            // monotonic counter for wplace-N ids
  isTransparent: false,     // desired art opacity flag
};
export function resetWplaceState()          // restore defaults (tests/init)
export function initWplaceStateFromUrl(search) // ?layerswap=old|reload|new|swap, ?basemap=
```

### Pure helpers (kept, fixed)

- `getMapStyle(version, basemapType)` — unchanged signature. Adds
  `"raster-opacity-transition": { duration: 0 }` to the `wplace` paint in
  **both** raster and vector styles (ports `db4897f` hunk 1, currently missing).
- New `getWplaceLayerDef(version, basemapType)` — returns the `wplace` layer
  definition found **by `id`**, not `layers[1]`. Required: raster has it at
  index 1, vector has it last (~112 layers).

### Effectful API

- `setWplaceVersion(map, version, { basemapType }?)` — branches on
  `state.strategy`, updates `state.version`. Returns a Promise resolving when
  the new art is visible (so callers can redraw overlays without assuming
  `styledata`).
- `setWplaceTransparency(map, transparent)` — sets `state.isTransparent`,
  applies to current + pending layers.
- Internal `getWplaceOpacity(map)` — reads current layer paint so a swap
  preserves 0.3 vs 1.

## Data flow / index.html integration

- **Init:** `index.html` calls `initWplaceStateFromUrl(window.location.search)`
  before map creation, then
  `new maplibregl.Map({ style: getMapStyle(state.version, state.basemapType) })`.
  `?layerswap=old|reload` → `strategy='reload'`; `?layerswap=new|swap` or
  absent → `'swap'`. `?basemap=` overrides `basemapType`. No visible UI
  (URL-param-only, as decided).
- **`reload` (old):** `map.setStyle(getMapStyle(version, basemapType))`,
  resolve on `styledata`. Identical to today's `setMapVersion`.
- **`swap` (new, port of `db4897f`):** if `pendingLayerId` exists, remove its
  layer+source (guarded by `getLayer`/`getSource`). Create `id =
  wplace-${++layerCount}`; `addSource(id, sources.wplace)`;
  `addLayer({ ...wplaceDef, id, source: id,
  paint: { ...def.paint, 'raster-opacity': 0 } })`. On `sourcedata`/`error`
  for `sourceId === id` with `tile` or `error` + `isSourceLoaded(id)`: copy
  opacity from old layer (`getPaintProperty(current,'raster-opacity') ??
  (isTransparent ? 0.3 : 1)`), set on new, remove old layer+source,
  `currentLayerId = id`, `pendingLayerId = null`, resolve. A superseded
  pending (a newer call replaced it) detaches its listener, removes its own
  layer+source, and rejects its Promise. Rapid slider drags cancel the prior
  pending (as in the original commit). A basemap change forces the `reload` path.
- **Call-site replacements:** slider `input`, `switchMapToVersion`,
  `restoreSavedMapVersion` all go through `setWplaceVersion(map, v)` with
  `.then(...)` for URL update / `drawExportOverlay` (the swap path never fires
  `styledata`, so `map.once('styledata', drawExportOverlay)` must go).
  Transparency click goes through `setWplaceTransparency` (fixes today's
  hardcoded `'wplace'` id vs dynamic `wplace-N` ids).

## Error handling

- Superseded loads detach (`off`) and never touch the map.
- `removeLayer`/`removeSource` guarded by existence checks (MapLibre throws on
  unknown ids; `error`-event swaps can race with cancels).
- Source that never loads still promotes on `error` (as in `db4897f`) so the
  map can't stick on stale art; the Promise always settles.
- Transparency toggled mid-load writes to both current and pending layers plus
  `state.isTransparent`, so promotion can't clobber it.
- Invalid `?layerswap=` / `?basemap=` values are ignored (defaults kept);
  `getMapStyle` still throws on invalid basemap (existing behavior).

## Testing (Option A harness, approved)

- **Committed unit tests:** `frontend/assets/map-styles.test.js` using Node
  built-in `node:test` + `node:assert/strict`, zero dependencies. Fake `map`
  mock (`addSource/addLayer/removeLayer/getLayer/getSource/on/off/
  isSourceLoaded/getPaintProperty/setPaintProperty`) plus a listener-fire
  helper. Cases: strategy branch, wplace-def-by-id for both basemaps, pending
  cancel on rapid calls, opacity preserved, mid-load transparency sticks,
  invalid URL params ignored, Promise settles on `error`, guarded removals
  don't throw. Run: `node --test frontend/assets/`. Include a minimal
  `frontend/package.json` (`{"type":"module","scripts":{"test":...}}`) so
  `npm test` works. New CI workflow `.github/workflows/frontend-js.yml`
  (`setup-node` + that command).
- **Manual (required):** slider drag → no flash; `?layerswap=old` → flash
  returns; transparency mid-load sticks; export APNG end-date switch +
  restore works; console clean.

## Out of scope

Visible toggle UI, basemap-switcher UI, multi-map support, Playwright E2E
(YAGNI until the unit layer exists).
