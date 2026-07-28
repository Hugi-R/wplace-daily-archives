// src/media/video.ts
import type { VideoConfig } from './types';

const MAX_VIDEO_TILES = 64;

// eslint-disable-next-line @typescript-eslint/no-explicit-any
async function loadWasm(): Promise<any> {
  return await import('../assets/wplacearchive.js');
}

export async function takeVideo(
  config: VideoConfig,
  signal?: AbortSignal,
): Promise<Blob> {
  const tileCount =
    Number(config.x2 - config.x1) * Number(config.y2 - config.y1);
  if (tileCount > MAX_VIDEO_TILES) {
    throw new Error('Animated PNG would be too big');
  }

  if (signal?.aborted) {
    throw new DOMException('Aborted', 'AbortError');
  }

  const wasm = await loadWasm();
  const result = wasm.wasm_video(
    config.layer,
    config.x1,
    config.y1,
    config.x2,
    config.y2,
    config.from,
    config.to,
  );

  return new Blob([result]);
}
