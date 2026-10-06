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
