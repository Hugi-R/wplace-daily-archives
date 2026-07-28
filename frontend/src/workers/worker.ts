// src/workers/worker.ts
import { expose } from 'comlink';

expose({
  init: async () => {},
  decompress: (_v: number, buf: ArrayBuffer) => buf,
  diffDecompress: () => { throw new Error('diffDecompress not yet implemented'); },
  downscale4to1: () => { throw new Error('downscale4to1 not yet implemented'); },
  diffDownscale4to1: () => { throw new Error('diffDownscale4to1 not yet implemented'); },
});
