<!-- src/islands/LayerSelector.svelte -->
<script lang="ts">
  import { layer, uiVisible } from '../state/stores';
  import { syncStoresToUrl } from '../state/url-sync';
  import * as m from '../paraglide/messages.js';

  const layers = ['tiles'];

  function handleChange(e: Event) {
    const target = e.target as HTMLSelectElement;
    layer.set(target.value);
    syncStoresToUrl();
  }
</script>

<div class="layer-selector" class:ui-hidden={!$uiVisible}>
  <label for="layer-select">{m.layer_label()}</label>
  <select id="layer-select" on:change={handleChange}>
    {#each layers as l}
      <option value={l} selected={l === $layer}>{l}</option>
    {/each}
  </select>
</div>

<style>
  .layer-selector {
    background: var(--panel-bg-weak);
    padding: 6px 12px;
    border-radius: 4px;
    font-size: 16px;
    display: flex;
    align-items: center;
    gap: 6px;
    max-width: calc(100vw - 40px);
  }

  .layer-selector label {
    font-size: 15px;
  }

  .layer-selector select {
    font-size: 15px;
    padding: 2px 4px;
  }
</style>
