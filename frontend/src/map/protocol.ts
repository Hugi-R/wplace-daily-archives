// src/map/protocol.ts
import type { Map } from 'maplibre-gl';
import { addProtocol } from 'maplibre-gl';
import type { TileWorkerPool } from '../workers/pool';

export interface ParsedTileUrl {
  layer: string;
  version: number;
  z: number;
  x: number;
  y: number;
}

export function parseTileUrl(url: string): ParsedTileUrl {
  // merged://tiles/{layer}/{version}/{z}/{x}/{y}.png
  const match = url.match(/tiles\/([^/]+)\/(\d+)\/(\d+)\/(\d+)\/(\d+)\.png/);
  if (!match) {
    throw new Error(`Invalid tile URL: ${url}`);
  }
  return {
    layer: match[1],
    version: parseInt(match[2], 10),
    z: parseInt(match[3], 10),
    x: parseInt(match[4], 10),
    y: parseInt(match[5], 10),
  };
}

export function getCacheKey(layer: string, version: number, z: number, x: number, y: number): string {
  return `tile-${layer}-${version}-${z}-${x}-${y}`;
}

export function registerProtocol(_map: Map, pool: TileWorkerPool): void {
  addProtocol('merged', async (params, _abortController) => {
    const { layer, version, z, x, y } = parseTileUrl(params.url);

    const cache = typeof caches !== 'undefined' ? await caches.open('merged-tiles') : null;
    const cacheKey = getCacheKey(layer, version, z, x, y);

    if (cache) {
      const cached = await cache.match(cacheKey);
      if (cached) {
        const buffer = await cached.arrayBuffer();
        return { data: buffer };
      }
    }

    const img = await getTile(pool, version, z, x, y);

    if (cache) {
      const response = new Response(img);
      await cache.put(cacheKey, response);
    }

    return { data: img };
  });
}

export async function getTile(
  pool: TileWorkerPool,
  version: number,
  z: number,
  x: number,
  y: number
): Promise<ArrayBuffer> {
  const perfStart = performance.now();
  const week = Math.floor(version / (7 * 24));

  const tile = await fetch(`/tiles/${week}/${z}/${x}/${y}.zst`);
  if (tile.status !== 200) {
    throw new Error('empty tile');
  }
  const buffer = await tile.arrayBuffer();

  const perfNetworkEnd = performance.now();
  reportMetric('decompress-network', perfNetworkEnd - perfStart);

  const img = await pool.decompress(version, buffer);

  const perfEnd = performance.now();
  reportMetric('decompress-process', perfEnd - perfNetworkEnd);
  reportMetric('decompress', perfEnd - perfStart);

  return img;
}

// Metrics collection (used by StatsPanel)
const metrics: Record<string, number[]> = {
  decompress: [],
  'decompress-network': [],
  'decompress-process': [],
};

export function reportMetric(name: string, value: number): void {
  if (metrics[name] === undefined) metrics[name] = [];
  metrics[name].push(value);
}

export function getMetrics(): Record<string, number[]> {
  return metrics;
}

export function computePercentiles(data: number[]): { p10: number; p50: number; p90: number } | null {
  if (data.length === 0) return null;
  const sorted = [...data].sort((a, b) => a - b);
  return {
    p10: sorted[Math.floor(0.1 * sorted.length)],
    p50: sorted[Math.floor(0.5 * sorted.length)],
    p90: sorted[Math.floor(0.9 * sorted.length)],
  };
}
