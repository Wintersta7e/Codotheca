/**
 * §43.2's ring table over §39's `RingsState`, one glyph per ring in §39's order. Day one is not a
 * ring status: it is `RingsState.waived`, read before any ring, and it draws no glyph — the three
 * labels and one sentence in place of their sub-lines.
 *
 * A `notComputed` ring is the `--unknown` glyph with U+2014, never an empty ring; a binary ring's
 * `open` is an empty track, which is a measured state and not an unknown.
 */
import type { RingsState, RingStatus } from '../../generated/protocol';
import {
  DAY_ONE_SENTENCE,
  RING_CLOSED_MARK,
  RING_LABELS,
  RING_NOT_APPLICABLE,
  RING_STATUS_WORDS,
  SHIP_OPEN_SUB,
  TEND_NOTHING_TO_TEND,
  UNKNOWN,
} from './copy';

export type RingKey = keyof typeof RING_LABELS;

/** What the glyph draws: an arc over the track, the track alone, or the `--unknown` fill. */
export type RingFill = 'arc' | 'track' | 'unknown';

export interface RingGlyph {
  readonly key: RingKey;
  readonly label: string;
  readonly status: RingStatus;
  readonly fill: RingFill;
  /** The arc, 0…1. */
  readonly fraction: number;
  readonly centre: string;
  /** The centre takes `--sig`: the ring is `closed`. */
  readonly closed: boolean;
  /** The centre is U+2014 in `--unknown-ink`. */
  readonly unknown: boolean;
  readonly sub: string | null;
  /** The ring's state in words, its accessible name. */
  readonly name: string;
}

export type RingsModel =
  | { readonly waived: true; readonly labels: readonly string[]; readonly sentence: string }
  | { readonly waived: false; readonly rings: readonly RingGlyph[] };

/** §39's quantity for a ring: the fraction its arc stands at while open, and its centre. */
interface Measure {
  readonly fraction: number;
  readonly figure: string;
}

/** A status the schema added after this table: fail rather than draw it blank. */
function noRendering(status: never): never {
  throw new Error(`the rings have no rendering for ${String(status)}`);
}

function glyph(key: RingKey, status: RingStatus, measure: Measure): RingGlyph {
  const label = RING_LABELS[key];
  const named = (...parts: (string | null)[]): string =>
    [label, RING_STATUS_WORDS[status], ...parts].filter((part) => part !== null).join(', ');
  const base = { key, label, status, closed: false, unknown: false, sub: null };
  switch (status) {
    case 'open': {
      // Ship's open week says what is recorded, in the sub-line and the accessible name alike.
      const sub = key === 'ship' ? SHIP_OPEN_SUB : null;
      return {
        ...base,
        fill: 'arc',
        fraction: measure.fraction,
        centre: measure.figure,
        sub,
        name: named(measure.figure, sub),
      };
    }
    case 'closed':
      return {
        ...base,
        fill: 'arc',
        fraction: 1,
        centre: measure.figure,
        closed: true,
        name: named(measure.figure),
      };
    case 'nothingToTend':
      return {
        ...base,
        fill: 'arc',
        fraction: 1,
        centre: RING_CLOSED_MARK,
        closed: true,
        sub: TEND_NOTHING_TO_TEND,
        name: named(TEND_NOTHING_TO_TEND),
      };
    case 'notApplicable':
      return { ...base, fill: 'track', fraction: 0, centre: RING_NOT_APPLICABLE, name: named() };
    case 'notComputed':
      return {
        ...base,
        fill: 'unknown',
        fraction: 0,
        centre: UNKNOWN,
        unknown: true,
        name: named(),
      };
    default:
      return noRendering(status);
  }
}

/** Focus: `min(credited, target) / target`, its centre the credited minutes. */
function focusMeasure(creditedSeconds: number, targetSeconds: number): Measure {
  const fraction = targetSeconds > 0 ? Math.min(creditedSeconds, targetSeconds) / targetSeconds : 0;
  return { fraction, figure: `${String(Math.floor(Math.max(0, creditedSeconds) / 60))}m` };
}

/** Ship and Tend are binary: one push, one earning debt day — none yet, or one. */
const binaryMeasure = (status: RingStatus): Measure =>
  status === 'closed' ? { fraction: 1, figure: '1/1' } : { fraction: 0, figure: '0/1' };

/** A ring the core left null outside day one has no reading, and is drawn as one. */
const UNREAD: Measure = { fraction: 0, figure: UNKNOWN };

export function ringsModel(rings: RingsState): RingsModel {
  if (rings.waived) {
    return {
      waived: true,
      labels: [RING_LABELS.focus, RING_LABELS.ship, RING_LABELS.tend],
      sentence: DAY_ONE_SENTENCE,
    };
  }
  const { focus, ship, tend } = rings;
  return {
    waived: false,
    rings: [
      focus === null
        ? glyph('focus', 'notComputed', UNREAD)
        : glyph('focus', focus.status, focusMeasure(focus.creditedSeconds, focus.targetSeconds)),
      ship === null
        ? glyph('ship', 'notComputed', UNREAD)
        : glyph('ship', ship.status, binaryMeasure(ship.status)),
      tend === null
        ? glyph('tend', 'notComputed', UNREAD)
        : glyph('tend', tend.status, binaryMeasure(tend.status)),
    ],
  };
}
