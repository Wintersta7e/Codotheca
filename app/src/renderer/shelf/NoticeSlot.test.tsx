import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { NOTICE_DISMISS_LABEL, NoticeSlot } from './NoticeSlot.js';
import type { Notice, NoticeKind } from './notice.js';

afterEach(cleanup);

const notice = (kind: NoticeKind, scope: string | null = null): Notice => ({
  kind,
  scope,
  title: kind.toUpperCase(),
  body: 'body text',
  actions: [],
});

describe('NoticeSlot', () => {
  it('renders no wrapper at all when nothing qualifies', () => {
    // Not height:0 — the wrapper's padding would still contribute 22px of dead band.
    const { container } = render(<NoticeSlot candidates={[]} dismissed={[]} onDismiss={vi.fn()} />);
    expect(container.firstChild).toBeNull();
  });

  it('renders exactly one notice when two qualify', () => {
    const { container } = render(
      <NoticeSlot
        candidates={[notice('newArrivals', 'run-7'), notice('problems', 'run-7')]}
        dismissed={[]}
        onDismiss={vi.fn()}
      />,
    );
    expect(container.querySelectorAll('.cdt-shelf-notice')).toHaveLength(1);
    expect(screen.getByText('PROBLEMS')).toBeTruthy();
    expect(screen.queryByText('NEWARRIVALS')).toBeNull();
  });

  it('renders no wrapper when the only candidate is already dismissed', () => {
    const { container } = render(
      <NoticeSlot
        candidates={[notice('problems', 'run-7')]}
        dismissed={['notice.dismissed.problems:run-7']}
        onDismiss={vi.fn()}
      />,
    );
    expect(container.firstChild).toBeNull();
  });

  it('varies only the left border', () => {
    const hot = render(
      <NoticeSlot candidates={[notice('coreFailure')]} dismissed={[]} onDismiss={vi.fn()} />,
    );
    const hotBox = hot.container.querySelector('.cdt-shelf-notice') as HTMLElement;
    expect(hotBox.style.getPropertyValue('--notice-accent')).toBe('var(--fail-hot)');

    const warm = render(
      <NoticeSlot candidates={[notice('problems', 'r')]} dismissed={[]} onDismiss={vi.fn()} />,
    );
    const warmBox = warm.container.querySelector('.cdt-shelf-notice') as HTMLElement;
    expect(warmBox.style.getPropertyValue('--notice-accent')).toBe('var(--sig)');

    // Everything else about the two boxes is the same rule.
    expect(hotBox.className).toBe(warmBox.className);
  });

  it('offers no dismissal on the notice that has none', () => {
    render(<NoticeSlot candidates={[notice('coreFailure')]} dismissed={[]} onDismiss={vi.fn()} />);
    expect(screen.queryByRole('button', { name: /dismiss/i })).toBeNull();
  });

  it('dismisses by scoped key', () => {
    const onDismiss = vi.fn();
    render(
      <NoticeSlot
        candidates={[notice('problems', 'run-7')]}
        dismissed={[]}
        onDismiss={onDismiss}
      />,
    );
    fireEvent.click(screen.getByRole('button', { name: NOTICE_DISMISS_LABEL }));
    expect(onDismiss).toHaveBeenCalledWith('notice.dismissed.problems:run-7');
  });

  it("renders the owning section's actions as real buttons", () => {
    const run = vi.fn();
    const withAction: Notice = {
      ...notice('coreFailure'),
      actions: [{ label: 'RETRY', kind: 'primary', run }],
    };
    render(<NoticeSlot candidates={[withAction]} dismissed={[]} onDismiss={vi.fn()} />);
    const button = screen.getByRole('button', { name: 'RETRY' });
    expect(button.className).toContain('cdt-shelf-notice-primary');
    fireEvent.click(button);
    expect(run).toHaveBeenCalledOnce();
  });

  it('is a labelled region, so the slot is reachable rather than an unnamed div', () => {
    render(
      <NoticeSlot candidates={[notice('problems', 'r')]} dismissed={[]} onDismiss={vi.fn()} />,
    );
    expect(screen.getByRole('region', { name: 'PROBLEMS' })).toBeTruthy();
  });

  it('declares no colour of its own — only the accent it is handed', () => {
    // §8.7: every colour resolves to a token. The one inline value is a `var()`, not a hex.
    const { container } = render(
      <NoticeSlot candidates={[notice('problems', 'r')]} dismissed={[]} onDismiss={vi.fn()} />,
    );
    expect(container.innerHTML).not.toMatch(/#[0-9a-fA-F]{3,8}\b/);
    expect(container.innerHTML).not.toMatch(/rgba?\(/);
  });

  it('names no destructive operation', () => {
    const { container } = render(
      <NoticeSlot candidates={[notice('problems', 'r')]} dismissed={[]} onDismiss={vi.fn()} />,
    );
    expect(container.innerHTML).not.toMatch(/FORGET/i);
    expect(NOTICE_DISMISS_LABEL).not.toMatch(/FORGET/i);
  });
});
