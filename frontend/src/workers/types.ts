// src/workers/types.ts
import type { Proxy } from 'comlink';

export interface TileWorkerAPI {
  init(): Promise<void>;
  decompress(version: number, buffer: Transferable): Transferable;
  diffDecompress(baseBuffer: Transferable, diffBuffer: Transferable): Transferable;
  downscale4to1(
    b1: Transferable,
    b2: Transferable,
    b3: Transferable,
    b4: Transferable
  ): Transferable;
  diffDownscale4to1(
    base1: Transferable,
    base2: Transferable,
    base3: Transferable,
    base4: Transferable,
    diff1: Transferable,
    diff2: Transferable,
    diff3: Transferable,
    diff4: Transferable
  ): Transferable;
}

export interface PoolTask {
  type: 'decompress' | 'decompress-diff' | 'downscale' | 'downscale-diff';
  buffers: ArrayBuffer[];
  resolve: (value: ArrayBuffer) => void;
  reject: (reason: Error) => void;
  signal?: AbortSignal;
}
