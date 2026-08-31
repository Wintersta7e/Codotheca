import { describe, expect, it } from 'vitest';
import {
  REDUCED_CLAMP_MS,
  allowsScheduledFrames,
  allowsTransforms,
  allowsTransitions,
  allowsTravellingHighlights,
  clampToOverride,
  resolveEffectsTier,
} from './tier';

const quiet = {
  prefersReducedMotion: false,
  softwareCompositing: false,
  contextLostThisSession: false,
};

describe('auto resolves, it never stores', () => {
  it('is full on a hardware compositor with no reduced-motion preference', () => {
    expect(resolveEffectsTier('auto', quiet)).toBe('full');
  });

  it('is reduced under prefers-reduced-motion: reduce', () => {
    expect(resolveEffectsTier('auto', { ...quiet, prefersReducedMotion: true })).toBe('reduced');
  });

  it('is off on a software-rendered compositor, and off outranks reduced', () => {
    expect(resolveEffectsTier('auto', { ...quiet, softwareCompositing: true })).toBe('off');
    expect(
      resolveEffectsTier('auto', {
        ...quiet,
        softwareCompositing: true,
        prefersReducedMotion: true,
      }),
    ).toBe('off');
  });

  it('is off when a context was lost this session', () => {
    expect(resolveEffectsTier('auto', { ...quiet, contextLostThisSession: true })).toBe('off');
  });

  it('an explicit setting outranks the resolver, including the preference', () => {
    expect(resolveEffectsTier('full', { ...quiet, prefersReducedMotion: true })).toBe('full');
    expect(resolveEffectsTier('reduced', quiet)).toBe('reduced');
    expect(resolveEffectsTier('off', quiet)).toBe('off');
  });
});

describe('the reduced-motion override clamps the tier to at most reduced', () => {
  it('clamps down and never up', () => {
    expect(clampToOverride('full', true)).toBe('reduced');
    expect(clampToOverride('reduced', true)).toBe('reduced');
    expect(clampToOverride('off', true)).toBe('off');
    expect(clampToOverride('full', false)).toBe('full');
  });
});

describe('what each tier permits', () => {
  it('off has no transition and no animation of any property', () => {
    expect(allowsTransitions('off')).toBe(false);
    expect(allowsTransforms('off')).toBe(false);
    expect(allowsTravellingHighlights('off')).toBe(false);
    expect(allowsScheduledFrames('off')).toBe(false);
  });

  it('reduced allows opacity and colour transitions only, clamped to 160 ms', () => {
    expect(REDUCED_CLAMP_MS).toBe(160);
    expect(allowsTransitions('reduced')).toBe(true);
    expect(allowsTransforms('reduced')).toBe(false);
    expect(allowsTravellingHighlights('reduced')).toBe(false);
    expect(allowsScheduledFrames('reduced')).toBe(false);
  });

  it('the scheduled frame is permitted at full and nowhere else', () => {
    expect(allowsScheduledFrames('full')).toBe(true);
    expect(allowsTravellingHighlights('full')).toBe(true);
    expect(allowsTransforms('full')).toBe(true);
  });
});
