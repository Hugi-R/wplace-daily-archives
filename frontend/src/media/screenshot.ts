// src/media/screenshot.ts
import type { ScreenshotConfig } from './types';

const MAX_SCREENSHOT_TILES = 400;

// eslint-disable-next-line @typescript-eslint/no-explicit-any
async function loadWasm(): Promise<any> {
  return await import('../assets/wplacearchive.js');
}

export async function takeScreenshot(
  config: ScreenshotConfig,
  signal?: AbortSignal,
): Promise<Blob> {
  const tileCount =
    Number(config.x2 - config.x1) * Number(config.y2 - config.y1);
  if (tileCount > MAX_SCREENSHOT_TILES) {
    throw new Error('Image would be too big');
  }

  if (signal?.aborted) {
    throw new DOMException('Aborted', 'AbortError');
  }

  const wasm = await loadWasm();
  const result = wasm.wasm_screenshot(
    config.layer,
    config.version,
    config.x1,
    config.y1,
    config.x2,
    config.y2,
  );

  return new Blob([result]);
}

export function downloadBlob(blob: Blob, filename: string): void {
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url;
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  document.body.removeChild(a);
  URL.revokeObjectURL(url);
}
