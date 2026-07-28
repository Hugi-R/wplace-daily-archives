<!-- src/islands/CoordForm.svelte -->
<script lang="ts">
  import { viewport, uiVisible } from '../state/stores';
  import * as m from '../paraglide/messages.js';

  let zoom: number = $viewport.zoom;
  let lat: number = $viewport.lat;
  let lng: number = $viewport.lng;
  let editing = false;

  // Keep the inputs in sync with the map/URL unless the user is actively editing.
  $: if (!editing) {
    zoom = $viewport.zoom;
    lat = $viewport.lat;
    lng = $viewport.lng;
  }

  function handleSubmit(e: Event) {
    e.preventDefault();
    editing = false;
    const map = (window as any).__wplaceMap;
    if (map && map.flyTo) {
      map.flyTo({ center: [lng, lat], zoom, essential: true });
    }
  }

  function handleZoomInput(e: Event) {
    const target = e.target as HTMLInputElement;
    zoom = parseFloat(target.value);
  }

  function handleLatInput(e: Event) {
    const target = e.target as HTMLInputElement;
    lat = parseFloat(target.value);
  }

  function handleLngInput(e: Event) {
    const target = e.target as HTMLInputElement;
    lng = parseFloat(target.value);
  }
</script>

<div class="coord-form" class:ui-hidden={!$uiVisible}>
  <form on:submit={handleSubmit}>
    <label for="coord-zoom">{m.zoom_label()}</label>
    <input
      id="coord-zoom"
      type="number"
      step="any"
      placeholder={m.zoom_label()}
      value={zoom}
      on:input={handleZoomInput}
      on:focus={() => (editing = true)}
      on:blur={() => (editing = false)}
    />
    <label for="coord-lat">{m.lat_label()}</label>
    <input
      id="coord-lat"
      type="number"
      step="any"
      placeholder={m.lat_label()}
      value={lat}
      on:input={handleLatInput}
      on:focus={() => (editing = true)}
      on:blur={() => (editing = false)}
    />
    <label for="coord-lng">{m.lng_label()}</label>
    <input
      id="coord-lng"
      type="number"
      step="any"
      placeholder={m.lng_label()}
      value={lng}
      on:input={handleLngInput}
      on:focus={() => (editing = true)}
      on:blur={() => (editing = false)}
    />
    <button type="submit">{m.go_button()}</button>
  </form>
</div>

<style>
  .coord-form {
    background: var(--panel-bg-weak);
    padding: 6px 12px;
    border-radius: 4px;
    font-size: 16px;
    display: flex;
    align-items: center;
    gap: 6px;
    max-width: calc(100vw - 40px);
  }

  form {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
  }

  label {
    font-size: 15px;
  }

  input {
    width: 60px;
    font-size: 15px;
    padding: 2px 4px;
  }

  form button {
    font-size: 15px;
  }
</style>
