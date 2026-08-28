/**
 * Frame codec, shell side. 4-byte little-endian length + UTF-8 JSON.
 *
 * Mirrors `core/src/proto/frame.rs`. The two are separately implemented on purpose: a shared
 * implementation could not exist across the language boundary, and the golden samples in
 * `protocol/wire-samples.json` are what keeps them honest.
 */
export const MAX_FRAME_BYTES = 8 * 1024 * 1024;

export class FrameOversizeError extends Error {
  constructor(readonly declared: number) {
    super(`frame length ${String(declared)} exceeds the ${String(MAX_FRAME_BYTES)}-byte cap`);
    this.name = 'FrameOversizeError';
  }
}

export function encodeFrame(payload: string): Buffer {
  const body = Buffer.from(payload, 'utf8');
  if (body.length > MAX_FRAME_BYTES) {
    throw new FrameOversizeError(body.length);
  }
  const header = Buffer.allocUnsafe(4);
  header.writeUInt32LE(body.length, 0);
  return Buffer.concat([header, body]);
}

/** Accumulates pipe chunks and yields whole frame bodies. */
export class FrameDecoder {
  private buffered: Buffer = Buffer.alloc(0);

  /**
   * Returns every complete frame in the buffer. Throws `FrameOversizeError` on a declared
   * length past the cap — the caller kills the connection; it never allocates the body.
   */
  push(chunk: Buffer): string[] {
    this.buffered = this.buffered.length === 0 ? chunk : Buffer.concat([this.buffered, chunk]);
    const out: string[] = [];
    for (;;) {
      if (this.buffered.length < 4) return out;
      const declared = this.buffered.readUInt32LE(0);
      if (declared > MAX_FRAME_BYTES) {
        this.buffered = Buffer.alloc(0);
        throw new FrameOversizeError(declared);
      }
      if (this.buffered.length < 4 + declared) return out;
      out.push(this.buffered.subarray(4, 4 + declared).toString('utf8'));
      this.buffered = this.buffered.subarray(4 + declared);
    }
  }
}
