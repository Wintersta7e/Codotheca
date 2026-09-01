// `fireEvent` rather than `@testing-library/user-event`: the latter is not a dependency of this
// workspace, and one `node_modules` here cannot serve both WSL and Windows, so adding one to
// press a button is not a trade this test needs to make.
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ErrorSubject } from './errorKind';
import { ErrorExplained, type ErrorRequest } from './ErrorExplained';

afterEach(cleanup);

const NOW = 1_700_000_000;

function subject(patch: Partial<ErrorSubject> = {}): ErrorSubject {
  return {
    projectId: 7,
    locationId: 3,
    errorKind: 'UNTRUSTED_REPO',
    errorAt: NOW - 3 * 86_400,
    refstateObservedAt: null,
    worktreeObservedAt: null,
    ...patch,
  } as ErrorSubject;
}

interface Drawn {
  readonly calls: { name: string; args: unknown }[];
  readonly onChanged: ReturnType<typeof vi.fn>;
  readonly onRelocate: ReturnType<typeof vi.fn>;
}

function draw(patch: Partial<ErrorSubject> = {}, variant: 'tile' | 'page' = 'page'): Drawn {
  const calls: { name: string; args: unknown }[] = [];
  const request = vi.fn((name: string, args: unknown) => {
    calls.push({ name, args });
    return Promise.resolve({} as never);
  }) as unknown as ErrorRequest;
  const onChanged = vi.fn();
  const onRelocate = vi.fn();
  render(
    <ErrorExplained
      subject={subject(patch)}
      variant={variant}
      nowSecs={NOW}
      request={request}
      onRelocate={onRelocate}
      onChanged={onChanged}
    />,
  );
  return { calls, onChanged, onRelocate };
}

describe('what mounts at all', () => {
  it('renders nothing when there is no error', () => {
    draw({ errorKind: null });
    expect(screen.queryByTestId('err-explained')).toBeNull();
  });

  it('renders nothing for an app-level code, which never reaches project.error_kind', () => {
    draw({ errorKind: 'INTERNAL' });
    expect(screen.queryByTestId('err-explained')).toBeNull();
  });

  it('renders nothing for the git kinds — they belong to the startup surface', () => {
    draw({ errorKind: 'GIT_TOO_OLD' });
    expect(screen.queryByTestId('err-explained')).toBeNull();
  });

  it('renders nothing when the project has an observation: that is stale, not never-indexed', () => {
    draw({ errorKind: 'REPO_UNREADABLE', worktreeObservedAt: NOW - 60 });
    expect(screen.queryByTestId('err-explained')).toBeNull();
  });
});

describe('the two variants', () => {
  it('gives the tile a badge, an age and TRY AGAIN, and no sentence', () => {
    draw({}, 'tile');
    expect(screen.getByTestId('err-badge').textContent).toBe('NOT TRUSTED');
    expect(screen.getByTestId('err-last-tried').textContent).toBe('LAST TRIED 3d AGO');
    expect(screen.getByRole('button', { name: 'TRY AGAIN' })).toBeTruthy();
    expect(screen.queryByTestId('err-prose')).toBeNull();
    expect(screen.queryByRole('button', { name: 'TRUST THIS REPOSITORY' })).toBeNull();
  });

  it('gives the page the sentence and the kind’s own control', () => {
    draw({}, 'page');
    expect(screen.getByTestId('err-prose').textContent).toBe(
      "Git won't open a repository owned by another user.",
    );
    expect(screen.getByRole('button', { name: 'TRUST THIS REPOSITORY' })).toBeTruthy();
  });

  it('renders no LAST TRIED node at all when error_at is NULL — the node, not a dash', () => {
    draw({ errorAt: null }, 'page');
    expect(screen.queryByTestId('err-last-tried')).toBeNull();
  });

  it('draws no condition dot and no tier frame — §5.4a draws none, §7.7a owns the other', () => {
    draw({}, 'page');
    const block = screen.getByTestId('err-explained');
    expect(block.querySelector('[data-testid="condition-dot"]')).toBeNull();
    expect(block.querySelector('[data-testid="tier-frame"]')).toBeNull();
  });
});

describe('the controls, and criterion 63', () => {
  it('wires TRY AGAIN to projects.requeue for one project, and never to scan.start', async () => {
    const { calls, onChanged } = draw({}, 'page');
    fireEvent.click(screen.getByRole('button', { name: 'TRY AGAIN' }));
    expect(calls).toEqual([{ name: 'projects.requeue', args: { id: 7 } }]);
    expect(calls.some((c) => c.name === 'scan.start')).toBe(false);
    await waitFor(() => {
      expect(onChanged).toHaveBeenCalled();
    });
  });

  it('wires TRUST THIS REPOSITORY to locations.setTrusted for one location', () => {
    const { calls } = draw({}, 'page');
    fireEvent.click(screen.getByRole('button', { name: 'TRUST THIS REPOSITORY' }));
    expect(calls).toEqual([{ name: 'locations.setTrusted', args: { locationId: 3 } }]);
  });

  it('hands RELOCATE to the shell rather than originating a path (§2.4)', () => {
    const { calls, onRelocate } = draw({ errorKind: 'PATH_GONE' }, 'page');
    fireEvent.click(screen.getByRole('button', { name: 'RELOCATE' }));
    expect(onRelocate).toHaveBeenCalledWith(3);
    expect(calls).toEqual([]);
  });

  it('offers no trust control when there is no location to trust', () => {
    draw({ locationId: null }, 'page');
    expect(screen.queryByRole('button', { name: 'TRUST THIS REPOSITORY' })).toBeNull();
  });
});
