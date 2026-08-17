let wasmModule = null;

// Initialize WASM module when worker starts
self.onmessage = async (event) => {
    const { type, data } = event.data;

    if (type === 'init') {
        // Load WASM module once
        const { default: init, init_panic_hook, get_image} =
            await import('./wimage_wasm.js');
        await init();
        init_panic_hook();
        wasmModule = { get_image };
        self.postMessage({ type: 'ready' });
        return;
    }

    if (type === 'decompress' && wasmModule) {
        const { taskId, version, buffers } = data;
        try {
            const uint8Array = new Uint8Array(buffers[0]);
            // compressed_bytes_to_png_blob returns a Uint8Array (PNG bytes)
            const pngBytes = wasmModule.get_image(version, uint8Array);

            // Copy the Uint8Array to create a new ArrayBuffer we can transfer
            const arrayBuffer = new Uint8Array(pngBytes).buffer;
            self.postMessage({
                type: 'decompress-result',
                taskId,
                version,
                arrayBuffer,
                error: null
            }, [arrayBuffer]); // Transfer ownership
        } catch (error) {
            self.postMessage({
                type: 'decompress-result',
                taskId,
                error: error
            });
            console.error(`Worker failed decompress task ${taskId}: ${error}`);
        }
    }
};