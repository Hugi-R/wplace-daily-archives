// src/state/url-sync.ts

import {
  version,
  layer,
  viewport,
  devMode,
  locale,
  defaultViewport,
  type Viewport,
} from './stores';

function getSearchString(): string {
  return typeof window !== 'undefined' ? window.location.search : '';
}

function getPathname(): string {
  return typeof window !== 'undefined' ? window.location.pathname : '';
}

function parseUrlParams(): {
  lat?: number;
  lng?: number;
  zoom?: number;
  version?: string;
  layer?: string;
  dev?: boolean;
} {
  const params = new URLSearchParams(getSearchString());
  const lat = parseFloat(params.get('lat') ?? '');
  const lng = parseFloat(params.get('lng') ?? '');
  const zoom = parseFloat(params.get('zoom') ?? '');
  return {
    lat: isNaN(lat) ? undefined : lat,
    lng: isNaN(lng) ? undefined : lng,
    zoom: isNaN(zoom) ? undefined : zoom,
    version: params.get('version') ?? undefined,
    layer: params.get('layer') ?? undefined,
    dev: params.get('dev') === 'true',
  };
}

/** Read URL params and seed all stores. Call once on initial load. */
export function syncUrlToStores(): void {
  if (typeof window === 'undefined') return;
  const params = parseUrlParams();

  // Derive the locale from the first path segment (e.g. /es/... → es)
  const segments = getPathname().split('/').filter(Boolean);
  const pathLocale = segments[0];
  if (pathLocale) {
    locale.set(pathLocale);
  }

  const vp: Viewport = { ...defaultViewport };
  if (params.lat !== undefined && params.lng !== undefined) {
    vp.lat = params.lat;
    vp.lng = params.lng;
  }
  if (params.zoom !== undefined) {
    vp.zoom = params.zoom;
  }
  viewport.set(vp);

  if (params.version !== undefined) {
    const v = parseInt(params.version);
    if (!isNaN(v)) version.set(v);
  }
  if (params.layer !== undefined) {
    layer.set(params.layer);
  }
  if (params.dev) {
    devMode.set(true);
  }
}

/** Write current store values to the URL via history.replaceState. */
export function syncStoresToUrl(): void {
  if (typeof window === 'undefined') return;
  const v = version.get();
  const l = layer.get();
  const vp = viewport.get();

  const params = new URLSearchParams(getSearchString());
  params.set('lat', vp.lat.toFixed(6));
  params.set('lng', vp.lng.toFixed(6));
  params.set('zoom', vp.zoom.toFixed(2));
  if (v !== undefined && v !== 0) {
    params.set('version', String(v));
  }
  if (l && l !== 'tiles') {
    params.set('layer', l);
  }
  if (devMode.get()) {
    params.set('dev', 'true');
  } else {
    params.delete('dev');
  }

  const newUrl = getPathname() + '?' + params.toString();
  history.replaceState(null, '', newUrl);
}

/** Returns a debounced version of syncStoresToUrl (300ms). */
let _debounceTimer: ReturnType<typeof setTimeout> | null = null;
export const debouncedUrlUpdate = (): void => {
  if (_debounceTimer !== null) {
    clearTimeout(_debounceTimer);
  }
  _debounceTimer = setTimeout(() => {
    syncStoresToUrl();
    _debounceTimer = null;
  }, 300);
};
