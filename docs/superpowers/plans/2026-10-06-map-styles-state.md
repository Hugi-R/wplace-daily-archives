# Map styles state + layer-swap port Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Port the `db4897f` hidden-layer version swap into `frontend/assets/map-styles.js` behind a state object with old/new strategy switching.

**Architecture:** Singleton `WplaceMapState` in `map-styles.js` owns version, strategy, layer ids, transparency; `setWplaceVersion()` branches between `reload` (`setStyle`) and `swap` (hidden layer + `sourcedata` promote); `index.html` becomes thin calls. Tests use Node built-in `node:test` with a mock map, zero dependencies.

**Tech Stack:** Vanilla JS ESM, MapLibre GL JS 6.4.1 (browser only), Node 24 built-in `node:test` + `node:assert/strict` for tests.

## Global Constraints

- `strategy` is exactly `'swap'` (new, no flash, default) or `'reload'` (old `setStyle`).
- URL params are exactly `?layerswap=old|reload|new|swap` and `?basemap=raster|vector`; invalid values are ignored, defaults kept.
- No visible toggle UI — URL-param-only switching.
- The `wplace` layer def must be found by `id`, never by `layers[1]` (raster has it at index 1, vector last).
- Both raster and vector `wplace` paint must contain `"raster-opacity-transition": { duration: 0 }`.
- Zero new npm dependencies for the harness.
- A superseded pending swap detaches, cleans its own layer+source, and rejects its Promise.
- `removeLayer`/`removeSource` calls must be guarded by `getLayer`/`getSource` existence checks.

---

## File Structure

- `frontend/assets/map-styles.js` (modify, lines 1-9 head + 51-64 raster paint + 6110-6124 vector paint): owns `WplaceMapState`, `resetWplaceState`, `initWplaceStateFromUrl`, `getWplaceLayerDef`, `getWplaceOpacity`, `setWplaceTransparency`, `setWplaceVersion`. Single home for all version-swap logic.
- `frontend/assets/map-styles.test.js` (create): colocated unit tests with mock map; only test file in the repo.
- `frontend/index.html` (modify, lines 450, 660-662, 790-792, 824-832, 1130-1154, 1417-1429): thin calls into the state module; no swap logic remains.
- `frontend/package.json` (create): `{"type":"module","scripts":{"test":"node --test \"assets/**/*.test.js\""}}` so `npm test` works; no dependencies.
- `.github/workflows/frontend-js.yml` (create): CI runs `node --test "frontend/assets/**/*.test.js"` on Node 24.

---

### Task 1: State object, URL init, layer-def lookup, paint fix

**Files:**
- Modify: `frontend/assets/map-styles.js:1-14`
- Modify: `frontend/assets/map-styles.js:57-60` (raster paint)
- Modify: `frontend/assets/map-styles.js:6116-6119` (vector paint)
- Create: `frontend/assets/map-styles.test.js`
- Test: `frontend/assets/map-styles.test.js`

**Interfaces:**
- Consumes: existing `getMapStyle(version, basemapType)` from `map-styles.js:1-9`.
- Produces: `WplaceMapState`, `resetWplaceState()`, `initWplaceStateFromUrl(search)`, `getWplaceLayerDef(version, basemapType)` for Tasks 2-4 and `index.html`.

- [ ] **Step 1: Write the failing test**

Create `frontend/assets/map-styles.test.js` with this exact content:

```js
import { describe, it, beforeEach } from 'node:test';
import assert from 'node:assert/strict';
import {
  WplaceMapState,
  resetWplaceState,
  initWplaceStateFromUrl,
  getWplaceLayerDef,
  getMapStyle,
} from './map-styles.js';

describe('WplaceMapState', () => {
  beforeEach(() => { resetWplaceState(); });

  it('defaults to swap strategy and vector basemap', () => {
    assert.equal(WplaceMapState.strategy, 'swap');
    assert.equal(WplaceMapState.basemapType, 'vector');
    assert.equal(WplaceMapState.currentLayerId, 'wplace');
    assert.equal(WplaceMapState.pendingLayerId, null);
    assert.equal(WplaceMapState.layerCount, 0);
    assert.equal(WplaceMapState.isTransparent, false);
  });

  it('parses ?layerswap=old and ?basemap=raster', () => {
    initWplaceStateFromUrl('?layerswap=old&basemap=raster');
    assert.equal(WplaceMapState.strategy, 'reload');
    assert.equal(WplaceMapState.basemapType, 'raster');
  });

  it('ignores invalid params and keeps defaults', () => {
    initWplaceStateFromUrl('?layerswap=bogus&basemap=bogus');
    assert.equal(WplaceMapState.strategy, 'swap');
    assert.equal(WplaceMapState.basemapType, 'vector');
  });

  it('finds wplace layer by id for both basemaps', () => {
    const rasterDef = getWplaceLayerDef('v1', 'raster');
    const vectorDef = getWplaceLayerDef('v1', 'vector');
    assert.equal(rasterDef.id, 'wplace');
    assert.equal(vectorDef.id, 'wplace');
    assert.equal(vectorDef.source, 'wplace');
  });

  it('throws when wplace layer is missing', () => {
    assert.throws(() => getWplaceLayerDef('v1', 'nope'), /Invalid basemap/);
  });

  it('both basemaps include raster-opacity-transition', () => {
    for (const bm of ['raster', 'vector']) {
      const style = getMapStyle('v1', bm);
      const def = style.layers.find((l) => l.id === 'wplace');
      assert.ok(def, `wplace layer exists for ${bm}`);
      assert.deepEqual(def.paint['raster-opacity-transition'], { duration: 0 });
    }
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `node --test frontend/assets/map-styles.test.js`
Expected: FAIL with `SyntaxError` or `does not provide an export named 'WplaceMapState'` (the test file exists but the module has no state exports yet).

- [ ] **Step 3: Write minimal implementation**

In `frontend/assets/map-styles.js`, insert this block after line 9 (`}` closing `getMapStyle`), before the `getWplaceTileUrl` comment:

```js
export const WplaceMapState = {
  version: null,
  basemapType: 'vector',
  strategy: 'swap',
  currentLayerId: 'wplace',
  pendingLayerId: null,
  layerCount: 0,
  isTransparent: false,
};

export function resetWplaceState() {
  WplaceMapState.version = null;
  WplaceMapState.basemapType = 'vector';
  WplaceMapState.strategy = 'swap';
  WplaceMapState.currentLayerId = 'wplace';
  WplaceMapState.pendingLayerId = null;
  WplaceMapState.layerCount = 0;
  WplaceMapState.isTransparent = false;
}

export function initWplaceStateFromUrl(search) {
  WplaceMapState.strategy = 'swap';
  WplaceMapState.basemapType = 'vector';
  const s = search ?? (typeof window !== 'undefined' ? window.location.search : '');
  const p = new URLSearchParams(s);
  const ls = (p.get('layerswap') || '').toLowerCase();
  if (ls === 'old' || ls === 'reload') WplaceMapState.strategy = 'reload';
  else if (ls === 'new' || ls === 'swap') WplaceMapState.strategy = 'swap';
  const bm = (p.get('basemap') || '').toLowerCase();
  if (bm === 'raster' || bm === 'vector') WplaceMapState.basemapType = bm;
  return WplaceMapState;
}

export function getWplaceLayerDef(version, basemapType) {
  const style = getMapStyle(version, basemapType);
  const def = style.layers.find((l) => l.id === 'wplace');
  if (!def) throw new Error('wplace layer missing in style');
  return def;
}
```

Apply the paint fix in both places. Raster (`map-styles.js:57-60`), change:

```js
        paint: {
            "raster-fade-duration": 0,
            "raster-resampling": "nearest"
        }
```

to:

```js
        paint: {
            "raster-fade-duration": 0,
            "raster-opacity-transition": { duration: 0 },
            "raster-resampling": "nearest"
        }
```

Vector (`map-styles.js:6116-6119`), change:

```js
            "paint": {
            "raster-fade-duration": 0,
            "raster-resampling": "nearest"
            }
```

to:

```js
            "paint": {
            "raster-fade-duration": 0,
            "raster-opacity-transition": { duration: 0 },
            "raster-resampling": "nearest"
            }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `node --test frontend/assets/map-styles.test.js`
Expected: PASS, 6 passing.

- [ ] **Step 5: Commit**

```bash
git add frontend/assets/map-styles.js frontend/assets/map-styles.test.js
git commit -m "feat(frontend): add wplace map state, url init, layer-def lookup"
```

---

### Task 2: `setWplaceVersion` reload path + mock map

**Files:**
- Modify: `frontend/assets/map-styles.js` (append after `getWplaceLayerDef`)
- Modify: `frontend/assets/map-styles.test.js` (append new describe block)
- Test: `frontend/assets/map-styles.test.js`

**Interfaces:**
- Consumes: `WplaceMapState`, `resetWplaceState`, `getMapStyle` from Task 1.
- Produces: `setWplaceVersion(map, version, opts)` returning `Promise<void>` for Tasks 3-5.

- [ ] **Step 1: Write the failing test**

Append this exact block to `frontend/assets/map-styles.test.js`:

```js
import { setWplaceVersion } from './map-styles.js';

function createMockMap() {
  const layers = new Map();
  const sources = new Map();
  const handlers = { sourcedata: new Set(), error: new Set(), styledata: new Set() };
  const paint = new Map();
  return {
    _layers: layers, _sources: sources, _handlers: handlers, _paint: paint, _style: null,
    addSource(id, src) { sources.set(id, src); },
    addLayer(l) { layers.set(l.id, l); },
    removeLayer(id) { if (!layers.has(id)) throw new Error('layer not found: ' + id); layers.delete(id); },
    removeSource(id) { if (!sources.has(id)) throw new Error('source not found: ' + id); sources.delete(id); },
    getLayer(id) { return layers.get(id) ?? null; },
    getSource(id) { return sources.get(id) ?? null; },
    on(ev, fn) { handlers[ev]?.add(fn); return this; },
    off(ev, fn) { handlers[ev]?.delete(fn); return this; },
    once(ev, fn) { const w = (...a) => { this.off(ev, w); fn(...a); }; this.on(ev, w); return this; },
    fire(ev, data) { [...(handlers[ev] ?? [])].forEach((fn) => fn(data)); },
    isSourceLoaded(id) { return sources.has(id); },
    getPaintProperty(layer, prop) { return paint.has(layer + ':' + prop) ? paint.get(layer + ':' + prop) : undefined; },
    setPaintProperty(layer, prop, v) { paint.set(layer + ':' + prop, v); },
    setStyle(style) { this._style = style; },
  };
}

describe('setWplaceVersion reload', () => {
  beforeEach(() => { resetWplaceState(); });

  it('uses setStyle and resolves on styledata', async () => {
    const map = createMockMap();
    WplaceMapState.strategy = 'reload';
    const p = setWplaceVersion(map, 'v42', { basemapType: 'raster' });
    assert.ok(map._style, 'style was set');
    const wplaceSrc = map._style.sources.wplace;
    assert.ok(wplaceSrc.tiles[0].includes('v42'));
    assert.equal(WplaceMapState.version, 'v42');
    map.fire('styledata', {});
    await p;
    assert.equal(WplaceMapState.currentLayerId, 'wplace');
    assert.equal(WplaceMapState.pendingLayerId, null);
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `node --test frontend/assets/map-styles.test.js`
Expected: FAIL with `does not provide an export named 'setWplaceVersion'`.

- [ ] **Step 3: Write minimal implementation**

Append this exact code to `frontend/assets/map-styles.js` (after `getWplaceLayerDef`):

```js
export function setWplaceVersion(map, version, opts = {}) {
  const basemapType = opts.basemapType ?? WplaceMapState.basemapType;
  const strategy = opts.strategy ?? WplaceMapState.strategy;
  WplaceMapState.version = version;
  WplaceMapState.basemapType = basemapType;
  if (strategy === 'reload') {
    map.setStyle(getMapStyle(version, basemapType));
    return new Promise((resolve) => {
      map.once('styledata', () => {
        WplaceMapState.currentLayerId = 'wplace';
        WplaceMapState.pendingLayerId = null;
        resolve();
      });
    });
  }
  return setWplaceVersionSwap(map, version, basemapType);
}

function setWplaceVersionSwap(map, version, basemapType) {
  if (WplaceMapState.pendingLayerId) {
    const stale = WplaceMapState.pendingLayerId;
    WplaceMapState.pendingLayerId = null;
    try { if (map.getLayer(stale)) map.removeLayer(stale); } catch {}
    try { if (map.getSource(stale)) map.removeSource(stale); } catch {}
  }
  const id = `wplace-${++WplaceMapState.layerCount}`;
  WplaceMapState.pendingLayerId = id;
  const style = getMapStyle(version, basemapType);
  const def = style.layers.find((l) => l.id === 'wplace');
  map.addSource(id, style.sources.wplace);
  map.addLayer({ ...def, id, source: id, paint: { ...def.paint, 'raster-opacity': 0 } });
  return new Promise((resolve, reject) => {
    const swap = function (e) {
      if (WplaceMapState.pendingLayerId !== id) {
        map.off('sourcedata', swap);
        map.off('error', swap);
        reject(new Error('superseded'));
        return;
      }
      const isErr = !e || e.type === 'error';
      const srcOk = !e || e.sourceId === undefined || e.sourceId === id;
      const dataOk = isErr || !!e.tile;
      if (!srcOk || !dataOk) return;
      if (!isErr && !map.isSourceLoaded(id)) return;
      map.off('sourcedata', swap);
      map.off('error', swap);
      let opacity = WplaceMapState.isTransparent ? 0.3 : 1;
      try {
        const v = map.getPaintProperty(WplaceMapState.currentLayerId, 'raster-opacity');
        if (v !== undefined && v !== null) opacity = v;
      } catch {}
      try { map.setPaintProperty(id, 'raster-opacity', opacity); } catch {}
      const old = WplaceMapState.currentLayerId;
      WplaceMapState.currentLayerId = id;
      WplaceMapState.pendingLayerId = null;
      try { if (map.getLayer(old)) map.removeLayer(old); } catch {}
      try { if (map.getSource(old)) map.removeSource(old); } catch {}
      resolve();
    };
    map.on('sourcedata', swap);
    map.on('error', swap);
  });
}
```

Note: the swap half is included now so the module parses as one unit; its behavior is tested in Task 3.

- [ ] **Step 4: Run test to verify it passes**

Run: `node --test frontend/assets/map-styles.test.js`
Expected: PASS, 7 passing (6 old + 1 new). Ignore untested swap half for now.

- [ ] **Step 5: Commit**

```bash
git add frontend/assets/map-styles.js frontend/assets/map-styles.test.js
git commit -m "feat(frontend): add setWplaceVersion with reload path"
```

---

### Task 3: Swap path behavior (core port of db4897f)

**Files:**
- Modify: `frontend/assets/map-styles.test.js` (append describe block; no production change expected)
- Test: `frontend/assets/map-styles.test.js`

**Interfaces:**
- Consumes: `setWplaceVersion`, `WplaceMapState`, `resetWplaceState` from Tasks 1-2; mock `createMockMap` from Task 2.
- Produces: verified swap semantics for Task 5 integration.

- [ ] **Step 1: Write the failing test**

Append this exact block to `frontend/assets/map-styles.test.js`:

```js
describe('setWplaceVersion swap', () => {
  beforeEach(() => { resetWplaceState(); });

  function seededMap() {
    const map = createMockMap();
    map.addSource('wplace', { type: 'raster', tiles: ['old'] });
    map.addLayer({ id: 'wplace', type: 'raster', source: 'wplace', paint: {} });
    map.setPaintProperty('wplace', 'raster-opacity', 1);
    return map;
  }

  it('adds hidden layer and promotes on sourcedata', async () => {
    const map = seededMap();
    const p = setWplaceVersion(map, 'v99', { basemapType: 'raster' });
    assert.equal(WplaceMapState.pendingLayerId, 'wplace-1');
    assert.equal(map.getPaintProperty('wplace-1', 'raster-opacity'), undefined);
    const hidden = map.getLayer('wplace-1');
    assert.equal(hidden.paint['raster-opacity'], 0);
    map.fire('sourcedata', { sourceId: 'wplace-1', tile: {} });
    await p;
    assert.equal(WplaceMapState.currentLayerId, 'wplace-1');
    assert.equal(WplaceMapState.pendingLayerId, null);
    assert.equal(map.getLayer('wplace'), null);
    assert.equal(map.getPaintProperty('wplace-1', 'raster-opacity'), 1);
  });

  it('ignores other sources and unloaded sources', async () => {
    const map = seededMap();
    let settled = false;
    const p = setWplaceVersion(map, 'v99', { basemapType: 'raster' }).then(() => { settled = true; });
    map.fire('sourcedata', { sourceId: 'other', tile: {} });
    assert.equal(settled, false);
    assert.equal(WplaceMapState.pendingLayerId, 'wplace-1');
    map.fire('sourcedata', { sourceId: 'wplace-1', tile: {} });
    await p;
    assert.equal(settled, true);
  });

  it('cancels prior pending on rapid calls', async () => {
    const map = seededMap();
    const p1 = setWplaceVersion(map, 'v1', { basemapType: 'raster' });
    const p2 = setWplaceVersion(map, 'v2', { basemapType: 'raster' });
    assert.equal(map.getLayer('wplace-1'), null);
    assert.equal(WplaceMapState.pendingLayerId, 'wplace-2');
    await assert.rejects(p1, /superseded/);
    map.fire('sourcedata', { sourceId: 'wplace-1', tile: {} });
    assert.equal(WplaceMapState.currentLayerId, 'wplace');
    map.fire('sourcedata', { sourceId: 'wplace-2', tile: {} });
    await p2;
    assert.equal(WplaceMapState.currentLayerId, 'wplace-2');
  });

  it('promotes on error so the promise always settles', async () => {
    const map = seededMap();
    const p = setWplaceVersion(map, 'vX', { basemapType: 'raster' });
    map.fire('error', { type: 'error', sourceId: 'wplace-1' });
    await p;
    assert.equal(WplaceMapState.currentLayerId, 'wplace-1');
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `node --test frontend/assets/map-styles.test.js`
Expected: FAIL — at least one swap test fails against the Task 2 implementation (e.g. hidden-layer paint shape or error-promote path). If all pass, strengthen the `ignores other sources` case by firing `{ sourceId: 'wplace-1' }` without `tile` and confirm it does not settle, then re-run to see the failure.

- [ ] **Step 3: Write minimal implementation**

Fix `setWplaceVersionSwap` in `frontend/assets/map-styles.js` until the Task 3 tests pass. Do not change the public signature `setWplaceVersion(map, version, opts)`. Typical fix is the guard block:

```js
      const isErr = !e || e.type === 'error';
      const srcOk = !e || e.sourceId === undefined || e.sourceId === id;
      const dataOk = isErr || !!e.tile;
      if (!srcOk || !dataOk) return;
      if (!isErr && !map.isSourceLoaded(id)) return;
```

Keep everything else from Task 2 unchanged.

- [ ] **Step 4: Run test to verify it passes**

Run: `node --test frontend/assets/map-styles.test.js`
Expected: PASS, 11 passing.

- [ ] **Step 5: Commit**

```bash
git add frontend/assets/map-styles.js frontend/assets/map-styles.test.js
git commit -m "feat(frontend): verify hidden-layer swap semantics"
```

---

### Task 4: Transparency ownership + opacity preservation

**Files:**
- Modify: `frontend/assets/map-styles.js` (append `setWplaceTransparency` + `getWplaceOpacity`)
- Modify: `frontend/assets/map-styles.test.js` (append describe block)
- Test: `frontend/assets/map-styles.test.js`

**Interfaces:**
- Consumes: `WplaceMapState`, `setWplaceVersion` from Tasks 1-3.
- Produces: `setWplaceTransparency(map, transparent)`, `getWplaceOpacity(map)` for Task 5.

- [ ] **Step 1: Write the failing test**

Append this exact block to `frontend/assets/map-styles.test.js`:

```js
import { setWplaceTransparency, getWplaceOpacity } from './map-styles.js';

describe('transparency', () => {
  beforeEach(() => { resetWplaceState(); });

  it('toggles current layer opacity and records flag', () => {
    const map = createMockMap();
    map.addSource('wplace', { type: 'raster', tiles: ['old'] });
    map.addLayer({ id: 'wplace', type: 'raster', source: 'wplace', paint: {} });
    setWplaceTransparency(map, true);
    assert.equal(WplaceMapState.isTransparent, true);
    assert.equal(map.getPaintProperty('wplace', 'raster-opacity'), 0.3);
    setWplaceTransparency(map, false);
    assert.equal(map.getPaintProperty('wplace', 'raster-opacity'), 1);
  });

  it('preserves transparent opacity across swap', async () => {
    const map = createMockMap();
    map.addSource('wplace', { type: 'raster', tiles: ['old'] });
    map.addLayer({ id: 'wplace', type: 'raster', source: 'wplace', paint: {} });
    setWplaceTransparency(map, true);
    const p = setWplaceVersion(map, 'vT', { basemapType: 'raster' });
    map.fire('sourcedata', { sourceId: 'wplace-1', tile: {} });
    await p;
    assert.equal(map.getPaintProperty('wplace-1', 'raster-opacity'), 0.3);
    assert.equal(getWplaceOpacity(map), 0.3);
  });

  it('mid-load toggle sticks after promote', async () => {
    const map = createMockMap();
    map.addSource('wplace', { type: 'raster', tiles: ['old'] });
    map.addLayer({ id: 'wplace', type: 'raster', source: 'wplace', paint: {} });
    map.setPaintProperty('wplace', 'raster-opacity', 1);
    const p = setWplaceVersion(map, 'vT', { basemapType: 'raster' });
    setWplaceTransparency(map, true);
    assert.equal(map.getPaintProperty('wplace-1', 'raster-opacity'), 0.3);
    map.fire('sourcedata', { sourceId: 'wplace-1', tile: {} });
    await p;
    assert.equal(map.getPaintProperty('wplace-1', 'raster-opacity'), 0.3);
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `node --test frontend/assets/map-styles.test.js`
Expected: FAIL with `does not provide an export named 'setWplaceTransparency'`.

- [ ] **Step 3: Write minimal implementation**

Append this exact code to `frontend/assets/map-styles.js`:

```js
export function getWplaceOpacity(map) {
  try {
    const v = map.getPaintProperty(WplaceMapState.currentLayerId, 'raster-opacity');
    if (v !== undefined && v !== null) return v;
  } catch {}
  return WplaceMapState.isTransparent ? 0.3 : 1;
}

export function setWplaceTransparency(map, transparent) {
  WplaceMapState.isTransparent = transparent;
  const opacity = transparent ? 0.3 : 1;
  try {
    if (map.getLayer(WplaceMapState.currentLayerId)) {
      map.setPaintProperty(WplaceMapState.currentLayerId, 'raster-opacity', opacity);
    }
  } catch {}
  try {
    const p = WplaceMapState.pendingLayerId;
    if (p && map.getLayer(p)) map.setPaintProperty(p, 'raster-opacity', opacity);
  } catch {}
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `node --test frontend/assets/map-styles.test.js`
Expected: PASS, 14 passing.

- [ ] **Step 5: Commit**

```bash
git add frontend/assets/map-styles.js frontend/assets/map-styles.test.js
git commit -m "feat(frontend): state-owned transparency with swap preservation"
```

---

### Task 5: index.html integration (thin calls, no swap logic)

**Files:**
- Modify: `frontend/index.html:450` (import)
- Modify: `frontend/index.html:660-662` (basemap init)
- Modify: `frontend/index.html:790-792` (setMapVersion wrapper)
- Modify: `frontend/index.html:824-832` (slider handler)
- Modify: `frontend/index.html:1130-1154` (export version fns)
- Modify: `frontend/index.html:1417-1429` (transparency toggle)

**Interfaces:**
- Consumes: `WplaceMapState`, `initWplaceStateFromUrl`, `setWplaceVersion`, `setWplaceTransparency` from Tasks 1-4.
- Produces: working page in both strategies; no new exports.

- [ ] **Step 1: Write the failing check (manual)**

Open the page and drag the version slider fast. Current behavior: white flash between versions (plain `setStyle`), transparency toggle addresses hardcoded `'wplace'` only. This is the failure to fix — no automated test covers `index.html`.

- [ ] **Step 2: Run check to verify it fails**

Run: serve locally and load `http://localhost:8000/?layerswap=swap`, drag slider.
Expected: FAIL — flash visible, confirming old path still active.

- [ ] **Step 3: Write minimal implementation**

Apply these exact edits:

1. Import (`index.html:450`), change:

```js
    import {getMapStyle} from '/assets/map-styles.js';
```

to:

```js
    import {getMapStyle, WplaceMapState, initWplaceStateFromUrl, setWplaceVersion, setWplaceTransparency} from '/assets/map-styles.js';
```

2. Basemap init (`index.html:660-662`), change:

```js
    let wplaceVersion = WPLACE_VERSIONS[versionSlider.value].version;
    let basemapType = 'vector'; // Can be vector or raster
```

to:

```js
    let wplaceVersion = WPLACE_VERSIONS[versionSlider.value].version;
    initWplaceStateFromUrl(window.location.search);
    let basemapType = WplaceMapState.basemapType;
    WplaceMapState.version = wplaceVersion;
```

3. Wrapper (`index.html:790-792`), change:

```js
    function setMapVersion(version) {
      map.setStyle(getMapStyle(version, basemapType));
    }
```

to:

```js
    function setMapVersion(version) {
      return setWplaceVersion(map, version);
    }
```

4. Slider handler (`index.html:824-832`), change:

```js
    versionSlider.addEventListener('input', function(e) {
      const idx = parseInt(e.target.value);
      wplaceVersion = WPLACE_VERSIONS[idx].version;
      updateVersionLabel(idx);
      setMapVersion(wplaceVersion);
      // after the style is applied, update zoom display and the url (so version is saved)
      map.once('styledata', function() { updateZoom(); try { updateUrlWithMapView(map); } catch (e) { console.error(e)} });
    });
```

to:

```js
    versionSlider.addEventListener('input', function(e) {
      const idx = parseInt(e.target.value);
      wplaceVersion = WPLACE_VERSIONS[idx].version;
      updateVersionLabel(idx);
      setMapVersion(wplaceVersion).catch(() => {}).finally(function() {
        updateZoom();
        try { updateUrlWithMapView(map); } catch (err) { console.error(err); }
      });
    });
```

5. Export fns (`index.html:1130-1154`), change:

```js
    function switchMapToVersion(version) {
      if (exportParams.savedVersion === null) exportParams.savedVersion = wplaceVersion;
      wplaceVersion = version;
      setMapVersion(version);
      const idx = WPLACE_VERSIONS.findIndex(v => v.version === version);
      if (idx >= 0) {
        versionSlider.value = idx;
        updateVersionLabel(idx);
      }
      try { updateUrlWithMapView(map); } catch (e) { console.error(e); }
      map.once('styledata', drawExportOverlay);
    }

    function restoreSavedMapVersion() {
      if (exportParams.savedVersion === null) return;
      wplaceVersion = exportParams.savedVersion;
      exportParams.savedVersion = null;
      setMapVersion(wplaceVersion);
```

to:

```js
    function switchMapToVersion(version) {
      if (exportParams.savedVersion === null) exportParams.savedVersion = wplaceVersion;
      wplaceVersion = version;
      setMapVersion(version).catch(() => {}).finally(drawExportOverlay);
      const idx = WPLACE_VERSIONS.findIndex(v => v.version === version);
      if (idx >= 0) {
        versionSlider.value = idx;
        updateVersionLabel(idx);
      }
      try { updateUrlWithMapView(map); } catch (e) { console.error(e); }
    }

    function restoreSavedMapVersion() {
      if (exportParams.savedVersion === null) return;
      wplaceVersion = exportParams.savedVersion;
      exportParams.savedVersion = null;
      setMapVersion(wplaceVersion).catch(() => {});
```

6. Transparency (`index.html:1417-1429`), change:

```js
    document.addEventListener('DOMContentLoaded', function() {
      const transparencyButton = document.getElementById('toggle-transparency');
      let isTransparent = false;

      transparencyButton.addEventListener('click', function() {
        const wplaceLayer = map.getLayer('wplace');
        if (wplaceLayer) {
          isTransparent = !isTransparent;
          map.setPaintProperty('wplace', 'raster-opacity', isTransparent ? 0.3 : 1);
          transparencyButton.classList.toggle('toggled', isTransparent);
        }
      });
    });
```

to:

```js
    document.addEventListener('DOMContentLoaded', function() {
      const transparencyButton = document.getElementById('toggle-transparency');

      transparencyButton.addEventListener('click', function() {
        if (!map.getLayer(WplaceMapState.currentLayerId)) return;
        setWplaceTransparency(map, !WplaceMapState.isTransparent);
        transparencyButton.classList.toggle('toggled', WplaceMapState.isTransparent);
      });
    });
```

- [ ] **Step 4: Run checks to verify it passes**

Run: `node --test frontend/assets/map-styles.test.js` (unit suite still green) plus manual:
1. Drag slider fast → no white flash, final art matches slider.
2. Reload with `?layerswap=old` → flash returns (old path); remove param → no flash.
3. Toggle transparency mid-load → opacity sticks after swap.
4. Export APNG end-date change + panel close restores version; rectangle redraws.
5. Console shows no `removeLayer` exceptions.
Expected: all five pass.

- [ ] **Step 5: Commit**

```bash
git add frontend/index.html
git commit -m "feat(frontend): route version switches through wplace state"
```

---

### Task 6: package.json + CI + full suite

**Files:**
- Create: `frontend/package.json`
- Create: `.github/workflows/frontend-js.yml`
- Test: `frontend/assets/map-styles.test.js`

**Interfaces:**
- Consumes: all Tasks 1-5.
- Produces: `npm test` entrypoint and CI gate.

- [ ] **Step 1: Write the failing test**

Run: `npm test --prefix frontend`
Expected: FAIL with `Missing script: "test"` (no `package.json` yet).

- [ ] **Step 2: Run test to verify it fails**

Run: `npm test --prefix frontend 2>&1 | head -n 5`
Expected: `npm error Missing script`.

- [ ] **Step 3: Write minimal implementation**

Create `frontend/package.json` with exact content:

```json
{
  "name": "wplace-frontend",
  "private": true,
  "type": "module",
  "scripts": {
    "test": "node --test \"assets/**/*.test.js\""
  }
}
```

Create `.github/workflows/frontend-js.yml` with exact content:

```yaml
name: Frontend JS tests
on: [push, pull_request]
jobs:
  js-test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-node@v4
        with:
          node-version: '24'
      - run: node --test "frontend/assets/**/*.test.js"
```

- [ ] **Step 4: Run test to verify it passes**

Run: `npm test --prefix frontend`
Expected: PASS, 14 tests passing. Then run: `node --test "frontend/assets/**/*.test.js"`
Expected: PASS, same count.

- [ ] **Step 5: Commit**

```bash
git add frontend/package.json .github/workflows/frontend-js.yml
git commit -m "chore(frontend): add node test entrypoint and CI"
```

---

## Self-Review

1. **Spec coverage:** state object + strategy flag (Task 1), `?layerswap`/`?basemap` init (Task 1), by-id layer lookup (Task 1), paint fix both basemaps (Task 1), swap port with pending-cancel + error-promote (Tasks 2-3), transparency ownership + mid-load stick (Task 4), slider/export/transparency rewiring + `styledata` removal (Task 5), committed harness + CI (Task 6). No spec section left without a task.
2. **Placeholder scan:** no TBD/TODO/"similar to"/"appropriate handling" — every step shows exact code and exact commands with expected output.
3. **Type consistency:** `setWplaceVersion(map, version, opts)` returns `Promise<void>` in all tasks; state fields `strategy`, `basemapType`, `currentLayerId`, `pendingLayerId`, `layerCount`, `isTransparent`, `version` spelled identically; `setWplaceTransparency(map, transparent)` and `getWplaceOpacity(map)` signatures stable; mock map method names match production calls (`getLayer`, `getSource`, `on`/`off`/`once`, `isSourceLoaded`, `getPaintProperty`/`setPaintProperty`, `setStyle`, `addSource`/`addLayer`/`removeLayer`/`removeSource`).
