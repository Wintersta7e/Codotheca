import { describe, expect, it } from 'vitest';
import type { CommitRef, Peek, ReadmeState } from '../../generated/protocol.js';
import { formatClock } from '../derive/observation.js';
import { formatPlaytime } from '../format/playtime.js';
import { formatTrackedBytes } from '../format/size.js';
import {
  commitDate,
  firstParagraph,
  PEEK_FACT_KEYS,
  peekFacts,
  readmeFallback,
  shortSha,
  UNCOMPUTED_FACT,
  worktreeLine,
} from './peekText.js';

const NOW = Math.floor(Date.UTC(2026, 5, 15, 14, 30) / 1000);
const readme = (state: ReadmeState['state'], text: string | null = null): ReadmeState => ({
  state,
  text,
  readAt: null,
});

describe('the two README fallbacks', () => {
  it('promises before the README job has run, and states a fact after it', () => {
    expect(readmeFallback(readme('not_indexed'))).toBe('No README indexed yet.');
    expect(readmeFallback(readme('absent'))).toBe('No README in this repository.');
  });
  it('returns null when there is text, so the text renders instead', () => {
    expect(readmeFallback(readme('present', 'hello'))).toBeNull();
  });
  it('treats whitespace as no README rather than as an empty paragraph', () => {
    expect(readmeFallback(readme('present', '   \n  '))).toBe('No README in this repository.');
  });
});

describe('firstParagraph', () => {
  it('cuts at the first blank line and collapses the wrap', () => {
    expect(firstParagraph('one\ntwo\n\nthree')).toBe('one two');
  });
  it('never interprets markup — the bytes are the text', () => {
    const src = '# Title <script>alert(1)</script> [a](b)';
    expect(firstParagraph(src)).toBe(src);
  });
});

describe('worktreeLine', () => {
  const at = NOW - 120;
  it('never says clean — absence of dirty is an observation with a time', () => {
    const line = worktreeLine({ observedAt: at, isDirty: false, untrackedCount: 0 }, NOW);
    expect(line).not.toBeNull();
    expect(line).toContain('no changes as of');
    expect(line?.toLowerCase()).not.toContain('clean');
  });
  it('renders its own age past the staleness threshold', () => {
    const stale = worktreeLine(
      { observedAt: NOW - 10_800, isDirty: false, untrackedCount: 0 },
      NOW,
    );
    expect(stale).toMatch(/observed 3h ago$/);
  });
  it('states the count when dirty, on the accessible name’s wording', () => {
    const line = worktreeLine({ observedAt: at, isDirty: true, untrackedCount: 3 }, NOW);
    // The clock comes from `formatClock`, never a literal: it renders in the reader's zone, so a
    // hard-coded `14:28` pins the machine's TZ rather than the wording (`observation.test.ts:19`).
    expect(line).toBe(`Uncommitted changes as of ${formatClock(at)} · 3 untracked`);
    expect(line).toMatch(/^Uncommitted changes as of \d{2}:\d{2} · 3 untracked$/);
  });
  it('says untracked was not counted rather than printing zero, when the job degraded', () => {
    const line = worktreeLine({ observedAt: at, isDirty: true, untrackedCount: null }, NOW);
    expect(line).toBe(`Uncommitted changes as of ${formatClock(at)} · untracked not counted`);
    expect(line).not.toMatch(/\b0 untracked\b/);
  });
  it('drops the clause on a real zero — observed, and nothing to report', () => {
    const line = worktreeLine({ observedAt: at, isDirty: true, untrackedCount: 0 }, NOW);
    expect(line).toBe(`Uncommitted changes as of ${formatClock(at)}`);
    expect(line).not.toContain('untracked');
  });
  it('renders nothing at all when nothing has been observed', () => {
    expect(worktreeLine({ observedAt: null, isDirty: null, untrackedCount: null }, NOW)).toBeNull();
  });
  it('renders nothing when the flag is NULL, however recent the observation', () => {
    // An unobserved dirty flag is not "not dirty"; a line either way would claim a currency the
    // observation does not have.
    expect(worktreeLine({ observedAt: at, isDirty: null, untrackedCount: 5 }, NOW)).toBeNull();
  });
});

describe('commit rows', () => {
  const commit = (over: Partial<CommitRef> = {}): CommitRef => ({
    sha: '9f1c2b7a4d5e6f70',
    subject: 'tighten the walk',
    at: Date.UTC(2026, 2, 4) / 1000,
    tzOffsetMin: 0,
    ...over,
  });
  it('shortens the sha to seven', () => {
    expect(shortSha(commit().sha)).toBe('9f1c2b7');
  });
  it('dates the row in the commit’s own zone, not the reader’s', () => {
    expect(commitDate(commit())).toBe('2026-03-04');
    expect(commitDate(commit({ at: Date.UTC(2026, 2, 4, 1) / 1000, tzOffsetMin: -120 }))).toBe(
      '2026-03-03',
    );
  });
});

describe('peekFacts', () => {
  // [p2] A **cloned** project with nothing computed. The fixture carried `location: null`,
  // which is now a different row shape entirely (§25.3a) — and the five-fact rule below is
  // about a project that has a working copy and no job has run over it yet.
  const base = {
    id: 1,
    readme: readme('absent'),
    commits: [],
    location: { id: 1, pathDisplay: '~/work/aurora' },
    remote: null,
    worktree: { observedAt: null, isDirty: null, untrackedCount: null },
    birthYear: null,
    primaryLanguage: null,
    sizeTrackedBytes: null,
    lastCommitAt: null,
    playtimeSeconds: 0,
    interruptedOp: null,
  } as unknown as Peek;
  const valueOf = (peek: Peek, key: string): string | undefined =>
    peekFacts(peek, NOW).find((f) => f.key === key)?.value;

  it('renders exactly five facts for a cloned row, and COMPLETION is not among them', () => {
    expect(peekFacts(base, NOW).map((f) => f.key)).toEqual([...PEEK_FACT_KEYS]);
    expect(PEEK_FACT_KEYS).toHaveLength(5);
    expect(PEEK_FACT_KEYS.join(' ')).not.toContain('COMPLETION');
  });
  it('renders an em dash for a job that has not run, never a zero', () => {
    for (const key of ['BIRTH', 'LANGUAGE', 'TRACKED', 'LAST COMMIT'] as const) {
      expect(valueOf(base, key)).toBe(UNCOMPUTED_FACT);
    }
  });
  it('renders PLAYTIME 0 as 0h — a measured zero, that ledger starts at install', () => {
    // §8.4.1's one carve-out from "never render unknown as zero": every other fact here is `—`
    // until its job has run, and this one is `0h` and true from install.
    expect(valueOf(base, 'PLAYTIME')).not.toBe(UNCOMPUTED_FACT);
    expect(valueOf(base, 'PLAYTIME')).toBe('0h');
  });
  it('labels the byte figure as tracked by using the one formatter', () => {
    const peek = { ...base, sizeTrackedBytes: 41 * 1024 ** 3 } as Peek;
    expect(valueOf(peek, 'TRACKED')).toBe(formatTrackedBytes(41 * 1024 ** 3));
    // R12's other half: the shared formatter is also the one that prints `41 GB` and not
    // `41.0 GB` — one decimal of precision, not a forced decimal place.
    expect(valueOf(peek, 'TRACKED')).toBe('41 GB');
  });
  it('prints playtime through the one playtime grammar, at every magnitude', () => {
    const peek = { ...base, playtimeSeconds: 2 * 3600 + 7 * 60 } as Peek;
    expect(valueOf(peek, 'PLAYTIME')).toBe(formatPlaytime(2 * 3600 + 7 * 60));
    expect(valueOf(peek, 'PLAYTIME')).toBe('2.1h');
  });
});

/**
 * **AC-P2-25-23, the fact-set half.** §25.3a: a not-cloned row renders a **different set**, not
 * §8.4.1's five with dashes in them.
 *
 * `PLAYTIME` is the one that leaves. `PLAYTIME 0h` is §8.4.1's one honest zero and that carve-out
 * is about a *cloned* project that was never launched; printing `0h` beside an install affordance
 * borrows it for a case it was never true of.
 */
describe('§25.3a the not-cloned fact set', () => {
  const notCloned = {
    id: 1,
    readme: readme('absent'),
    commits: [],
    location: null,
    remote: null,
    worktree: { observedAt: null, isDirty: null, untrackedCount: null },
    birthYear: null,
    primaryLanguage: null,
    sizeTrackedBytes: null,
    lastCommitAt: null,
    playtimeSeconds: 0,
    interruptedOp: null,
  } as unknown as Peek;

  it('AC-P2-25-23-facts drops PLAYTIME entirely, asserted as an absence and not as a dash', () => {
    const keys = peekFacts(notCloned, NOW).map((f) => f.key);
    expect(keys).not.toContain('PLAYTIME');
    expect(keys).toEqual(['BIRTH', 'LANGUAGE', 'TRACKED', 'LAST COMMIT']);
    expect(
      peekFacts(notCloned, NOW)
        .map((f) => f.value)
        .join(' '),
    ).not.toContain('0h');
  });

  it('renders the glyph for the three history-derived facts', () => {
    const byKey = new Map(peekFacts(notCloned, NOW).map((f) => [f.key, f.value]));
    for (const key of ['BIRTH', 'TRACKED', 'LAST COMMIT'] as const) {
      expect(byKey.get(key)).toBe(UNCOMPUTED_FACT);
    }
  });

  it('renders the forge language when observed and the glyph otherwise', () => {
    const byKey = new Map(peekFacts(notCloned, NOW).map((f) => [f.key, f.value]));
    expect(byKey.get('LANGUAGE')).toBe(UNCOMPUTED_FACT);
    const observed = { ...notCloned, primaryLanguage: 'Rust' } as Peek;
    expect(peekFacts(observed, NOW).find((f) => f.key === 'LANGUAGE')?.value).toBe('Rust');
  });
});
