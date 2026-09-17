import { describe, expect, it } from 'vitest';
import type { ProblemItem, Problems } from '../../generated/protocol';
import {
  AMBIGUOUS_GROUP_NOTE,
  candidateLine,
  PROBLEM_GROUP_LABEL,
  summaryDetailLine,
  summaryHeaderClauses,
  summaryHeaderLine,
} from './copy';

function problems(patch: Record<string, unknown> = {}): Problems {
  const { runId = 1, ...header } = patch;
  return {
    runId,
    header: {
      walkedDirs: 214_903,
      repositories: 147,
      problemCount: 9,
      ambiguousLineageCount: 2,
      ...header,
    },
    groups: [],
  } as unknown as Problems;
}

function item(patch: Record<string, unknown> = {}): ProblemItem {
  return {
    pathDisplay: '/w/thing',
    detail: null,
    count: 1,
    projectId: null,
    locationId: null,
    lastSeenAt: null,
    candidateProjectIds: [],
    candidateNames: [],
    ...patch,
  };
}

describe('the header line', () => {
  it('is §11.1’s four clauses, in order, with grouped figures', () => {
    expect(summaryHeaderLine(problems())).toBe(
      '214,903 directories walked · 147 repositories · 9 problems · 2 with ambiguous lineage',
    );
  });

  it('omits the problem clause while a scan is in flight — null is not zero', () => {
    expect(summaryHeaderLine(problems({ problemCount: null, ambiguousLineageCount: null }))).toBe(
      '214,903 directories walked · 147 repositories',
    );
  });

  it('renders a measured zero, because a finished scan that found none is a fact', () => {
    expect(summaryHeaderLine(problems({ problemCount: 0, ambiguousLineageCount: 0 }))).toBe(
      '214,903 directories walked · 147 repositories · 0 problems',
    );
  });

  it('removes the ambiguous clause entirely at zero (§11.1), never “0 with ambiguous lineage”', () => {
    const line = summaryHeaderLine(problems({ ambiguousLineageCount: 0 })) ?? '';
    expect(line).not.toContain('ambiguous');
  });

  it('says nothing at all when no scan has ever run — not a zeroed line', () => {
    expect(summaryHeaderLine(problems({ runId: null, walkedDirs: 0, repositories: 0 }))).toBeNull();
    expect(summaryHeaderClauses(problems({ runId: null }))).toBeNull();
  });

  it('separates the figure from its label so §11.1 can colour them differently', () => {
    const clauses = summaryHeaderClauses(problems()) ?? [];
    expect(clauses[0]).toEqual({ figure: '214,903', label: 'directories walked' });
    expect(clauses).toHaveLength(4);
  });

  it('uses one singular form per clause, so 1 repository is not “1 repositories”', () => {
    const line = summaryHeaderLine(
      problems({ repositories: 1, problemCount: 1, ambiguousLineageCount: 1 }),
    );
    expect(line).toBe(
      '214,903 directories walked · 1 repository · 1 problem · 1 with ambiguous lineage',
    );
  });
});

describe('the nine labels', () => {
  it('names every group §11.1 names, in its uppercase mono form', () => {
    expect(PROBLEM_GROUP_LABEL).toEqual({
      permission_denied: 'PERMISSION DENIED',
      untrusted_repo: 'UNTRUSTED REPOSITORIES',
      unreadable_repo: 'UNREADABLE REPOSITORIES',
      deferred_slow: 'DEFERRED-SLOW',
      clock_skew: 'CLOCK SKEW',
      non_utf8_path: 'NON-UTF-8 PATHS',
      offline_store: 'OFFLINE STORES',
      ambiguous_lineage: 'AMBIGUOUS LINEAGE',
      // [p2] §24.3c's ninth group. The record is keyed by `ProblemKind`, so this is a type error
      // until the label exists — and `toEqual` makes it a test failure until it is named here too.
      abandoned_install: 'ABANDONED INSTALLS',
    });
  });
});

describe('the detail beneath a row', () => {
  it('shows scan_problem.detail for the six groups that have one', () => {
    expect(summaryDetailLine('permission_denied', item({ detail: 'EACCES' }))).toBe('EACCES');
    expect(summaryDetailLine('clock_skew', item({ detail: null }))).toBeNull();
  });

  it('shows nothing under an offline row — criterion 63 forbids a drive or volume name', () => {
    expect(
      summaryDetailLine('offline_store', item({ detail: 'volume Backup SSD not mounted' })),
    ).toBeNull();
  });

  it('composes the ambiguous sentence from names, not from the core’s detail', () => {
    expect(
      summaryDetailLine(
        'ambiguous_lineage',
        item({
          detail: 'lineage 0xdeadbeef',
          candidateNames: ['Atlas', 'Borealis'],
          candidateProjectIds: [2, 3],
        }),
      ),
    ).toBe('Same history as Atlas and Borealis.');
  });
});

describe('the ambiguous group', () => {
  it('names two candidates outright', () => {
    expect(
      candidateLine(item({ candidateNames: ['Atlas', 'Borealis'], candidateProjectIds: [2, 3] })),
    ).toBe('Same history as Atlas and Borealis.');
  });

  it('names two and counts the rest at three or more', () => {
    expect(
      candidateLine(
        item({ candidateNames: ['Atlas', 'Borealis'], candidateProjectIds: [2, 3, 4, 5] }),
      ),
    ).toBe('Same history as Atlas, Borealis and 2 more.');
  });

  it('says nothing rather than guessing when the core sent no names', () => {
    expect(candidateLine(item())).toBeNull();
  });

  it('carries §11.1’s note verbatim, and offers no join', () => {
    expect(AMBIGUOUS_GROUP_NOTE).toContain('Codotheca did not guess which');
    expect(AMBIGUOUS_GROUP_NOTE).toContain('Joining two projects cannot be undone yet');
    expect(AMBIGUOUS_GROUP_NOTE).not.toMatch(/forget|delete|remove/i);
  });
});
