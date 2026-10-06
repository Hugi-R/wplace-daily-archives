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

describe('setWplaceVersion swap', () => {
  beforeEach(() => { resetWplaceState(); WplaceMapState.basemapType = 'raster'; });

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
import { setWplaceTransparency, getWplaceOpacity } from './map-styles.js';

describe('transparency', () => {
  beforeEach(() => { resetWplaceState(); WplaceMapState.basemapType = 'raster'; });

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

  it('reapplies transparency after reload strategy styledata', async () => {
    const map = createMockMap();
    map.addSource('wplace', { type: 'raster', tiles: ['old'] });
    map.addLayer({ id: 'wplace', type: 'raster', source: 'wplace', paint: {} });
    // Real setStyle discards all layers/paint; simulate by clearing paint on setStyle.
    const origSetStyle = map.setStyle.bind(map);
    map.setStyle = (s) => { map._paint.clear(); origSetStyle(s); };
    WplaceMapState.strategy = 'reload';
    setWplaceTransparency(map, true);
    const p = setWplaceVersion(map, 'vR', { basemapType: 'raster' });
    map.fire('styledata', {});
    await p;
    assert.equal(map.getPaintProperty('wplace', 'raster-opacity'), 0.3);
  });
});
