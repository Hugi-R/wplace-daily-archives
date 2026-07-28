// Mock WASM module for development/testing
export default async function init() {}
export function init_panic_hook() {}
export function wasm_screenshot() { throw new Error("WASM not loaded"); }
export function wasm_video() { throw new Error("WASM not loaded"); }
export function get_image() { throw new Error("WASM not loaded"); }
