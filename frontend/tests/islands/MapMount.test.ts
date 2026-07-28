// tests/islands/MapMount.test.ts
import { describe, it, expect, beforeEach, afterEach, vi, afterAll } from 'vitest';
import { render, cleanup } from '@testing-library/svelte';

// Mock Worker to prevent instantiation in jsdom
vi.stubGlobal('Worker', class {
  onmessage = null;
  onerror = null;
  postMessage() {}
  terminate() {}
});

// Mock MapLibre (named export: Map)
vi.mock('maplibre-gl', () => {
  const MockMap = class {
    on() {}
    once() {}
    getCenter() { return { lat: 0, lng: 0 }; }
    getZoom() { return 2; }
    remove() {}
    setStyle() {}
  };
  return {
    Map: MockMap,
  };
});

// Mock protocol and worker pool
vi.mock('../../src/map/protocol', () => ({
  registerProtocol: vi.fn(),
}));
vi.mock('../../src/workers/pool', () => ({
  TileWorkerPool: class { terminate() {} },
}));

vi.mock('../../src/state/url-sync', () => ({
  syncStoresToUrl: vi.fn(),
  debouncedUrlUpdate: vi.fn(),
}));
vi.mock('../../src/map/style', () => ({
  getMapStyle: () => ({}),
}));

import MapMount from '../../src/islands/MapMount.svelte';

afterAll(() => {
  vi.restoreAllMocks();
  cleanup();
});

describe('MapMount', () => {
  it('renders map container element', () => {
    const { container } = render(MapMount);
    const mapContainer = container.querySelector('#map-container');
    expect(mapContainer).toBeTruthy();
  });
});
