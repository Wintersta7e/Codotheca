import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import {
  FOCUS_HEARTBEAT_MS,
  FOCUS_HEARTBEAT_SECS,
  FOCUS_STALE_MS,
  FOCUS_STALE_SECS,
  HEARTBEATS_BEFORE_STALE,
  focusArgs,
} from './sessionFocus';
import { required } from './required';

/** The value of an `i64` constant declared in `core/src/session/mod.rs`, as written there. */
function rustSeconds(name: string): number {
  const source = readFileSync(new URL('../../../core/src/session/mod.rs', import.meta.url), 'utf8');
  const match = new RegExp(`${name}\\s*:\\s*i64\\s*=\\s*([0-9_]+)`).exec(source);
  expect(match, `${name} is not declared in core/src/session/mod.rs`).not.toBeNull();
  const digits = required(required(match, `${name} declaration`)[1], 'its value');
  return Number(digits.replaceAll('_', ''));
}

describe('the focus constants', () => {
  it('mirrors the Rust constant the core actually measures against', () => {
    // The whole reason this test exists: one value, two languages, and the project's
    // dominant defect class is exactly that.
    expect(rustSeconds('FOCUS_STALE_SECS')).toBe(FOCUS_STALE_SECS);
  });

  it('mirrors the heartbeat the core sums into its one-touch credit bound', () => {
    expect(rustSeconds('FOCUS_HEARTBEAT_SECS')).toBe(FOCUS_HEARTBEAT_SECS);
  });

  it('reads the Rust source it claims to, rather than an empty string', () => {
    // A gate whose passing run scans zero files is a failing gate: a moved or renamed Rust
    // file would make the mirror above vacuous, not red.
    const source = readFileSync(
      new URL('../../../core/src/session/mod.rs', import.meta.url),
      'utf8',
    );
    expect(source.length).toBeGreaterThan(500);
    expect(source).toContain('SEGMENT_IDLE_SECS');
  });

  it('heartbeats well inside the staleness window', () => {
    expect(HEARTBEATS_BEFORE_STALE).toBeGreaterThanOrEqual(3);
    expect(FOCUS_HEARTBEAT_SECS * HEARTBEATS_BEFORE_STALE).toBe(FOCUS_STALE_SECS);
  });

  it('cannot over-credit more than the segment idle window absorbs', () => {
    // §9: a segment closes after 20 minutes with no activity and no own-view focus. A stale
    // claim believed for FOCUS_STALE_SECS must be smaller than that, or a dead renderer would
    // hold a segment open by itself.
    expect(FOCUS_STALE_SECS).toBeLessThan(20 * 60);
  });

  it('states milliseconds derived from the seconds, never a second literal', () => {
    expect(FOCUS_STALE_MS).toBe(FOCUS_STALE_SECS * 1000);
    expect(FOCUS_HEARTBEAT_MS).toBe(FOCUS_HEARTBEAT_SECS * 1000);
  });

  it('builds the one payload both sides send', () => {
    expect(focusArgs(null)).toEqual({ projectId: null });
    expect(focusArgs(7)).toEqual({ projectId: 7 });
  });
});
