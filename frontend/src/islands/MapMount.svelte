<!-- src/islands/MapMount.svelte -->
<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { Map as MapLibre, setWorkerUrl } from 'maplibre-gl';
  import 'maplibre-gl/dist/maplibre-gl.css';
  // Vite resolves this to a dev-served URL with the correct MIME type, instead of
  // MapLibre's default which points into node_modules/.vite/deps (forbidden MIME).
  import maplibreWorkerUrl from 'maplibre-gl/dist/maplibre-gl-worker.mjs?url';
 
  import { version, layer, viewport } from '../state/stores';
  import { syncUrlToStores, syncStoresToUrl, debouncedUrlUpdate } from '../state/url-sync';
  import { getMapStyle } from '../map/style';
  import { registerProtocol } from '../map/protocol';
  import { TileWorkerPool } from '../workers/pool';

  let map: MapLibre | null = null;
  let pool: TileWorkerPool | null = null;
  let container: HTMLElement;

  // Subscription handles for cleanup
  let unsubVersion: (() => void) | null = null;
  let unsubLayer: (() => void) | null = null;

  onMount(() => {
    // Seed stores from the URL *before* initializing the map, otherwise the map
    // inits at the default viewport and its first moveend overwrites the URL.
    syncUrlToStores();

    setWorkerUrl(maplibreWorkerUrl);
    pool = new TileWorkerPool();

    const vp = viewport.get();
    const v = version.get();
    const l = layer.get();

    map = new MapLibre({
      container: container,
      style: getMapStyle(v, l),
      center: [vp.lng, vp.lat],
      zoom: vp.zoom,
    });

    registerProtocol(map, pool);

    // Wire version changes
    unsubVersion = version.subscribe((v) => {
      if (map) {
        const l = layer.get();
        map.setStyle(getMapStyle(v, l));
        map.once('styledata', () => syncStoresToUrl());
      }
    });

    // Wire layer changes
    unsubLayer = layer.subscribe((l) => {
      if (map) {
        const v = version.get();
        map.setStyle(getMapStyle(v, l));
        map.once('styledata', () => syncStoresToUrl());
      }
    });

    // Wire map movement to viewport store
    map.on('moveend', () => {
      if (!map) return;
      const center = map.getCenter();
      const zoom = map.getZoom();
      viewport.set({ lat: center.lat, lng: center.lng, zoom });
      debouncedUrlUpdate();
    });

    // Expose map instance for other islands to access
    (window as any).__wplaceMap = map;
  });

  onDestroy(() => {
    unsubVersion?.();
    unsubLayer?.();
    pool?.terminate();
    map?.remove();
  });
</script>

<div bind:this={container} id="map-container"></div>
