import { describe, expect, it } from 'vitest';
import { FrameDecoder, FrameOversizeError, MAX_FRAME_BYTES, encodeFrame } from './frame';

describe('frame codec', () => {
  it('decodes a frame split across three chunks once, whole', () => {
    const wire = encodeFrame('{"t":"hello"}');
    const d = new FrameDecoder();
    expect(d.push(wire.subarray(0, 2))).toEqual([]);
    expect(d.push(wire.subarray(2, 6))).toEqual([]);
    expect(d.push(wire.subarray(6))).toEqual(['{"t":"hello"}']);
  });

  it('yields two frames from one chunk, in order', () => {
    const wire = Buffer.concat([encodeFrame('{"a":1}'), encodeFrame('{"b":2}')]);
    expect(new FrameDecoder().push(wire)).toEqual(['{"a":1}', '{"b":2}']);
  });

  it('treats a malformed length as a protocol kill, not an allocation', () => {
    const header = Buffer.allocUnsafe(4);
    header.writeUInt32LE(0xffffffff, 0);
    const d = new FrameDecoder();
    expect(() => d.push(header)).toThrow(FrameOversizeError);
  });

  it('refuses a payload over the cap at the encoder', () => {
    expect(() => encodeFrame('x'.repeat(MAX_FRAME_BYTES + 1))).toThrow(FrameOversizeError);
  });
});
