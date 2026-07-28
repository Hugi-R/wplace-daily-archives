<!-- src/islands/VersionSlider.svelte -->
<script lang="ts">
  import { onMount } from 'svelte';
  import { version, uiVisible } from '../state/stores';
  import { syncStoresToUrl } from '../state/url-sync';
  import * as m from '../paraglide/messages.js';

  interface ApiVersion {
    version: string;
    date: string;
  }

  let versions: ApiVersion[] = [];

  function getDateForValue(value: number): string {
    const idx = Math.round(value);
    if (idx >= 0 && idx < versions.length) {
      return versions[idx].date;
    }
    return '';
  }

  function handleInput(e: Event) {
    const target = e.target as HTMLInputElement;
    const val = parseInt(target.value, 10);
    if (!isNaN(val)) {
      version.set(val);
      syncStoresToUrl();
    }
  }

  onMount(async () => {
    try {
      const res = await fetch('/api/versions');
      if (res.ok) {
        versions = await res.json();
      }
    } catch {
      // silently ignore fetch errors
    }
  });
</script>

<div class="version-slider" class:ui-hidden={!$uiVisible}>
  <label for="version-range" class="version-label">{m.version_label()}</label>
  <input
    id="version-range"
    type="range"
    min={0}
    max={versions.length - 1}
    value={version.get()}
    on:input={handleInput}
    list="version-datalist"
  />
  <datalist id="version-datalist">
    {#each versions as v, i}
      <option value={i}>{v.date}</option>
    {/each}
  </datalist>
  <span class="date-label">{getDateForValue(version.get())}</span>
</div>

<style>
  .version-slider {
    position: fixed;
    top: 0;
    left: 0;
    right: 0;
    z-index: 100;
    box-sizing: border-box;
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 12px;
    background: var(--panel-bg);
    box-shadow: var(--shadow-lg);
    border-bottom: 1px solid var(--border-color);
    font-size: 17px;
  }

  .version-slider input[type='range'] {
    flex: 1;
    margin: 0;
  }

  .version-label {
    font-size: 17px;
    white-space: nowrap;
  }

  .date-label {
    font-size: 0.875rem;
    min-width: 90px;
  }
</style>
