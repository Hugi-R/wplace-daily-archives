// src/map/coordinates.ts

export function lng2tileX(lng: number, z: number): number {
  return Math.floor((lng + 180) / 360 * Math.pow(2, z));
}

export function lat2tileY(lat: number, z: number): number {
  const rad = lat * Math.PI / 180;
  return Math.floor(
    (1 - Math.log(Math.tan(rad) + 1 / Math.cos(rad)) / Math.PI) / 2 * Math.pow(2, z)
  );
}

export function tile2lng(x: number, z: number): number {
  return x / Math.pow(2, z) * 360 - 180;
}

export function tile2lat(y: number, z: number): number {
  const n = Math.PI - 2 * Math.PI * y / Math.pow(2, z);
  return (180 / Math.PI) * Math.atan(0.5 * (Math.exp(n) - Math.exp(-n)));
}

export interface TileCoords {
  z: number;
  x: number;
  y: number;
}

export interface LngLat {
  lng: number;
  lat: number;
}

export function getTileCoords(lngLat: LngLat, zoom: number): TileCoords {
  const z = Math.floor(zoom);
  const x = lng2tileX(lngLat.lng, z);
  const y = lat2tileY(lngLat.lat, z);
  return { z, x, y };
}
