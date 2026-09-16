import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { createRef, type ReactElement } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ProjectId, ResolvedTarget, TargetId, TargetRow } from '../../../generated/protocol';
import { ProjectPageDepsContext, type ProjectPageDeps } from '../deps';
import { NOW, targetFixture } from '../testFixtures';
import { editorTargets, OpensIn, OPENS_IN_FOOTER, tierLabel, verifyNote } from './OpensIn';

afterEach(cleanup);

const editorA = targetFixture({ id: 1 as TargetId, name: 'Editor A', sortIndex: 0 });
const editorB = targetFixture({ id: 2 as TargetId, name: 'Editor B', sortIndex: 1 });
const shell = targetFixture({ id: 3 as TargetId, kind: 'terminal', name: 'Shell', sortIndex: 0 });
const PROJECT = 7 as ProjectId;

function provide(request: ReturnType<typeof vi.fn>, children: ReactElement): ReactElement {
  const deps: ProjectPageDeps = {
    request: request as unknown as ProjectPageDeps['request'],
    relocate: () => Promise.resolve({ kind: 'cancelled' }),
    openRemoteLink: () => Promise.resolve({ kind: 'not_linkable' }),
    subscribe: () => () => undefined,
    now: () => NOW,
  };
  return <ProjectPageDepsContext.Provider value={deps}>{children}</ProjectPageDepsContext.Provider>;
}

function draw(
  over: {
    request?: ReturnType<typeof vi.fn>;
    targets?: readonly TargetRow[];
    resolved?: ResolvedTarget | null;
  } = {},
): { onChanged: ReturnType<typeof vi.fn>; request: ReturnType<typeof vi.fn> } {
  const onChanged = vi.fn();
  const request = over.request ?? vi.fn(() => Promise.resolve({}));
  render(
    provide(
      request,
      <OpensIn
        projectId={PROJECT}
        targets={over.targets ?? [editorA, editorB, shell]}
        resolved={over.resolved === undefined ? { target: editorA, tier: 'global' } : over.resolved}
        onChanged={onChanged}
      />,
    ),
  );
  return { onChanged, request };
}

describe('the target list', () => {
  it('offers editors only, in order, and does not reorder the caller’s array', () => {
    const given = [editorB, shell, editorA];
    const before = [...given];
    expect(editorTargets(given).map((t) => t.name)).toEqual(['Editor A', 'Editor B']);
    expect(given).toEqual(before);
  });

  it('labels the resolved row by the tier that resolved it', () => {
    expect(tierLabel('project')).toBe('SET');
    expect(tierLabel('location')).toBe('DEFAULT');
    expect(tierLabel('language')).toBe('DEFAULT');
    expect(tierLabel('global')).toBe('DEFAULT');
  });

  it('renders verify_state on the row, and says nothing about a verified one', () => {
    expect(verifyNote('ok')).toBeNull();
    expect(verifyNote('missing')).toBe('NOT FOUND WHERE IT WAS');
    expect(verifyNote('unverified')).toBe('NOT CHECKED YET');
  });
});

describe('the control', () => {
  it('shows the resolved target name on the closed control', () => {
    draw();
    expect(screen.getByTestId('cp-opensin-value').textContent).toBe('Editor A');
  });

  it('says CHOOSE rather than naming a target nothing resolved to', () => {
    draw({ resolved: null });
    expect(screen.getByTestId('cp-opensin-value').textContent).toBe('CHOOSE');
  });

  it('opens on press and tracks that in the accessibility tree', () => {
    draw();
    const trigger = screen.getByTestId('cp-opensin');
    expect(trigger.getAttribute('aria-expanded')).toBe('false');
    expect(screen.queryByRole('menu')).toBeNull();
    fireEvent.click(trigger);
    expect(trigger.getAttribute('aria-expanded')).toBe('true');
    expect(screen.getByRole('menu')).toBeTruthy();
  });

  it('writes the per-project override, which is the scope the footer promises', async () => {
    const { request, onChanged } = draw();
    fireEvent.click(screen.getByTestId('cp-opensin'));
    fireEvent.click(screen.getByRole('menuitem', { name: /Editor B/ }));
    await waitFor(() => {
      expect(request).toHaveBeenCalledWith('targets.setDefault', {
        targetId: editorB.id,
        projectId: PROJECT,
        locationId: null,
        language: null,
      });
    });
    await waitFor(() => {
      expect(onChanged).toHaveBeenCalled();
    });
  });

  it('marks the resolved row with its tier and leaves the others unmarked', () => {
    draw({ resolved: { target: editorA, tier: 'project' } });
    fireEvent.click(screen.getByTestId('cp-opensin'));
    expect(screen.getByRole('menuitem', { name: /Editor A/ }).textContent).toContain('SET');
    expect(screen.getByRole('menuitem', { name: /Editor B/ }).textContent).not.toContain('SET');
    expect(screen.getByRole('menuitem', { name: /Editor B/ }).textContent).not.toContain('DEFAULT');
  });

  it('carries the footer verbatim', () => {
    draw();
    fireEvent.click(screen.getByTestId('cp-opensin'));
    expect(screen.getByTestId('cp-opensin-footer').textContent).toBe(
      'SET FOR THIS PROJECT ONLY · THE DEFAULT LIVES IN SETTINGS',
    );
    expect(OPENS_IN_FOOTER).toBe('SET FOR THIS PROJECT ONLY · THE DEFAULT LIVES IN SETTINGS');
  });

  it('asserts nothing when the write does not land', async () => {
    // The control keeps showing what the core last said resolved. Reporting a change the core
    // refused would leave the rail claiming an override that does not exist.
    const request = vi.fn(() => Promise.reject(new Error('refused')));
    const { onChanged } = draw({ request });
    fireEvent.click(screen.getByTestId('cp-opensin'));
    fireEvent.click(screen.getByRole('menuitem', { name: /Editor B/ }));
    await waitFor(() => {
      expect(request).toHaveBeenCalledTimes(1);
    });
    expect(onChanged).not.toHaveBeenCalled();
    expect(screen.getByTestId('cp-opensin-value').textContent).toBe('Editor A');
  });

  it('forwards a ref to the trigger, so CHOOSE AN APP has somewhere to send focus', () => {
    const ref = createRef<HTMLButtonElement>();
    render(
      provide(
        vi.fn(() => Promise.resolve({})),
        <OpensIn
          ref={ref}
          projectId={PROJECT}
          targets={[editorA]}
          resolved={null}
          onChanged={vi.fn()}
        />,
      ),
    );
    expect(ref.current).toBe(screen.getByTestId('cp-opensin'));
    ref.current?.focus();
    expect(document.activeElement).toBe(ref.current);
  });

  it('renders nothing at all when no editor is known — never a dead control', () => {
    draw({ targets: [shell], resolved: null });
    expect(screen.queryByTestId('cp-opensin')).toBeNull();
  });
});
