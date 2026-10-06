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
