/**
 * §5.4a — the condition dot, stated once for every surface that draws it, and the glow ladder.
 *
 * This module owns no day arithmetic: the band is computed by the core and arrives as
 * `condition_signal`. The design's own vocabulary (`warm`, `cooling`, `blueprint`) is stored
 * nowhere and appears nowhere here.
 */

// R31: `ConditionSignal` is declared in `protocol/schema/protocol.json` and generated into
// `app/src/generated/protocol.ts`. Re-exported rather than restated — a hand-written union
// compiles and then drifts from the core's enum the first time a band is added.
export type { ConditionSignal } from '../../generated/protocol';

import type { ConditionSignal } from '../../generated/protocol';

export interface ConditionInput {
  /** null = no scan job has produced a band. Not `empty`, not `offline`. */
  signal: ConditionSignal | null;
  isReference: boolean;
  isArchived: boolean;
}

export interface ConditionDot {
  fill: string | null;
  ring: string | null;
  glow: string | null;
}

export type DotSurface = 'gridTile' | 'heroTile' | 'listRow' | 'quickSwitch' | 'projectPage';

export const DOT_SIZE_PX: Readonly<Record<DotSurface, number>> = Object.freeze({
  gridTile: 8,
  heroTile: 9,
  listRow: 7,
  quickSwitch: 7,
  projectPage: 9,
});

const BANDS: Readonly<Record<ConditionSignal, ConditionDot>> = Object.freeze({
  live: { fill: '#4a9dff', ring: null, glow: '0 0 9px 0 var(--sig)' },
  idle: { fill: '#4a9dff', ring: null, glow: null },
  dormant: { fill: '#5f7285', ring: null, glow: null },
  neglected: { fill: '#6c7885', ring: null, glow: null },
  abandoned: { fill: '#8a6a4a', ring: null, glow: null },
  // Not the mark: the ring is, at 10.46:1. The disc exists only so the ring does not read
  // as `empty`'s unfilled one.
  offline: { fill: '#1e262e', ring: '1px solid #bacede', glow: null },
  empty: { fill: null, ring: '1px dashed #8b97a3', glow: null },
});

/** Returns null when no dot is drawn at all. The slot stays empty; §11.1's badge says why. */
export function conditionDot(input: ConditionInput): ConditionDot | null {
  if (input.isReference) {
    // A 1px ring is the thinnest mark on a card and takes the decision floor, not the 3:1 one.
    return { fill: null, ring: '1px solid #8b97a3', glow: null };
  }
  if (input.isArchived) {
    return { fill: '#cfd6dc', ring: null, glow: null };
  }
  if (input.signal === null) {
    return null;
  }
  return BANDS[input.signal];
}

const LADDER: Readonly<Record<ConditionSignal, number>> = Object.freeze({
  live: 0.8,
  idle: 0.5,
  dormant: 0.24,
  neglected: 0.13,
  abandoned: 0.07,
  offline: 0,
  empty: 0,
});

/**
 * The overrides are evaluated before the band, which the prototype had backwards: it let a
 * recently-touched reference project glow at 0.8.
 */
export function glowStrength(input: ConditionInput & { hasOpenSession: boolean }): number {
  if (input.isReference) return 0;
  if (input.isArchived) return 0.13;
  if (input.hasOpenSession) return 1;
  if (input.signal === null) return 0;
  return LADDER[input.signal];
}

/** The card's halo. §11.6 owns the flicker that dips from it; nothing here animates. */
export function glowShadow(strength: number): string {
  const blur = Math.round(10 + strength * 22);
  const spread = Math.round(-8 - strength * 2);
  return `0 0 ${blur}px ${spread}px var(--sig)`;
}
