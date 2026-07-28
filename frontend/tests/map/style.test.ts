// tests/map/style.test.ts
import { describe, it, expect } from 'vitest';
import { getMapStyle } from '../../src/map/style';

describe('style', () => {
  it('returns valid MapLibre style with osm and wplace sources', () => {
    const style = getMapStyle(12345);
    expect(style.version).toBe(8);
    expect(style.sources.osm.type).toBe('raster');
    expect(style.sources.wplace.type).toBe('raster');
    expect(style.layers).toHaveLength(2);
  });

  it('includes version in wplace tile URL', () => {
    const style = getMapStyle(99999);
    expect(style.sources.wplace.tiles[0]).toContain('99999');
  });

  it('includes layer in wplace tile URL', () => {
    const style = getMapStyle(12345, 'alternate');
    expect(style.sources.wplace.tiles[0]).toContain('alternate');
  });

  it('uses default layer "tiles" when not specified', () => {
    const style = getMapStyle(12345);
    expect(style.sources.wplace.tiles[0]).toContain('tiles');
  });

  it('wplace layer has nearest resampling', () => {
    const style = getMapStyle(12345);
    const wplaceLayer = style.layers.find((l) => l.id === 'wplace');
    expect(wplaceLayer?.paint?.['raster-resampling']).toBe('nearest');
  });
});
