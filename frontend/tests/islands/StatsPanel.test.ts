// tests/islands/StatsPanel.test.ts
import { describe, it, expect, beforeEach, afterEach, vi, afterAll } from 'vitest';
import { render, cleanup } from '@testing-library/svelte';

// Mock the protocol module
vi.mock('../../src/map/protocol', () => ({
  getMetrics: () => ({
    decompress: [10, 20, 30, 40, 50],
    'decompress-network': [5, 10, 15, 20, 25],
    'decompress-process': [3, 6, 9, 12, 15],
  }),
  computePercentiles: (data: number[]) => {
    if (data.length === 0) return null;
    const sorted = [...data].sort((a, b) => a - b);
    return {
      p10: sorted[Math.floor(0.1 * sorted.length)],
      p50: sorted[Math.floor(0.5 * sorted.length)],
      p90: sorted[Math.floor(0.9 * sorted.length)],
    };
  },
}));

// Mock nanostores with controllable stores
vi.mock('../../src/state/stores', async () => {
  const { atom } = await vi.importActual('nanostores');
  const devMode = atom(false);
  const uiVisible = atom(true);
  return {
    devMode,
    uiVisible,
  };
});

import StatsPanel from '../../src/islands/StatsPanel.svelte';
import { devMode, uiVisible } from '../../src/state/stores';

afterAll(() => {
  vi.restoreAllMocks();
  cleanup();
});

describe('StatsPanel', () => {
  beforeEach(() => {
    devMode.set(false);
    uiVisible.set(true);
  });

  afterEach(() => {
    cleanup();
  });

  it('does not render when devMode is false', () => {
    devMode.set(false);
    uiVisible.set(true);
    const { container } = render(StatsPanel);
    const statsPanel = container.querySelector('.stats-panel');
    expect(statsPanel).toBeFalsy();
  });

  it('renders when devMode is true', () => {
    devMode.set(true);
    uiVisible.set(true);
    const { container } = render(StatsPanel);
    const statsPanel = container.querySelector('.stats-panel');
    expect(statsPanel).toBeTruthy();
  });

  it('does not render when uiVisible is false', () => {
    devMode.set(true);
    uiVisible.set(false);
    const { container } = render(StatsPanel);
    const statsPanel = container.querySelector('.stats-panel');
    expect(statsPanel).toBeFalsy();
  });
});
