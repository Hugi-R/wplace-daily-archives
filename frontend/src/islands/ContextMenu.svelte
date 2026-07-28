<!-- src/islands/ContextMenu.svelte -->
<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { uiVisible, version, layer } from '../state/stores';
  import { getTileCoords } from '../map/coordinates';
  import { getCacheKey } from '../map/protocol';
  import * as m from '../paraglide/messages.js';

  interface MenuItem {
    label: string;
    action: () => void;
  }

  let menuOpen = false;
  let menuX = 0;
  let menuY = 0;
  let items: MenuItem[] = [];
  let menuElement: HTMLElement;

  function handleClickOutside(e: MouseEvent) {
    if (!menuElement.contains(e.target as Node)) {
      menuOpen = false;
      items = [];
    }
  }

  function handleContextMenu(e: Event) {
    const map = (window as any).__wplaceMap;
    if (!map || !$uiVisible) return;

    e.preventDefault();

    const mapEvent = e as unknown as { point?: { x: number; y: number } };
    if (!mapEvent.point) return;

    // Get the click coordinates
    const point = map.unproject([mapEvent.point.x, mapEvent.point.y]);

    // Calculate tile coordinates
    const currentZoom = map.getZoom();
    const tile = getTileCoords({ lat: point.lat, lng: point.lng }, currentZoom);
    const cacheKey = getCacheKey(layer.get(), version.get(), tile.z, tile.x, tile.y);

    menuX = mapEvent.point.x;
    menuY = mapEvent.point.y;

    items = [
      {
        label: m.download_tile(),
        action: async () => {
          try {
            const cache = await caches.open('merged-tiles');
            const response = await cache.match(cacheKey);
            if (response) {
              const blob = await response.blob();
              const url = URL.createObjectURL(blob);
              const a = document.createElement('a');
              a.href = url;
              a.download = `tile-${tile.z}-${tile.x}-${tile.y}.png`;
              a.click();
              URL.revokeObjectURL(url);
            }
          } catch {
            // silently ignore
          }
          menuOpen = false;
          items = [];
        },
      },
      {
        label: m.go_wplace_live(),
        action: () => {
          const url = `https://wplace.live?lat=${point.lat.toFixed(6)}&lng=${point.lng.toFixed(6)}`;
          window.open(url, '_blank');
          menuOpen = false;
          items = [];
        },
      },
    ];

    menuOpen = true;
  }

  onMount(() => {
    const map = (window as any).__wplaceMap;
    if (map) {
      map.on('contextmenu', handleContextMenu);
    }
    document.addEventListener('click', handleClickOutside);
  });

  onDestroy(() => {
    const map = (window as any).__wplaceMap;
    if (map) {
      map.off('contextmenu', handleContextMenu);
    }
    document.removeEventListener('click', handleClickOutside);
  });
</script>

{#if menuOpen}
  <div
    bind:this={menuElement}
    class="context-menu"
    class:ui-hidden={!$uiVisible}
    style="left: {menuX}px; top: {menuY}px;"
  >
    {#each items as item}
      <button on:click={item.action}>{item.label}</button>
    {/each}
  </div>
{/if}

<style>
  .context-menu {
    position: fixed;
    z-index: 9999;
    background: var(--context-bg);
    border: 1px solid var(--context-border);
    border-radius: 6px;
    box-shadow: var(--shadow-mid);
    padding: 8px 0;
    min-width: 140px;
    font-size: 15px;
    color: var(--button-text);
  }

  button {
    display: block;
    width: 100%;
    text-align: left;
    padding: 8px 24px;
    border: none;
    background: none;
    box-shadow: none;
    outline: none;
    cursor: pointer;
    font-size: 15px;
  }

  button:hover {
    background: var(--highlight);
    color: var(--highlight-text);
  }
</style>
