import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { type Outbound, parseOutbound } from './wire';

const SAMPLES = fileURLToPath(new URL('../../../../protocol/wire-samples.json', import.meta.url));

function outboundSamples(): unknown[] {
  const parsed: unknown = JSON.parse(readFileSync(SAMPLES, 'utf8'));
  const list: unknown = (parsed as { outbound?: unknown }).outbound;
  if (!Array.isArray(list)) throw new Error('wire-samples.json has no outbound array');
  return list;
}

describe('wire envelope', () => {
  it('narrows every golden outbound sample to a known envelope', () => {
    const list = outboundSamples();
    expect(list.length).toBe(5);
    const tags = list.map((s) => {
      const frame = parseOutbound(JSON.stringify(s));
      expect(frame).not.toBeNull();
      return (frame as Outbound).t;
    });
    expect(tags).toEqual(['hello', 'response', 'error', 'event', 'snapshot']);
  });

  it('carries an outcome on an error frame, because a lost write is not a failed one', () => {
    const raw = outboundSamples()[2];
    const frame = parseOutbound(JSON.stringify(raw));
    expect(frame?.t).toBe('error');
    if (frame?.t !== 'error') throw new Error('unreachable');
    expect(frame.code).toBe('CORE_RESTARTED');
    expect(frame.outcome).toBe('unknown');
  });

  it('rejects an unknown tag', () => {
    expect(parseOutbound('{"t":"nope"}')).toBeNull();
    expect(parseOutbound('not json')).toBeNull();
    expect(parseOutbound('null')).toBeNull();
  });
});
