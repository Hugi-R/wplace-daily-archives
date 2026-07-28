<!-- src/islands/ZoomButtons.svelte -->
<script lang="ts">
  import { uiVisible } from '../state/stores';
  import * as m from '../paraglide/messages.js';

  function zoomIn() {
    const map = (window as any).__wplaceMap;
    if (map?.zoomIn) map.zoomIn();
  }

  function zoomOut() {
    const map = (window as any).__wplaceMap;
    if (map?.zoomOut) map.zoomOut();
  }
</script>

<div class="zoom-button-container" class:ui-hidden={!$uiVisible}>
  <button class="zoom-button" title={m.zoom_in_title()} on:click={zoomIn} aria-label={m.zoom_in_title()}>
    <svg xmlns="http://www.w3.org/2000/svg" fill="none" width="22" height="22" viewBox="0 0 24 24" stroke-width="1.5" stroke="currentColor" aria-hidden="true">
      <path stroke-linecap="round" stroke-linejoin="round" d="M12 4.5v15m7.5-7.5h-15" />
    </svg>
  </button>
  <button class="zoom-button" title={m.zoom_out_title()} on:click={zoomOut} aria-label={m.zoom_out_title()}>
    <svg xmlns="http://www.w3.org/2000/svg" fill="none" width="22" height="22" viewBox="0 0 24 24" stroke-width="1.5" stroke="currentColor" aria-hidden="true">
      <path stroke-linecap="round" stroke-linejoin="round" d="M5 12h14" />
    </svg>
  </button>
</div>

<style>
  .zoom-button-container {
    position: absolute;
    bottom: 50px;
    left: 10px;
    z-index: 3;
    display: flex;
    flex-direction: column;
    font-weight: bold;
    background: var(--button-face);
    padding: 3px;
    gap: 2px;
    border-radius: 7px;
    box-shadow: var(--shadow-sm);
  }

  .zoom-button {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 2px;
    box-shadow: none;
    outline: none;
  }
</style>
