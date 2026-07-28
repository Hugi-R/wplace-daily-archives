<!-- src/islands/VideoConfig.svelte -->
<script lang="ts">
  import { uiVisible, layer, version, viewport } from '../state/stores';
  import { takeScreenshot, downloadBlob } from '../media/screenshot';
  import { takeVideo } from '../media/video';
  import { getTileCoords } from '../map/coordinates';
  import * as m from '../paraglide/messages.js';

  let showVideoForm = false;

  // Video form state
  let x1Str: string = '0';
  let y1Str: string = '0';
  let x2Str: string = '1';
  let y2Str: string = '1';
  let dateFrom: string = '';
  let dateTo: string = '';

  let message = '';
  let loading = false;

  function toBigInt(value: string): bigint {
    const n = parseInt(value, 10);
    return isNaN(n) ? 0n : BigInt(n);
  }

  async function handleScreenshot() {
    if (!$uiVisible) return;
    message = '';
    loading = true;

    try {
      // Derive a single-tile range from the current viewport center
      const vp = $viewport;
      const tile = getTileCoords({ lng: vp.lng, lat: vp.lat }, vp.zoom);
      const tileX = BigInt(tile.x);
      const tileY = BigInt(tile.y);

      const config = {
        layer: $layer.get(),
        version: $version.get(),
        x1: tileX,
        y1: tileY,
        x2: tileX + 1n,
        y2: tileY + 1n,
      };

      const blob = await takeScreenshot(config);
      downloadBlob(blob, `screenshot-${Date.now()}.png`);
      message = m.screenshot_saved();
    } catch (err: any) {
      message = err.message || m.screenshot_failed({ error: String(err) });
    } finally {
      loading = false;
    }
  }

  async function handleVideo() {
    if (!$uiVisible) return;
    message = '';
    loading = true;

    try {
      const config = {
        layer: $layer.get(),
        x1: toBigInt(x1Str),
        y1: toBigInt(y1Str),
        x2: toBigInt(x2Str),
        y2: toBigInt(y2Str),
        from: dateFrom ? new Date(dateFrom).getTime() / 1000 : $version.get(),
        to: dateTo ? new Date(dateTo).getTime() / 1000 : $version.get(),
      };

      const blob = await takeVideo(config);
      downloadBlob(blob, `video-${Date.now()}.png`);
      showVideoForm = false;
      message = m.video_saved();
    } catch (err: any) {
      message = err.message || m.video_failed({ error: String(err) });
    } finally {
      loading = false;
    }
  }

  function cancelVideoForm() {
    showVideoForm = false;
  }
</script>

<div class="video-config" class:ui-hidden={!$uiVisible}>
  <button type="button" on:click={handleScreenshot} disabled={loading}>
    {m.screenshot_button()}
  </button>
  <button type="button" on:click={() => { showVideoForm = !showVideoForm; }} disabled={loading}>
    {m.video_button()}
  </button>

  {#if showVideoForm}
    <form class="video-form" on:submit={(e) => { e.preventDefault(); handleVideo(); }}>
      <fieldset>
        <legend>{m.video_tile_range()}</legend>
        <label>X1 <input type="number" step="1" bind:value={x1Str} /></label>
        <label>Y1 <input type="number" step="1" bind:value={y1Str} /></label>
        <label>X2 <input type="number" step="1" bind:value={x2Str} /></label>
        <label>Y2 <input type="number" step="1" bind:value={y2Str} /></label>
      </fieldset>
      <fieldset>
        <legend>{m.video_date_range()}</legend>
        <label>{m.video_from()} <input type="date" bind:value={dateFrom} /></label>
        <label>{m.video_to()} <input type="date" bind:value={dateTo} /></label>
      </fieldset>
      <div class="form-actions">
        <button type="submit" disabled={loading}>{m.video_generate()}</button>
        <button type="button" on:click={cancelVideoForm}>{m.video_cancel()}</button>
      </div>
    </form>
  {/if}

  {#if message}
    <p class="message">{message}</p>
  {/if}
</div>

<style>
  .video-config {
    background: var(--panel-bg-weak);
    padding: 6px 12px;
    border-radius: 4px;
    font-size: 16px;
    display: flex;
    flex-wrap: wrap;
    align-items: flex-start;
    gap: 6px;
    max-width: calc(100vw - 40px);
  }

  button {
    font-size: 15px;
    padding: 2px 4px;
    cursor: pointer;
  }

  button:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .video-form {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 8px;
    background: var(--panel-bg);
    border: 1px solid var(--border-color);
    border-radius: 4px;
    box-shadow: var(--shadow-sm);
    width: 100%;
  }

  fieldset {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    padding: 2px;
    border: none;
  }

  legend {
    font-size: 14px;
    font-weight: bold;
  }

  label {
    display: flex;
    align-items: center;
    gap: 4px;
    font-size: 14px;
  }

  input {
    width: 60px;
    font-size: 14px;
    padding: 2px 4px;
  }

  .form-actions {
    display: flex;
    gap: 6px;
  }

  .message {
    font-size: 14px;
    color: var(--muted-text-strong);
    width: 100%;
  }
</style>
