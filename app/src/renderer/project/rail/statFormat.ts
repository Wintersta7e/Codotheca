/**
 * §8.5.1's primary control, as a word.
 *
 * `REFERENCE ONLY` is not among the states: §5.5 excludes a reference project from statistics,
 * not from being opened, so it gets `PLAY` like everything else.
 *
 * A control that cannot act is presented as a **statement** rather than as a disabled control —
 * the same rule that keeps Health and Remote off the tab strip and the back step off a reroll at
 * offset 0.
 */
export type CtaWord = 'PLAY' | 'RESUME' | 'REOPEN' | 'CHOOSE AN APP';

export type CtaState = { kind: 'action'; word: CtaWord } | { kind: 'statement'; text: string };

export function ctaState(args: {
  hasLiveSession: boolean;
  isArchived: boolean;
  hasPresentLocation: boolean;
  hasResolvedTarget: boolean;
}): CtaState {
  // Nothing reachable to open outranks every other word: the other three would all be lies.
  if (!args.hasPresentLocation) return { kind: 'statement', text: 'NO REACHABLE COPY' };
  if (args.hasLiveSession) return { kind: 'action', word: 'RESUME' };
  // Not a dead PLAY: the press has somewhere to go, which is the app chooser.
  if (!args.hasResolvedTarget) return { kind: 'action', word: 'CHOOSE AN APP' };
  if (args.isArchived) return { kind: 'action', word: 'REOPEN' };
  return { kind: 'action', word: 'PLAY' };
}
