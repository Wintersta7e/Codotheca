import { describe, expect, it } from 'vitest';
import { ctaState } from './statFormat';

const base = {
  hasLiveSession: false,
  isArchived: false,
  hasPresentLocation: true,
  hasResolvedTarget: true,
};

describe('the primary control', () => {
  it('reads PLAY, RESUME while a session is live, REOPEN when archived', () => {
    expect(ctaState(base)).toEqual({ kind: 'action', word: 'PLAY' });
    expect(ctaState({ ...base, isArchived: true })).toEqual({ kind: 'action', word: 'REOPEN' });
    expect(ctaState({ ...base, hasLiveSession: true })).toEqual({ kind: 'action', word: 'RESUME' });
    expect(ctaState({ ...base, hasLiveSession: true, isArchived: true })).toEqual({
      kind: 'action',
      word: 'RESUME',
    });
  });

  it('offers a choice rather than a dead PLAY when nothing resolves', () => {
    expect(ctaState({ ...base, hasResolvedTarget: false })).toEqual({
      kind: 'action',
      word: 'CHOOSE AN APP',
    });
  });

  it('becomes a statement, not a disabled control, when no copy is reachable', () => {
    expect(ctaState({ ...base, hasPresentLocation: false })).toEqual({
      kind: 'statement',
      text: 'NO REACHABLE COPY',
    });
    // Unreachable outranks every other word, including a live session's.
    expect(
      ctaState({ ...base, hasPresentLocation: false, hasLiveSession: true, isArchived: true }),
    ).toEqual({ kind: 'statement', text: 'NO REACHABLE COPY' });
  });

  it('offers no REFERENCE ONLY: a reference project is excluded from statistics, not from opening', () => {
    const words = [
      ctaState(base),
      ctaState({ ...base, isArchived: true }),
      ctaState({ ...base, hasLiveSession: true }),
      ctaState({ ...base, hasResolvedTarget: false }),
      ctaState({ ...base, hasPresentLocation: false }),
    ].map((state) => (state.kind === 'action' ? state.word : state.text));
    expect(words.join(' ')).not.toMatch(/REFERENCE|CLONE|RESTORE|GITHUB/);
  });
});
