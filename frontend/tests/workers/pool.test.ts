// tests/workers/pool.test.ts
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';

// Mock comlink before importing pool
vi.mock('comlink', () => ({
  proxy: vi.fn((worker: unknown) =>
    Promise.resolve({
      init: vi.fn(() => Promise.resolve()),
      decompress: vi.fn((_version: number, _buffer: ArrayBuffer) => {
        return new ArrayBuffer(8);
      }),
      diffDecompress: vi.fn((_base: ArrayBuffer, _diff: ArrayBuffer) => {
        return new ArrayBuffer(8);
      }),
      downscale4to1: vi.fn(
        (_b1: ArrayBuffer, _b2: ArrayBuffer, _b3: ArrayBuffer, _b4: ArrayBuffer) => {
          return new ArrayBuffer(8);
        }
      ),
      diffDownscale4to1: vi.fn(
        (_base1: ArrayBuffer, _base2: ArrayBuffer, _base3: ArrayBuffer, _base4: ArrayBuffer,
         _diff1: ArrayBuffer, _diff2: ArrayBuffer, _diff3: ArrayBuffer, _diff4: ArrayBuffer) => {
          return new ArrayBuffer(8);
        }
      ),
    })
  ),
  transfer: vi.fn((value: ArrayBuffer) => value),
}));

// Mock the Worker constructor
const mockWorkerInstances: MockedWorker[] = [];

interface MockedWorker {
  onmessage: ((e: MessageEvent) => void) | null;
  onerror: ((e: ErrorEvent) => void) | null;
  terminate: () => void;
}

beforeEach(() => {
  mockWorkerInstances.length = 0;
  vi.stubGlobal('Worker', class {
    onmessage: ((e: MessageEvent) => void) | null = null;
    onerror: ((e: ErrorEvent) => void) | null = null;
    terminate = () => {};

    constructor() {
      mockWorkerInstances.push(this);
    }
    postMessage(_data: unknown, _transfer?: unknown[]) {}
  });
});

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe('TileWorkerPool', () => {
  it('creates pool with specified size', async () => {
    const { TileWorkerPool } = await import('../../src/workers/pool');
    const pool = new TileWorkerPool(2);
    expect(mockWorkerInstances.length).toBe(2);
    pool.terminate();
  });

  it('creates pool with default size based on hardwareConcurrency', async () => {
    Object.defineProperty(navigator, 'hardwareConcurrency', { value: 8, writable: true, configurable: true });
    const { TileWorkerPool } = await import('../../src/workers/pool');
    const pool = new TileWorkerPool();
    // Default caps at Math.min(8, 6) = 6
    expect(mockWorkerInstances.length).toBe(6);
    pool.terminate();
  });

  it('caps pool size at 6', async () => {
    Object.defineProperty(navigator, 'hardwareConcurrency', { value: 16, writable: true, configurable: true });
    const { TileWorkerPool } = await import('../../src/workers/pool');
    const pool = new TileWorkerPool();
    expect(mockWorkerInstances.length).toBe(6);
    pool.terminate();
  });

  it('terminates all workers', async () => {
    const { TileWorkerPool } = await import('../../src/workers/pool');
    const pool = new TileWorkerPool(3);
    const terminateSpy = vi.spyOn(mockWorkerInstances[0], 'terminate');
    pool.terminate();
    expect(terminateSpy).toHaveBeenCalled();
  });

  it('decompress returns a result after workers are ready', async () => {
    const { TileWorkerPool } = await import('../../src/workers/pool');
    const pool = new TileWorkerPool(1);

    // Wait for workers to initialize
    await new Promise((resolve) => setTimeout(resolve, 50));

    const buffer = new ArrayBuffer(4);
    const result = await pool.decompress(1, buffer);
    expect(result).toBeInstanceOf(ArrayBuffer);
    pool.terminate();
  });

  it('downscale4to1 validates buffer count', async () => {
    const { TileWorkerPool } = await import('../../src/workers/pool');
    const pool = new TileWorkerPool(1);

    await new Promise((resolve) => setTimeout(resolve, 50));

    await expect(pool.downscale4to1([new ArrayBuffer(4)])).rejects.toThrow(
      'downscale4to1 requires exactly 4 buffers'
    );
    pool.terminate();
  });

  it('diffDownscale4to1 validates buffer counts', async () => {
    const { TileWorkerPool } = await import('../../src/workers/pool');
    const pool = new TileWorkerPool(1);

    await new Promise((resolve) => setTimeout(resolve, 50));

    await expect(
      pool.diffDownscale4to1([new ArrayBuffer(4)], [new ArrayBuffer(4)])
    ).rejects.toThrow('diffDownscale4to1 requires 4 base buffers and 4 diff buffers');
    pool.terminate();
  });
});
