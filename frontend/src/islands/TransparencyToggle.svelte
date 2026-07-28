<!-- src/islands/TransparencyToggle.svelte -->
<script lang="ts">
  import { uiVisible } from '../state/stores';
  import * as m from '../paraglide/messages.js';

  let isTransparent = false;

  function handleClick() {
    isTransparent = !isTransparent;
    const map = (window as any).__wplaceMap;
    if (map) {
      map.setPaintProperty('wplace', 'raster-opacity', isTransparent ? 0.3 : 1);
    }
  }
</script>

<button
  class="transparency-toggle"
  class:ui-hidden={!$uiVisible}
  class:toggled={isTransparent}
  on:click={handleClick}
  aria-label={m.toggle_transparency()}
  title={m.toggle_transparency()}
>
  <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 -960 960 960" width="22" height="22" fill="currentColor" aria-hidden="true">
    <path d="M440-440v-80h80v80h-80Zm-80 80v-80h80v80h-80Zm160 0v-80h80v80h-80Zm80-80v-80h80v80h-80Zm-320 0v-80h80v80h-80Zm-80 320q-33 0-56.5-23.5T120-200v-560q0-33 23.5-56.5T200-840h560q33 0 56.5 23.5T840-760v560q0 33-23.5 56.5T760-120H200Zm80-80h80v-80h-80v80Zm160 0h80v-80h-80v80Zm320 0v-80 80Zm-560-80h80v-80h80v80h80v-80h80v80h80v-80h80v80h80v-80h-80v-80h80v-320H200v320h80v80h-80v80Zm0 80v-560 560Zm560-240v80-80ZM600-280v80h80v-80h-80Z"></path>
  </svg>
</button>

<style>
  .transparency-toggle {
    position: fixed;
    bottom: 10px;
    left: 10px;
    z-index: 3;
    padding: 6px;
    font-size: 0;
  }
</style>
