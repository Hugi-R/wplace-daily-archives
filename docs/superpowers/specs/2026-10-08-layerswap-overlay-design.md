# layerswap=overlay design

Date: 2026-10-08
Status: approved (pending spec review)
Scope: `frontend/assets/map-styles.js` + `frontend/assets/map-styles.test.js`

## Context

`map-styles.js` supports two `layerswap` strategies via `?layerswap=`:

- `swap` (default): adds the new `wplace-N` layer hidden (`raster-opacity: 0`) on
  top, keeps the old layer visible, then instant-swaps opacity and removes the
  old layer/source once the new source loads.
- `reload`: full `map.setStyle()` reset, resolves on `styledata`.

Request: a third option that draws the new layer visibly on top of the old one
and removes the old layer once the new one is loaded (progressive cover instead
of instant cut).

## Decisions (user-confirmed)

- New layer is **visible immediately** on top at current opacity; new tiles
  progressively cover the old; old layer/source removed after load.
- Name: **`overlay`** (`?layerswap=overlay`). Rejected: `cover` (less standard),
  `progressive` (long, overlaps progressive-loading jargon).
- Error handling: **promote anyway** on source error, matching `swap` (promise
  always settles; no keep-old fallback).
- Reuse: single shared swap path with **initial opacity as a parameter**
  (user suggestion) instead of a duplicated function.

## Behavior

- `overlay`: `addSource(id)` + `addLayer({ ...def, id, source: id })` appended on
  top (wplace is already the topmost layer in both raster and vector styles, so
  plain append stacks the new layer above the old) with
  `raster-opacity` = current opacity (`1`, or `0.3` when `isTransparent`).
- `swap`: identical except initial `raster-opacity = 0`.
- Promotion (shared): on `sourcedata` with matching `sourceId` + tile and
  `map.isSourceLoaded(id)`, or on `error` for that source: re-read current
  opacity (covers mid-load transparency toggles), apply to new layer, set
  `currentLayerId = id`, clear `pendingLayerId`, remove old layer + source,
  resolve.
- Supersede (shared, unchanged): a new call while `pendingLayerId` is set
  removes the stale pending layer/source, unsubscribes prior listeners, rejects
  prior promise with `superseded`.

## Wiring

- `initWplaceStateFromUrl`: accept `overlay` alongside `swap`/`reload`
  (`?layerswap=overlay`); invalid values keep the `swap` default.
- `setWplaceVersion`: `strategy === 'overlay'` routes to the shared layered path
  with `overlay` routes with initial opacity = `getWplaceOpacity(map)`; `swap` routes with `0`; basemap or
  supersampling changes still force the `reload` path for any strategy.
- `setWplaceTransparency` already targets both current and pending layers, so
  mid-load toggles keep working for `overlay` with no extra code.

## Tests (`map-styles.test.js`)

Extend the existing mock-map suites:

1. URL parsing accepts `?layerswap=overlay`.
2. Overlay adds the new layer with current opacity visible immediately
   (not `0`), old layer still present until load.
3. Promotes on `sourcedata` (old removed, `currentLayerId` advanced).
4. Rapid calls cancel prior (`superseded`, stale layer removed).
5. Promotes on `error`.
6. Transparency (`0.3`) preserved on the overlay layer across promote.

## Non-goals

- No crossfade animation, no keep-old-on-error fallback, no changes to the
  `reload` path or tile URL/style builders.
