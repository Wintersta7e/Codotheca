import type {
  CommitRef,
  Peek,
  ReadmeState,
  WorktreeObservation,
} from '../../generated/protocol.js';
import {
  formatAge,
  formatClock,
  noChangesLine,
  WORKTREE_STALE_AFTER_SECS,
} from '../derive/observation.js';
import { formatPlaytime } from '../format/playtime.js';
import { formatTrackedBytes } from '../format/size.js';

/** §8.4.1: a fact whose job has not run renders `—`, never `0`. */
export const UNCOMPUTED_FACT = '—';

export type PeekFactKey = 'BIRTH' | 'LANGUAGE' | 'TRACKED' | 'LAST COMMIT' | 'PLAYTIME';

/**
 * Five, and `COMPLETION` is not one of them: nothing in phase 1 writes `completion_lit` (§1.2),
 * so the fact would read `unknown` on 100% of rows — furniture, not honesty (§8.4.1).
 */
export const PEEK_FACT_KEYS: readonly PeekFactKey[] = [
  'BIRTH',
  'LANGUAGE',
  'TRACKED',
  'LAST COMMIT',
  'PLAYTIME',
];

/**
 * [p2] §25.3a: the keys a row of **this shape** carries.
 *
 * A project with no location drops `PLAYTIME`. `PLAYTIME 0h` is §8.4.1's one honest zero, and
 * that carve-out is about a *cloned* project that was never launched — printing `0h` beside an
 * install affordance borrows it for a case it was never true of. The other four stay and render
 * `—`, because they are history- or HEAD-derived and neither exists.
 */
export function peekFactKeys(peek: Peek): readonly PeekFactKey[] {
  return peek.location === null
    ? PEEK_FACT_KEYS.filter((key) => key !== 'PLAYTIME')
    : PEEK_FACT_KEYS;
}

export interface PeekFact {
  readonly key: PeekFactKey;
  readonly value: string;
}

/**
 * The fact: J6 has run and found nothing. §8.5.3's panel renders the same sentence, so it is a
 * constant rather than three literals — §8.1 states it and Peek is its first renderer, so this
 * is where it lives and the project page imports it (R12).
 */
export const README_ABSENT = 'No README in this repository.';

/**
 * Two fallbacks, because they are two different states (§8.4.1). One string for both renders
 * unknown as zero: the first is a promise, the second is a fact.
 */
export function readmeFallback(readme: ReadmeState): string | null {
  if (readme.state === 'not_indexed') return 'No README indexed yet.';
  if (readme.state === 'absent') return README_ABSENT;
  return readme.text === null || readme.text.trim() === '' ? README_ABSENT : null;
}

/**
 * Plain text, never rendered markup (§8.4). Nothing is stripped either — stripping markdown is
 * interpreting it, and the rule is that the bytes are the text. The core already caps the stored
 * excerpt, so the paragraph cut is the second bound, not the only one.
 */
export function firstParagraph(text: string): string {
  const [head = ''] = text.replace(/\r\n/g, '\n').split(/\n[ \t]*\n/, 1);
  return head
    .split('\n')
    .map((line) => line.trim())
    .filter(Boolean)
    .join(' ');
}

/**
 * §6 discharged on the triage surface. Absence of dirty is `no changes as of <T>` and never
 * "clean"; past the staleness threshold the line renders its own age. Dirty takes §11.7's
 * accessible-name wording verbatim so the visible string and the announced one are one string.
 */
export function worktreeLine(worktree: WorktreeObservation, now: number): string | null {
  const { observedAt, isDirty, untrackedCount } = worktree;
  if (observedAt === null || isDirty === null) return null;
  if (!isDirty) return noChangesLine(observedAt, now);

  let line = `Uncommitted changes as of ${formatClock(observedAt)}`;
  // §4.2's degraded J2 runs tracked-only and enumerates nothing, so the count is absent rather
  // than zero. Printing `0 untracked` there is the invariant's exact failure mode.
  if (untrackedCount === null) line += ' · untracked not counted';
  else if (untrackedCount > 0) line += ` · ${String(untrackedCount)} untracked`;

  const age = now - observedAt;
  return age >= WORKTREE_STALE_AFTER_SECS ? `${line} — observed ${formatAge(age)} ago` : line;
}

export function shortSha(sha: string): string {
  return sha.slice(0, 7);
}

/** The commit's own zone, which is why the wire carries `tzOffsetMin` beside `at`. */
export function commitDate(commit: CommitRef): string {
  const local = new Date((commit.at + commit.tzOffsetMin * 60) * 1000);
  const month = String(local.getUTCMonth() + 1).padStart(2, '0');
  const day = String(local.getUTCDate()).padStart(2, '0');
  return `${String(local.getUTCFullYear())}-${month}-${day}`;
}

export function peekFacts(peek: Peek, now: number): readonly PeekFact[] {
  const values: Record<PeekFactKey, string> = {
    BIRTH: peek.birthYear === null ? UNCOMPUTED_FACT : String(peek.birthYear),
    LANGUAGE: peek.primaryLanguage ?? UNCOMPUTED_FACT,
    TRACKED:
      peek.sizeTrackedBytes === null ? UNCOMPUTED_FACT : formatTrackedBytes(peek.sizeTrackedBytes),
    'LAST COMMIT':
      peek.lastCommitAt === null ? UNCOMPUTED_FACT : `${formatAge(now - peek.lastCommitAt)} ago`,
    // The one exception §8.4.1 states: this ledger starts at install, so 0 is true — and
    // `peekFactKeys` drops the key entirely for a project that was never installed.
    PLAYTIME: formatPlaytime(peek.playtimeSeconds),
  };
  return peekFactKeys(peek).map((key) => ({ key, value: values[key] }));
}
