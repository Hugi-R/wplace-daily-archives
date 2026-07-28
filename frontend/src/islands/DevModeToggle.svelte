<!-- src/islands/DevModeToggle.svelte -->
<script lang="ts">
  import { devMode, uiVisible } from '../state/stores';
  import { syncStoresToUrl } from '../state/url-sync';
  import * as m from '../paraglide/messages.js';

  function toggle() {
    devMode.set(!$devMode);
    syncStoresToUrl();
  }
</script>

<button
  class="dev-toggle"
  class:ui-hidden={!$uiVisible}
  class:toggled={$devMode}
  on:click={toggle}
  title={m.dev_button_title()}
  aria-pressed={$devMode}
>
  {m.dev_button()}
</button>

<style>
  .dev-toggle {
    font-size: 15px;
    padding: 2px 8px;
  }
</style>
