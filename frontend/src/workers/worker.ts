// src/workers/worker.ts
import { expose, transfer } from 'comlink';
import init, { get_image, init_panic_hook } from '../../pkg/wpda_wasm.js';

expose({
  init: async () => {
    await init();
    init_panic_hook();
  },
  getImage: (version: number, buffer: ArrayBuffer): ArrayBuffer => {
    // get_image takes a Uint8Array and returns a freshly-allocated PNG
    // Uint8Array (offset 0, own buffer), so .buffer is a clean ArrayBuffer.
    const png = get_image(version, new Uint8Array(buffer));
    return transfer(png.buffer, [png.buffer]);
  },
});
