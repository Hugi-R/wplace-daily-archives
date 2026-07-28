// tests/media/video.test.ts
import { describe, it, expect, vi, beforeEach } from 'vitest';

vi.mock('../../src/assets/wplacearchive.js', () => ({
  wasm_video: () => new Uint8Array([0x89, 0x50, 0x4e, 0x47]),
}));

import { takeVideo } from '../../src/media/video';

describe('takeVideo', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('rejects when tile count exceeds 64', async () => {
    const config = {
      layer: 'tiles',
      x1: 0n,
      y1: 0n,
      x2: 9n,
      y2: 9n,
      from: 0,
      to: 0xFFFFFFFF,
    };
    await expect(takeVideo(config)).rejects.toThrow('Animated PNG would be too big');
  });

  it('allows exactly 64 tiles', async () => {
    const config = {
      layer: 'tiles',
      x1: 0n,
      y1: 0n,
      x2: 8n,
      y2: 8n,
      from: 0,
      to: 0xFFFFFFFF,
    };
    const result = await takeVideo(config);
    expect(result).toBeInstanceOf(Blob);
  });

  it('handles abort signal', async () => {
    const controller = new AbortController();
    controller.abort();
    const config = {
      layer: 'tiles',
      x1: 0n,
      y1: 0n,
      x2: 1n,
      y2: 1n,
      from: 0,
      to: 0xFFFFFFFF,
    };
    await expect(takeVideo(config, controller.signal)).rejects.toThrow('Aborted');
  });
});
