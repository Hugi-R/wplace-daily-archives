// tests/state/url-sync.test.ts
import { describe, it, expect, beforeEach, vi, afterEach } from 'vitest';
import {
  version,
  layer,
  viewport,
  devMode,
  defaultViewport,
} from '../../src/state/stores';

describe('url-sync', () => {
  let urlSyncModule: typeof import('../../src/state/url-sync');

  beforeEach(() => {
    version.set(0);
    layer.set('tiles');
    viewport.set(defaultViewport);
    devMode.set(false);

    Object.defineProperty(window, 'location', {
      value: {
        pathname: '/en/',
        search: '',
      },
      writable: true,
    });
    vi.spyOn(history, 'replaceState').mockImplementation(() => {});
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('syncUrlToStores reads lat/lng/zoom from URL', async () => {
    window.location.search = '?lat=40.7&lng=-74.0&zoom=5';
    urlSyncModule = await import('../../src/state/url-sync');
    urlSyncModule.syncUrlToStores();

    expect(viewport.get()).toEqual({ lat: 40.7, lng: -74.0, zoom: 5 });
  });

  it('syncUrlToStores reads dev param', async () => {
    window.location.search = '?dev=true';
    urlSyncModule = await import('../../src/state/url-sync');
    urlSyncModule.syncUrlToStores();

    expect(devMode.get()).toBe(true);
  });

  it('syncUrlToStores ignores invalid params', async () => {
    window.location.search = '?lat=abc&lng=xyz';
    urlSyncModule = await import('../../src/state/url-sync');
    urlSyncModule.syncUrlToStores();

    expect(viewport.get()).toEqual(defaultViewport);
  });

  it('syncStoresToUrl writes current state to URL', async () => {
    version.set(12345);
    layer.set('alternate');
    viewport.set({ lat: 40.7128, lng: -74.006, zoom: 5.5 });

    urlSyncModule = await import('../../src/state/url-sync');
    urlSyncModule.syncStoresToUrl();

    expect(history.replaceState).toHaveBeenCalledWith(
      null,
      '',
      '/en/?lat=40.712800&lng=-74.006000&zoom=5.50&version=12345&layer=alternate'
    );
  });
});
