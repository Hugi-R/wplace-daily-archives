// src/workers/pool.ts
import { wrap, transfer } from 'comlink';
import type { TileWorkerAPI } from './types';
import WorkerClass from './worker.ts?worker';

interface WorkerSlot {
  worker: Worker;
  api: TileWorkerAPI | null;
  ready: boolean;
  busy: boolean;
  retries: number;
}

export class TileWorkerPool {
  private slots: WorkerSlot[] = [];
  private queue: Array<{
    task: (slot: WorkerSlot) => Promise<ArrayBuffer>;
    signal?: AbortSignal;
  }> = [];
  private processingQueue = false;

  constructor(size: number = Math.min(navigator.hardwareConcurrency ?? 4, 6)) {
    for (let i = 0; i < size; i++) {
      this.addWorker();
    }
  }

  private addWorker(): void {
    const worker = new WorkerClass();

    const slot: WorkerSlot = {
      worker,
      api: null,
      ready: false,
      busy: false,
      retries: 0,
    };

    worker.onmessage = (e: MessageEvent) => {
      if (e.data?.type === 'error' || e.data?.type === 'unhandledrejection') {
        console.error('[pool] Worker diagnostic:', e.data);
      }
    };

     worker.onerror = (e: ErrorEvent) => {
      console.error(`[pool] Worker crashed (attempt ${slot.retries + 1}):`, e.message, e.filename, e.lineno);
      slot.busy = false;
      slot.ready = false;
      if (slot.retries < 3) this.respawnWorker(slot);
    };

    this.slots.push(slot);

    (async () => {
      try {
        const api = wrap(worker) as unknown as TileWorkerAPI;
        slot.api = api;
        await api.init();
        slot.ready = true;
        this.processQueue();
      } catch (e) {
        console.error(`[pool] Worker init failed (attempt ${slot.retries + 1}):`, e);
        slot.ready = false;
        if (slot.retries < 3) this.respawnWorker(slot);
      }
    })();
  }

  private respawnWorker(slot: WorkerSlot): void {
    slot.retries++;
    if (slot.retries > 3) {
      console.warn('[pool] Worker gave up after 3 retries, slot disabled');
      return;
    }

    slot.worker.terminate();
    slot.worker = new WorkerClass();

    slot.worker.onerror = (e: ErrorEvent) => {
      console.error(`[pool] Respawned worker crashed (attempt ${slot.retries}):`, e.message);
      slot.busy = false;
      slot.ready = false;
      if (slot.retries < 3) this.respawnWorker(slot);
    };

    (async () => {
      try {
        const api = wrap(slot.worker) as unknown as TileWorkerAPI;
        slot.api = api;
        await api.init();
        slot.ready = true;
        this.processQueue();
      } catch (e) {
        console.error(`[pool] Respawned worker init failed (attempt ${slot.retries}):`, e);
        slot.ready = false;
        if (slot.retries < 3) this.respawnWorker(slot);
      }
    })();
  }

  private getNextSlot(): WorkerSlot | null {
    return this.slots.find((s) => s.ready && !s.busy) ?? null;
  }

  private processQueue(): void {
    if (this.processingQueue) return;
    this.processingQueue = true;

    while (this.queue.length > 0) {
      const slot = this.getNextSlot();
      if (!slot) break;

      const item = this.queue.shift()!;

      if (item.signal?.aborted) {
        item.task(slot).catch(() => {});
        continue;
      }

      slot.busy = true;
      item
        .task(slot)
        .then((result) => {
          slot.busy = false;
          this.processQueue();
          return result;
        })
        .catch((err) => {
          slot.busy = false;
          this.processQueue();
          throw err;
        });
    }

    this.processingQueue = false;
  }

  async getImage(
    version: number,
    buffer: ArrayBuffer,
    signal?: AbortSignal,
  ): Promise<ArrayBuffer> {
    return this.enqueue(async (slot) => {
      if (!slot.api) throw new Error('No available worker');
      return slot.api.getImage(version, transfer(buffer, [buffer]));
    }, signal);
  }

  private enqueue(
    task: (slot: WorkerSlot) => Promise<ArrayBuffer>,
    signal?: AbortSignal,
  ): Promise<ArrayBuffer> {
    return new Promise<ArrayBuffer>((resolve, reject) => {
      const slot = this.getNextSlot();
      if (slot && !signal?.aborted) {
        slot.busy = true;
        task(slot)
          .then((r) => { slot.busy = false; resolve(r); })
          .catch((e) => { slot.busy = false; reject(e); });
      } else {
        this.queue.push({ task, signal });
        this.processQueue();
      }
    });
  }

  terminate(): void {
    this.slots.forEach((s) => s.worker.terminate());
  }
}
