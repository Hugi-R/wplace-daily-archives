<!-- src/islands/StatsPanel.svelte -->
<script lang="ts">
  import { onDestroy } from 'svelte';
  import { devMode, uiVisible } from '../state/stores';
  import { getMetrics, computePercentiles } from '../map/protocol';

  interface Stats {
    decompress: ReturnType<typeof computePercentiles>;
    'decompress-network': ReturnType<typeof computePercentiles>;
    'decompress-process': ReturnType<typeof computePercentiles>;
  }

  let stats: Stats = {
    decompress: null,
    'decompress-network': null,
    'decompress-process': null,
  };

  function updateStats() {
    const metrics = getMetrics();
    stats = {
      decompress: computePercentiles(metrics.decompress ?? []),
      'decompress-network': computePercentiles(metrics['decompress-network'] ?? []),
      'decompress-process': computePercentiles(metrics['decompress-process'] ?? []),
    };
  }

  updateStats();
  const interval = setInterval(updateStats, 1000);

  onDestroy(() => clearInterval(interval));

  function formatMs(ms: number | undefined): string {
    if (ms === undefined || ms === null) return '—';
    return `${ms.toFixed(1)} ms`;
  }
</script>

{#if $devMode && $uiVisible}
<div class="stats-panel">
  <table>
    <thead>
      <tr>
        <th>Metric</th>
        <th>P10</th>
        <th>P50</th>
        <th>P90</th>
      </tr>
    </thead>
    <tbody>
      <tr>
        <td>Decompress</td>
        <td>{formatMs(stats.decompress?.p10)}</td>
        <td>{formatMs(stats.decompress?.p50)}</td>
        <td>{formatMs(stats.decompress?.p90)}</td>
      </tr>
      <tr>
        <td>Network</td>
        <td>{formatMs(stats['decompress-network']?.p10)}</td>
        <td>{formatMs(stats['decompress-network']?.p50)}</td>
        <td>{formatMs(stats['decompress-network']?.p90)}</td>
      </tr>
      <tr>
        <td>Process</td>
        <td>{formatMs(stats['decompress-process']?.p10)}</td>
        <td>{formatMs(stats['decompress-process']?.p50)}</td>
        <td>{formatMs(stats['decompress-process']?.p90)}</td>
      </tr>
    </tbody>
  </table>
</div>
{/if}

<style>
  .stats-panel {
    position: fixed;
    bottom: 1rem;
    left: 1rem;
    max-width: 300px;
    background: var(--panel-bg);
    padding: 10px;
    border-radius: 4px;
    box-shadow: var(--shadow-lg);
    font-size: 14px;
    font-family: monospace;
    z-index: 1000;
  }

  table {
    border-collapse: collapse;
  }

  th, td {
    padding: 2px 8px;
    text-align: left;
  }

  th {
    border-bottom: 1px solid var(--border-color);
  }
</style>
