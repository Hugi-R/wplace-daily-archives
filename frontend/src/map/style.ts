// src/map/style.ts

export interface MapStyle {
  version: number;
  sources: {
    osm: {
      type: 'raster';
      tiles: string[];
      minzoom: number;
      maxzoom: number;
      tileSize: number;
      attribution: string;
    };
    wplace: {
      type: 'raster';
      tiles: string[];
      minzoom: number;
      maxzoom: number;
      attribution: string;
    };
  };
  layers: Array<{
    id: string;
    type: 'raster';
    source: string;
    minzoom: number;
    maxzoom: number;
    paint?: Record<string, unknown>;
  }>;
}

function getWplaceTileUrl(version: number, layer: string): string {
  return `merged://tiles/${layer}/${version}/{z}/{x}/{y}.png`;
}

export function getMapStyle(version: number, layer: string = 'tiles'): MapStyle {
  return {
    version: 8,
    sources: {
      osm: {
        type: 'raster',
        tiles: [
          'https://a.tile.openstreetmap.org/{z}/{x}/{y}.png',
          'https://b.tile.openstreetmap.org/{z}/{x}/{y}.png',
          'https://c.tile.openstreetmap.org/{z}/{x}/{y}.png',
        ],
        minzoom: 0,
        maxzoom: 12,
        tileSize: 256,
        attribution: '\u00a9 OpenStreetMap contributors',
      },
      wplace: {
        type: 'raster',
        tiles: [getWplaceTileUrl(version, layer)],
        minzoom: 0,
        maxzoom: 11,
        attribution: '\u00a9 wplace.live',
      },
    },
    layers: [
      {
        id: 'osm',
        type: 'raster',
        source: 'osm',
        minzoom: 0,
        maxzoom: 19,
      },
      {
        id: 'wplace',
        type: 'raster',
        source: 'wplace',
        minzoom: 0,
        maxzoom: 22,
        paint: {
          'raster-fade-duration': 0,
          'raster-resampling': 'nearest',
        },
      },
    ],
  };
}
