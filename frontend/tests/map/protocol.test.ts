// tests/map/protocol.test.ts
import { describe, it, expect } from 'vitest';
import { parseTileUrl, getCacheKey } from '../../src/map/protocol';

describe('protocol', () => {
  it('parseTileUrl extracts layer, version, z, x, y', () => {
    const parsed = parseTileUrl('merged://tiles/tiles/12345/5/10/20.png');
    expect(parsed).toEqual({
      layer: 'tiles',
      version: 12345,
      z: 5,
      x: 10,
      y: 20,
    });
  });

  it('parseTileUrl handles alternate layer', () => {
    const parsed = parseTileUrl('merged://tiles/alternate/999/3/4/5.png');
    expect(parsed.layer).toBe('alternate');
    expect(parsed.version).toBe(999);
  });

  it('parseTileUrl throws on invalid URL', () => {
    expect(() => parseTileUrl('merged://invalid')).toThrow('Invalid tile URL');
  });

  it('getCacheKey includes layer segment', () => {
    const key = getCacheKey('tiles', 12345, 5, 10, 20);
    expect(key).toBe('tile-tiles-12345-5-10-20');
  });

  it('getCacheKey differs for different layers', () => {
    const key1 = getCacheKey('tiles', 12345, 5, 10, 20);
    const key2 = getCacheKey('alternate', 12345, 5, 10, 20);
    expect(key1).not.toBe(key2);
  });
});
