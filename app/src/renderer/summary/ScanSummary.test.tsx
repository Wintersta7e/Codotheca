import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ProblemGroup, ProblemItem, ProblemKind, Problems } from '../../generated/protocol';
import type { ErrorRequest } from '../errors/ErrorExplained';
import { ScanSummary } from './ScanSummary';
import { NO_PROBLEMS_HEADING, PROBLEM_GROUP_LABEL, PROBLEMS_DISMISSED_HEADING } from './copy';

afterEach(cleanup);

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
    problemId: null,
    ...patch,
  };
}

function group(kind: ProblemKind, items: readonly ProblemItem[]): ProblemGroup {
  return { kind, count: items.length, items };
}

function payload(
  groups: readonly ProblemGroup[],
  header: Record<string, unknown> = {},
  runId: number | null = 1,
): Problems {
  return {
    runId,
    header: {
      walkedDirs: 214_903,
      repositories: 147,
      problemCount: 9,
      ambiguousLineageCount: 2,
      ...header,
    },
    groups,
  } as unknown as Problems;
}

interface Drawn {
  readonly calls: { name: string; args: unknown }[];
  readonly onOpenProject: ReturnType<typeof vi.fn>;
  readonly onChanged: ReturnType<typeof vi.fn>;
}

function draw(
  problems: Problems,
  problemsDismissed = false,
  answer: () => Promise<unknown> = () => Promise.resolve({}),
): Drawn {
  const calls: { name: string; args: unknown }[] = [];
  const request = vi.fn((name: string, args: unknown) => {
    calls.push({ name, args });
    return answer();
  }) as unknown as ErrorRequest;
  const onOpenProject = vi.fn();
  const onChanged = vi.fn();
  render(
    <ScanSummary
      problems={problems}
      problemsDismissed={problemsDismissed}
      request={request}
      onOpenProject={onOpenProject}
      onChanged={onChanged}
    />,
  );
  return { calls, onOpenProject, onChanged };
}

const untrusted = group('untrusted_repo', [
  item({ pathDisplay: '/w/borrowed', detail: 'dubious ownership', problemId: 31 }),
]);

const ambiguous = group('ambiguous_lineage', [
  item({
    pathDisplay: '/w/twin',
    projectId: 9,
    locationId: 12,
    candidateProjectIds: [2, 3],
    candidateNames: ['Atlas', 'Borealis'],
  }),
]);

describe('the header', () => {
  it('renders the full line after a finished scan', () => {
    draw(payload([]));
    expect(screen.getByTestId('sum-header').textContent).toContain('214,903');
    expect(screen.getByTestId('sum-header').textContent).toContain('9 problems');
  });

  it('says no scan has run rather than drawing a zeroed line', () => {
    draw(
      payload(
        [],
        { walkedDirs: 0, repositories: 0, problemCount: null, ambiguousLineageCount: null },
        null,
      ),
    );
    expect(screen.getByText('NO SCAN HAS RUN YET')).toBeTruthy();
    expect(screen.queryByTestId('sum-header')).toBeNull();
  });

  it('says nothing went wrong when a finished scan found none — not an empty screen', () => {
    draw(payload([], { problemCount: 0, ambiguousLineageCount: 0 }));
    expect(screen.getByText('NOTHING WENT WRONG')).toBeTruthy();
  });
});

describe('the groups', () => {
  it('renders each group’s label with its count, in the order the core sent', () => {
    draw(payload([untrusted, ambiguous]));
    const labels = screen.getAllByTestId('sum-group-label').map((n) => n.textContent);
    expect(labels).toEqual(['UNTRUSTED REPOSITORIES 1', 'AMBIGUOUS LINEAGE 1']);
  });

  it('renders no group with no items, even if the core regresses and sends one', () => {
    draw(payload([group('clock_skew', [])]));
    expect(screen.queryByText(/CLOCK SKEW/)).toBeNull();
  });

  it('renders no detail under an offline row (criterion 63)', () => {
    draw(
      payload([
        group('offline_store', [
          item({
            pathDisplay: '/w/detached/thing',
            detail: 'volume Spare Disk not mounted',
            locationId: 5,
          }),
        ]),
      ]),
    );
    expect(screen.getByText('/w/detached/thing')).toBeTruthy();
    expect(screen.queryByText(/Spare Disk/)).toBeNull();
    expect(screen.queryByTestId('sum-detail')).toBeNull();
  });
});

describe('the two controls, and the one that does not exist', () => {
  it('trusts an untrusted row by its problem id through problems.trust and nothing else', async () => {
    const { calls, onChanged } = draw(payload([untrusted]));
    fireEvent.click(screen.getByRole('button', { name: 'TRUST THIS REPOSITORY' }));
    expect(calls).toEqual([{ name: 'problems.trust', args: { problemId: 31 } }]);
    await waitFor(() => {
      expect(onChanged).toHaveBeenCalled();
    });
  });

  it('draws no TRUST on an untrusted row the core cannot find again, location or not', () => {
    draw(payload([group('untrusted_repo', [item({ pathDisplay: '/w/x', locationId: 11 })])]));
    expect(screen.getByText('/w/x')).toBeTruthy();
    expect(screen.queryByRole('button', { name: 'TRUST THIS REPOSITORY' })).toBeNull();
  });

  it('sends one problems.trust however often it is pressed while the first is in flight', () => {
    const { calls } = draw(payload([untrusted]), false, () => new Promise(() => undefined));
    const button = screen.getByRole('button', { name: 'TRUST THIS REPOSITORY' });
    fireEvent.click(button);
    fireEvent.click(button);
    expect(calls).toHaveLength(1);
    expect(button.getAttribute('aria-disabled')).toBe('true');
  });

  it('hands focus to the next control before the trusted row leaves, else the one before', async () => {
    const two = group('untrusted_repo', [
      item({ pathDisplay: '/w/a', problemId: 31 }),
      item({ pathDisplay: '/w/b', problemId: 32 }),
    ]);
    draw(payload([two, ambiguous]));
    const [first, second] = screen.getAllByRole('button', { name: 'TRUST THIS REPOSITORY' });
    if (!first || !second) throw new Error('two TRUST buttons');
    const open = screen.getByRole('button', { name: 'OPEN PROJECT' });
    first.focus();
    fireEvent.click(first);
    await waitFor(() => {
      expect(document.activeElement).toBe(second);
    });
    second.focus();
    fireEvent.click(second);
    await waitFor(() => {
      expect(document.activeElement).toBe(open);
    });
    cleanup();

    draw(payload([ambiguous, untrusted]));
    const last = screen.getByRole('button', { name: 'TRUST THIS REPOSITORY' });
    last.focus();
    fireEvent.click(last);
    await waitFor(() => {
      expect(document.activeElement).toBe(screen.getByRole('button', { name: 'OPEN PROJECT' }));
    });
  });

  it('keeps focus and re-arms the button when the core refuses', async () => {
    const { calls, onChanged } = draw(payload([untrusted]), false, () =>
      Promise.reject(new Error('refused')),
    );
    const button = screen.getByRole('button', { name: 'TRUST THIS REPOSITORY' });
    button.focus();
    fireEvent.click(button);
    await waitFor(() => {
      expect(onChanged).toHaveBeenCalled();
    });
    expect(document.activeElement).toBe(button);
    expect(button.getAttribute('aria-disabled')).toBe('false');
    fireEvent.click(button);
    expect(calls).toHaveLength(2);
  });

  it('gives an ambiguous row OPEN PROJECT, the note and both candidate names (criterion 2)', () => {
    const { onOpenProject } = draw(payload([ambiguous]));
    const drawn = screen.getByTestId('sum-group-ambiguous_lineage');
    expect(within(drawn).getByTestId('sum-note').textContent).toContain(
      'Codotheca did not guess which',
    );
    expect(within(drawn).getByTestId('sum-detail').textContent).toBe(
      'Same history as Atlas and Borealis.',
    );
    expect(within(drawn).getByRole('button', { name: 'OPEN PROJECT' })).toBeTruthy();
    expect(onOpenProject).not.toHaveBeenCalled();
  });

  it('offers no merge anywhere — §11.1’s ruling, not an omission', () => {
    const { calls, onOpenProject } = draw(payload([untrusted, ambiguous]));
    for (const button of screen.getAllByRole('button')) {
      expect(button.textContent).not.toMatch(/merge|join|combine|forget/i);
    }
    fireEvent.click(screen.getByRole('button', { name: 'OPEN PROJECT' }));
    expect(onOpenProject).toHaveBeenCalledWith(9);
    expect(calls.some((c) => c.name === 'projects.merge')).toBe(false);
  });
});

/**
 * Ruled by the owner: dismissing the banner clears this run's list here too, because a list that
 * outlives its own dismissal reads as a control that did nothing.
 */
describe('a run whose banner was dismissed', () => {
  it('shows no group, and says they were dismissed rather than that nothing went wrong', () => {
    draw(payload([untrusted, ambiguous]), true);
    // Matched as a substring: the heading renders as `UNTRUSTED REPOSITORIES 1`, label and count
    // in one element, so an exact-text query finds nothing whether or not the group is drawn —
    // which is how the two assertions below first passed against a summary that still listed both.
    expect(screen.queryByText(new RegExp(PROBLEM_GROUP_LABEL.untrusted_repo, 'u'))).toBeNull();
    expect(screen.queryByText(new RegExp(PROBLEM_GROUP_LABEL.ambiguous_lineage, 'u'))).toBeNull();
    expect(screen.getByText(PROBLEMS_DISMISSED_HEADING)).toBeTruthy();
    // Inventing a clean scan would be the lie this heading exists to avoid.
    expect(screen.queryByText(NO_PROBLEMS_HEADING)).toBeNull();
  });

  it('drops the problem clause, because a count over an empty list is the same contradiction', () => {
    draw(payload([untrusted, ambiguous]), true);
    const header = screen.getByTestId('sum-header').textContent;
    expect(header).not.toMatch(/problem/iu);
    // What was actually walked is still true and still shown.
    expect(header).toMatch(/directories walked/u);
  });

  it('still lists them when it has not been dismissed, so the flag is what decides', () => {
    draw(payload([untrusted, ambiguous]));
    expect(screen.getByText(new RegExp(PROBLEM_GROUP_LABEL.untrusted_repo, 'u'))).toBeTruthy();
    expect(screen.queryByText(PROBLEMS_DISMISSED_HEADING)).toBeNull();
  });
});
