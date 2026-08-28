import { describe, expect, it } from 'vitest';
import {
  conditionDot,
  DOT_SIZE_PX,
  glowShadow,
  glowStrength,
  type ConditionInput,
  type ConditionSignal,
} from './condition';

const plain = (signal: ConditionSignal | null): ConditionInput => ({
  signal,
  isReference: false,
  isArchived: false,
});

describe('the condition dot', () => {
  it('draws no dot at all when nothing was measured', () => {
    // §5.4a: absence is not `empty` (measured, no commits) and not `offline` (measured, then
    // frozen). It is the only case where nothing was measured, and the honest render is none.
    expect(conditionDot(plain(null))).toBeNull();
  });

  it('renders each band exactly as the table states it', () => {
    expect(conditionDot(plain('live'))).toEqual({
      fill: '#4a9dff',
      ring: null,
      glow: '0 0 9px 0 var(--sig)',
    });
    expect(conditionDot(plain('idle'))).toEqual({ fill: '#4a9dff', ring: null, glow: null });
    expect(conditionDot(plain('dormant'))).toEqual({ fill: '#5f7285', ring: null, glow: null });
    expect(conditionDot(plain('neglected'))).toEqual({ fill: '#6c7885', ring: null, glow: null });
    expect(conditionDot(plain('abandoned'))).toEqual({ fill: '#8a6a4a', ring: null, glow: null });
  });

  it('gives offline a dark disc inside a bright ring, and empty an unfilled dashed one', () => {
    expect(conditionDot(plain('offline'))).toEqual({
      fill: '#1e262e',
      ring: '1px solid #bacede',
      glow: null,
    });
    expect(conditionDot(plain('empty'))).toEqual({
      fill: null,
      ring: '1px dashed #8b97a3',
      glow: null,
    });
  });

  it('applies reference and archived before the ladder, not as bands', () => {
    expect(conditionDot({ signal: 'live', isReference: true, isArchived: false })).toEqual({
      fill: null,
      ring: '1px solid #8b97a3',
      glow: null,
    });
    expect(conditionDot({ signal: 'abandoned', isReference: false, isArchived: true })).toEqual({
      fill: '#cfd6dc',
      ring: null,
      glow: null,
    });
  });

  it('emits no colour the table does not contain', () => {
    const table = new Set([
      '#4a9dff',
      '#5f7285',
      '#6c7885',
      '#8a6a4a',
      '#1e262e',
      '#bacede',
      '#8b97a3',
      '#cfd6dc',
    ]);
    const signals: (ConditionSignal | null)[] = [
      'live',
      'idle',
      'dormant',
      'neglected',
      'abandoned',
      'offline',
      'empty',
      null,
    ];
    for (const signal of signals) {
      for (const isReference of [false, true]) {
        for (const isArchived of [false, true]) {
          const dot = conditionDot({ signal, isReference, isArchived });
          if (dot === null) continue;
          for (const hex of `${dot.fill ?? ''} ${dot.ring ?? ''}`.match(/#[0-9a-f]{6}/g) ?? []) {
            expect(table.has(hex)).toBe(true);
          }
        }
      }
    }
  });

  it('never emits the 1.74:1 ring a superseded draft gave a never-indexed project', () => {
    const rendered = JSON.stringify(
      (['live', 'offline', 'empty', null] as (ConditionSignal | null)[]).map((s) =>
        conditionDot(plain(s)),
      ),
    );
    expect(rendered).not.toContain('#3c454e');
  });

  it('states one size per surface, and the hero is not the tile', () => {
    expect(DOT_SIZE_PX.gridTile).toBe(8);
    expect(DOT_SIZE_PX.heroTile).toBe(9);
    expect(DOT_SIZE_PX.listRow).toBe(7);
    expect(DOT_SIZE_PX.quickSwitch).toBe(7);
    expect(DOT_SIZE_PX.projectPage).toBe(9);
  });
});

describe('the glow ladder', () => {
  const g = (over: Partial<Parameters<typeof glowStrength>[0]>): number =>
    glowStrength({ ...plain('idle'), hasOpenSession: false, ...over });

  it('runs from a live session down to an abandoned floor', () => {
    expect(g({ hasOpenSession: true })).toBe(1);
    expect(g({ signal: 'live' })).toBe(0.8);
    expect(g({ signal: 'idle' })).toBe(0.5);
    expect(g({ signal: 'dormant' })).toBe(0.24);
    expect(g({ signal: 'neglected' })).toBe(0.13);
    expect(g({ signal: 'abandoned' })).toBe(0.07);
  });

  it('lets the override win over the band, which the prototype had backwards', () => {
    // The prototype evaluates the band before the floor, letting a recently-touched
    // reference project glow at 0.8. The override ordering wins.
    expect(g({ signal: 'live', isReference: true, hasOpenSession: true })).toBe(0);
    expect(g({ signal: 'live', isArchived: true })).toBe(0.13);
  });

  it('gives a project nothing measured, offline or empty no glow', () => {
    expect(g({ signal: null })).toBe(0);
    expect(g({ signal: 'offline' })).toBe(0);
    expect(g({ signal: 'empty' })).toBe(0);
  });

  it('emits the halo in the stated form', () => {
    expect(glowShadow(0.5)).toBe('0 0 21px -9px var(--sig)');
    expect(glowShadow(0)).toBe('0 0 10px -8px var(--sig)');
    expect(glowShadow(1)).toBe('0 0 32px -10px var(--sig)');
  });
});
