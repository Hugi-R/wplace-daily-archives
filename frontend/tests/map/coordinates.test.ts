// tests/map/coordinates.test.ts
import { describe, it, expect } from 'vitest';
import { lng2tileX, lat2tileY, tile2lng, tile2lat, getTileCoords } from '../../src/map/coordinates';

describe('coordinates', () => {
  it('lng2tileX converts longitude to tile x at zoom 0', () => {
    expect(lng2tileX(0, 0)).toBe(0);
    expect(lng2tileX(-180, 0)).toBe(0);
    expect(lng2tileX(179.9999, 0)).toBe(0); // only one tile at zoom 0
  });

  it('lng2tileX at zoom 2', () => {
    expect(lng2tileX(-180, 2)).toBe(0);
    expect(lng2tileX(-90, 2)).toBe(1);
    expect(lng2tileX(0, 2)).toBe(2);
    expect(lng2tileX(90, 2)).toBe(3);
  });

  it('lat2tileY converts latitude to tile y at zoom 1', () => {
    expect(lat2tileY(0, 1)).toBe(1);
    expect(lat2tileY(85.051128, 1)).toBe(0); // near north pole (within Mercator limit)
    expect(lat2tileY(-85.051128, 1)).toBe(1); // near south pole (only 2 tiles at z=1)
  });

  it('tile2lng converts tile x back to longitude', () => {
    expect(tile2lng(0, 0)).toBeCloseTo(-180);
    expect(tile2lng(1, 0)).toBeCloseTo(180);
  });

  it('tile2lat converts tile y back to latitude', () => {
    expect(tile2lat(0, 0)).toBeCloseTo(85.051129, 3);
    expect(tile2lat(1, 0)).toBeCloseTo(-85.051129, 3);
  });

  it('round-trip lng -> tileX -> lng is approximately identity', () => {
    const lng = -74.006;
    const x = lng2tileX(lng, 11);
    const back = tile2lng(x, 11);
    expect(back).toBeCloseTo(lng, 0); // tile width at z=11 is ~0.176°
  });

  it('round-trip lat -> tileY -> lat is approximately identity', () => {
    const lat = 40.7128;
    const y = lat2tileY(lat, 11);
    const back = tile2lat(y, 11);
    expect(back).toBeCloseTo(lat, 2);
  });

  it('getTileCoords returns correct tile for NYC at zoom 11', () => {
    const coords = getTileCoords({ lng: -74.006, lat: 40.7128 }, 11);
    expect(coords.z).toBe(11);
    expect(coords.x).toBe(lng2tileX(-74.006, 11));
    expect(coords.y).toBe(lat2tileY(40.7128, 11));
  });
});
