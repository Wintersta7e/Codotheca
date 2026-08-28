/**
 * The frame envelope, shell side. Mirrors `core/src/proto/wire.rs` field for field;
 * `protocol/wire-samples.json` is what proves it still does.
 */
import type { ErrorCode, Outcome, Topic } from '../../generated/protocol';

/**
 * R31: `Outcome` is generated from `protocol/schema/protocol.json` (`'unknown'`, one variant).
 * Re-exported so importers of the frames get it from the same module, and declared nowhere
 * else. `outcome: null` is §2.2's "definitely did not take effect"; there is no `'failed'`
 * value on the wire.
 */
export type { Outcome };

export interface HelloFrame {
  t: 'hello';
  protocol_version: number;
  core_version: string;
  epoch: number;
  pid: number;
}
export interface ResponseFrame {
  t: 'response';
  epoch: number;
  id: number;
  ok: unknown;
}
export interface ErrorFrame {
  t: 'error';
  epoch: number;
  id: number;
  code: ErrorCode;
  message: string;
  /** `null` — the request definitely did not take effect. `'unknown'` — it may have. */
  outcome: Outcome | null;
}
export interface EventFrame {
  t: 'event';
  epoch: number;
  topic: Topic;
  event: string;
  seq: number;
  data: unknown;
}
export interface SnapshotFrame {
  t: 'snapshot';
  epoch: number;
  topic: Topic;
  through_seq: number;
  data: unknown;
}

export type Outbound = HelloFrame | ResponseFrame | ErrorFrame | EventFrame | SnapshotFrame;

export type Inbound =
  | { t: 'request'; id: number; command: string; args: unknown }
  | { t: 'subscribe'; topic: Topic }
  | { t: 'unsubscribe'; topic: Topic }
  | { t: 'resync'; topic: Topic };

const TAGS = new Set(['hello', 'response', 'error', 'event', 'snapshot']);

/** Narrows one decoded frame body. Returns null for anything that is not a known envelope. */
export function parseOutbound(json: string): Outbound | null {
  let v: unknown;
  try {
    v = JSON.parse(json);
  } catch {
    return null;
  }
  if (typeof v !== 'object' || v === null) return null;
  const tag: unknown = (v as { t?: unknown }).t;
  if (typeof tag !== 'string' || !TAGS.has(tag)) return null;
  return v as Outbound;
}
