<!-- src/islands/TileOverlay.svelte -->
<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { devMode, uiVisible } from '../state/stores';
  import type { Map } from 'maplibre-gl';
  import * as m from '../paraglide/messages.js';

  export let enabled: boolean = false;

  let canvas: HTMLCanvasElement;
  let map: Map | null = null;
  let visible = true;

  function getMap(): Map | null {
    if (map) return map;
    return (window as any).__wplaceMap ?? null;
  }

  function lngToTileX(lng: number, zoom: number): number {
    return Math.floor(((lng + 180) / 360) * Math.pow(2, zoom));
  }

  function latToTileY(lat: number, zoom: number): number {
    const latRad = (lat * Math.PI) / 180;
    return Math.floor(
      ((1 - Math.log(Math.tan(latRad) + 1 / Math.cos(latRad)) / Math.PI) / 2) * Math.pow(2, zoom)
    );
  }

  function drawOverlay() {
    const mlMap = getMap();
    if (!mlMap || !visible || !enabled) return;

    const container = mlMap.getContainer();
    const width = container.clientWidth;
    const height = container.clientHeight;

    canvas.width = width;
    canvas.height = height;

    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    ctx.clearRect(0, 0, width, height);

    const zoom = mlMap.getZoom();
    if (zoom === undefined) return;

    // Get visible bounds in pixel coordinates
    const topLeft = mlMap.unproject([0, 0]);
    const bottomRight = mlMap.unproject([width, height]);

    const minTileX = lngToTileX(bottomRight.lng, zoom);
    const maxTileX = lngToTileX(topLeft.lng, zoom);
    const minTileY = latToTileY(topLeft.lat, zoom);
    const maxTileY = latToTileY(bottomRight.lat, zoom);

    ctx.strokeStyle = 'rgba(255, 0, 0, 0.6)';
    ctx.lineWidth = 1;
    ctx.fillStyle = 'rgba(255, 255, 255, 0.8)';
    ctx.font = '10px monospace';

    for (let tx = minTileX; tx <= maxTileX; tx++) {
      for (let ty = minTileY; ty <= maxTileY; ty++) {
        // Calculate pixel position of tile top-left corner
        const lng = (tx / Math.pow(2, zoom)) * 360 - 180;
        const n = Math.PI - 2 * Math.PI * (ty / Math.pow(2, zoom));
        const lat = (180 / Math.PI) * Math.atan(0.5 * (Math.exp(n) - Math.exp(-n)));

        const pixel = mlMap.project({ lng, lat });
        const nextLng = ((tx + 1) / Math.pow(2, zoom)) * 360 - 180;
        const nextPixel = mlMap.project({ lng: nextLng, lat });

        const tileWidth = nextPixel.x - pixel.x;
        const nextLatN = Math.PI - 2 * Math.PI * ((ty + 1) / Math.pow(2, zoom));
        const nextLat = (180 / Math.PI) * Math.atan(0.5 * (Math.exp(nextLatN) - Math.exp(-nextLatN)));
        const bottomPixel = mlMap.project({ lng, lat: nextLat });
        const tileHeight = bottomPixel.y - pixel.y;

        ctx.strokeRect(pixel.x, pixel.y, tileWidth, tileHeight);

        // Draw tile ID in center
        const labelX = pixel.x + tileWidth / 2;
        const labelY = pixel.y + tileHeight / 2;
        ctx.fillText(`${tx},${ty}`, labelX - 15, labelY + 3);
      }
    }
  }

  function handleToggle(e: Event) {
    const checked = (e.target as HTMLInputElement).checked;
    visible = checked;
    enabled = checked;
    if (!checked) {
      const ctx = canvas.getContext('2d');
      if (ctx) {
        ctx.clearRect(0, 0, canvas.width, canvas.height);
      }
    } else {
      drawOverlay();
    }
  }

  onMount(() => {
    // MapMount may not have exposed the map yet (island hydration order is not
    // guaranteed), so retry briefly until it's available.
    let attempts = 0;
    const attach = () => {
      const mlMap = getMap();
      if (mlMap) {
        map = mlMap;
        mlMap.on('moveend', drawOverlay);
        mlMap.on('zoomend', drawOverlay);
        mlMap.on('resize', drawOverlay);
        return;
      }
      if (attempts++ < 50) setTimeout(attach, 100);
    };
    attach();
  });

  onDestroy(() => {
    const mlMap = getMap();
    if (mlMap) {
      mlMap.off('moveend', drawOverlay);
      mlMap.off('zoomend', drawOverlay);
      mlMap.off('resize', drawOverlay);
    }
  });
</script>

{#if $devMode}
<div class="tile-overlay-container" class:ui-hidden={!$uiVisible}>
  <label class="tile-overlay-toggle" class:ui-hidden={!$uiVisible}>
    <input type="checkbox" checked={enabled} on:change={handleToggle} />
    {m.show_tile_overlay()}
  </label>
  <canvas bind:this={canvas} class="tile-canvas" />
</div>
{/if}

<style>
  .tile-overlay-container {
    position: fixed;
    top: 0;
    left: 0;
    width: 100%;
    height: 100%;
    pointer-events: none;
    z-index: 999;
  }

  .tile-canvas {
    position: absolute;
    top: 0;
    left: 0;
    width: 100%;
    height: 100%;
  }

  .tile-overlay-toggle {
    position: absolute;
    top: 0.5rem;
    right: 0.5rem;
    background: var(--panel-bg-weak);
    color: var(--text-color);
    padding: 6px 12px;
    border-radius: 4px;
    font-size: 16px;
    box-shadow: var(--shadow-sm);
    pointer-events: auto;
    z-index: 1001;
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
</style>
