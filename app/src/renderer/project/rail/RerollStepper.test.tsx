import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ProjectId } from '../../../generated/protocol';
import { ProjectPageDepsContext, type ProjectPageDeps } from '../deps';
import { NOW } from '../testFixtures';
import { nextOffset, readout, RerollStepper } from './RerollStepper';

afterEach(cleanup);

const PROJECT = 7 as ProjectId;

function draw(
  offset: number,
  reply: Record<string, unknown> = {},
): { onOffset: ReturnType<typeof vi.fn>; request: ReturnType<typeof vi.fn> } {
  const onOffset = vi.fn();
  const request = vi.fn((_name: string, args: { offset: number }) =>
    Promise.resolve({
      projectId: 7,
      offset: args.offset,
      rejected: false,
      sceneHash: 'aa',
      artState: 'ready',
      ...reply,
    }),
  );
  const deps: ProjectPageDeps = {
    request: request as unknown as ProjectPageDeps['request'],
    relocate: () => Promise.resolve({ kind: 'cancelled' }),
    subscribe: () => () => undefined,
    now: () => NOW,
  };
  render(
    <ProjectPageDepsContext.Provider value={deps}>
      <RerollStepper
        projectId={PROJECT}
        seedBasename="aurora"
        offset={offset}
        onOffset={onOffset}
      />
    </ProjectPageDepsContext.Provider>,
  );
  return { onOffset, request };
}

describe('the walk', () => {
  it('steps by one and is floored at zero, never a random draw and never a wrap', () => {
    expect(nextOffset(0, 1)).toBe(1);
    expect(nextOffset(3, -1)).toBe(2);
    expect(nextOffset(0, -1)).toBe(0);
    expect(nextOffset(4_000, 1)).toBe(4_001);
  });

  it('writes the seed string with the offset as a suffix on the hashed string', () => {
    expect(readout(0, 'aurora')).toBeNull();
    expect(readout(3, 'aurora')).toBe('aurora#3');
  });
});

describe('the control', () => {
  it('offers only the forward step at offset 0 — the back step is absent, not disabled', () => {
    draw(0);
    const forward = screen.getByTestId('cp-reroll-fwd');
    expect(forward).toBeTruthy();
    expect(forward.hasAttribute('disabled')).toBe(false);
    expect(screen.queryByTestId('cp-reroll-back')).toBeNull();
    expect(screen.queryByTestId('cp-reroll-readout')).toBeNull();
  });

  it('grows a back step and a readout once the walk has started', () => {
    draw(2);
    expect(screen.getByTestId('cp-reroll-back')).toBeTruthy();
    expect(screen.getByTestId('cp-reroll-readout').textContent).toBe('aurora#2');
  });

  it('sends the absolute target offset, never an increment', async () => {
    const { request } = draw(2);
    fireEvent.click(screen.getByTestId('cp-reroll-fwd'));
    await waitFor(() => {
      expect(request).toHaveBeenCalledWith('art.rerender', { projectId: PROJECT, offset: 3 });
    });
  });

  it('steps back to the absolute previous offset, which re-derives the same card', async () => {
    const { request } = draw(2);
    fireEvent.click(screen.getByTestId('cp-reroll-back'));
    await waitFor(() => {
      expect(request).toHaveBeenCalledWith('art.rerender', { projectId: PROJECT, offset: 1 });
    });
  });

  it('names exactly one project per call', async () => {
    const { request } = draw(0);
    fireEvent.click(screen.getByTestId('cp-reroll-fwd'));
    await waitFor(() => {
      expect(request).toHaveBeenCalledTimes(1);
    });
    const [, args] = request.mock.calls[0] as [string, Record<string, unknown>];
    expect(Object.keys(args).sort()).toEqual(['offset', 'projectId']);
  });

  it('adopts the stored offset when the core rejects the step', async () => {
    const { onOffset } = draw(9, { rejected: true, offset: 4 });
    fireEvent.click(screen.getByTestId('cp-reroll-fwd'));
    await waitFor(() => {
      expect(onOffset).toHaveBeenCalledWith(4);
    });
  });

  it('keeps the offset it had when the command fails outright', async () => {
    const request = vi.fn(() => Promise.reject(new Error('gone')));
    const deps: ProjectPageDeps = {
      request: request,
      relocate: () => Promise.resolve({ kind: 'cancelled' }),
      subscribe: () => () => undefined,
      now: () => NOW,
    };
    const onOffset = vi.fn();
    render(
      <ProjectPageDepsContext.Provider value={deps}>
        <RerollStepper projectId={PROJECT} seedBasename="aurora" offset={2} onOffset={onOffset} />
      </ProjectPageDepsContext.Provider>,
    );
    fireEvent.click(screen.getByTestId('cp-reroll-fwd'));
    await waitFor(() => {
      expect(request).toHaveBeenCalledTimes(1);
    });
    expect(onOffset).not.toHaveBeenCalled();
    expect(screen.getByTestId('cp-reroll-readout').textContent).toBe('aurora#2');
  });

  it('issues one command per press, not one per click while a press is in flight', async () => {
    let release: ((value: unknown) => void) | null = null;
    const request = vi.fn(
      () =>
        new Promise((resolve) => {
          release = resolve;
        }),
    );
    const deps: ProjectPageDeps = {
      request: request as unknown as ProjectPageDeps['request'],
      relocate: () => Promise.resolve({ kind: 'cancelled' }),
      subscribe: () => () => undefined,
      now: () => NOW,
    };
    render(
      <ProjectPageDepsContext.Provider value={deps}>
        <RerollStepper projectId={PROJECT} seedBasename="aurora" offset={0} onOffset={vi.fn()} />
      </ProjectPageDepsContext.Provider>,
    );
    fireEvent.click(screen.getByTestId('cp-reroll-fwd'));
    fireEvent.click(screen.getByTestId('cp-reroll-fwd'));
    await waitFor(() => {
      expect(request).toHaveBeenCalledTimes(1);
    });
    if (release === null) throw new Error('the request was never issued');
    (release as (value: unknown) => void)({ offset: 1, rejected: false });
  });

  it('is named for what it does, and names no other surface', () => {
    draw(2);
    expect(screen.getByTestId('cp-reroll-fwd').getAttribute('aria-label')).toBe(
      'Reroll this card art',
    );
    expect(screen.getByTestId('cp-reroll-back').getAttribute('aria-label')).toBe(
      'Step back one card art',
    );
    expect(document.body.textContent).not.toMatch(/SECTION|COLLECTION|SHELF|ALL/);
  });
});
