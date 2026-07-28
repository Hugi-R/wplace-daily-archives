// tests/media/screenshot.test.ts
import { describe, it, expect, vi, beforeEach } from 'vitest';

vi.mock('../../src/assets/wplacearchive.js', () => ({
  wasm_screenshot: () => new Uint8Array([0x89, 0x50, 0x4e, 0x47]),
}));

import { takeScreenshot } from '../../src/media/screenshot';

describe('takeScreenshot', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('rejects when tile count exceeds 400', async () => {
    const config = {
      layer: 'tiles',
      version: 0,
      x1: 0n,
      y1: 0n,
      x2: 21n,
      y2: 21n,
    };
    await expect(takeScreenshot(config)).rejects.toThrow('Image would be too big');
  });

  it('allows exactly 400 tiles', async () => {
    const config = {
      layer: 'tiles',
      version: 0,
      x1: 0n,
      y1: 0n,
      x2: 20n,
      y2: 20n,
    };
    const result = await takeScreenshot(config);
    expect(result).toBeInstanceOf(Blob);
  });

  it('handles abort signal', async () => {
    const controller = new AbortController();
    controller.abort();
    const config = {
      layer: 'tiles',
      version: 0,
      x1: 0n,
      y1: 0n,
      x2: 1n,
      y2: 1n,
    };
    await expect(takeScreenshot(config, controller.signal)).rejects.toThrow('Aborted');
  });
});
