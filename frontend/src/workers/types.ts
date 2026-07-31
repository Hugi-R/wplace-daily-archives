// src/workers/types.ts
import type { Proxy } from 'comlink';

export interface TileWorkerAPI {
  init(): Promise<void>;
  getImage(version: number, buffer: Transferable): Transferable;
}

export interface PoolTask {
  type: 'get-image';
  buffers: ArrayBuffer[];
  resolve: (value: ArrayBuffer) => void;
  reject: (reason: Error) => void;
  signal?: AbortSignal;
}
