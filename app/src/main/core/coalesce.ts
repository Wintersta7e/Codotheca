/**
 * Batching between the main process and the renderer.
 *
 * A scan emitting thousands of individual IPC messages janks Electron regardless of encoding,
 * and the encoding is not the bottleneck (§2.3).
 */

/** One frame at the 60 Hz floor. */
export const COALESCE_WINDOW_MS = 16;

export interface Coalescer<T> {
  push(item: T): void;
  flushNow(): void;
  stop(): void;
}

export interface CoalescerOptions<T> {
  windowMs: number;
  sink: (batch: T[]) => void;
  /** Returns its own cancel function, so nothing here needs a timer handle. */
  schedule: (fn: () => void, ms: number) => () => void;
}

export function createCoalescer<T>(opts: CoalescerOptions<T>): Coalescer<T> {
  let buffer: T[] = [];
  let cancel: (() => void) | null = null;

  function flush(): void {
    cancel = null;
    if (buffer.length === 0) return;
    const batch = buffer;
    buffer = [];
    opts.sink(batch);
  }

  return {
    push(item: T): void {
      buffer.push(item);
      cancel ??= opts.schedule(flush, opts.windowMs);
    },
    flushNow(): void {
      cancel?.();
      flush();
    },
    stop(): void {
      cancel?.();
      cancel = null;
      buffer = [];
    },
  };
}
