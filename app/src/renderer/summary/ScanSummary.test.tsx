import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ProblemGroup, ProblemItem, ProblemKind, Problems } from '../../generated/protocol';
import type { ErrorRequest } from '../errors/ErrorExplained';
import { ScanSummary } from './ScanSummary';

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

function draw(problems: Problems): Drawn {
  const calls: { name: string; args: unknown }[] = [];
  const request = vi.fn((name: string, args: unknown) => {
    calls.push({ name, args });
    return Promise.resolve({} as never);
  }) as unknown as ErrorRequest;
  const onOpenProject = vi.fn();
  const onChanged = vi.fn();
  render(
    <ScanSummary
      problems={problems}
      request={request}
      onOpenProject={onOpenProject}
      onChanged={onChanged}
    />,
  );
  return { calls, onOpenProject, onChanged };
}

const untrusted = group('untrusted_repo', [
  item({ pathDisplay: '/w/borrowed', detail: 'dubious ownership', projectId: 4, locationId: 11 }),
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
  it('trusts one repository through locations.setTrusted and nothing else', async () => {
    const { calls, onChanged } = draw(payload([untrusted]));
    fireEvent.click(screen.getByRole('button', { name: 'TRUST THIS REPOSITORY' }));
    expect(calls).toEqual([{ name: 'locations.setTrusted', args: { locationId: 11 } }]);
    await waitFor(() => {
      expect(onChanged).toHaveBeenCalled();
    });
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
      expect(button.textContent ?? '').not.toMatch(/merge|join|combine|forget/i);
    }
    fireEvent.click(screen.getByRole('button', { name: 'OPEN PROJECT' }));
    expect(onOpenProject).toHaveBeenCalledWith(9);
    expect(calls.some((c) => c.name === 'projects.merge')).toBe(false);
  });
});
