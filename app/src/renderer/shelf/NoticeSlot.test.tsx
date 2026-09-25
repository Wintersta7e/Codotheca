import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { NOTICE_DISMISS_LABEL, NoticeSlot } from './NoticeSlot.js';
import type { Notice, NoticeKind } from './notice.js';
import { required } from '../../shared/required.js';

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
    const hotBox = required(
      hot.container.querySelector<HTMLElement>('.cdt-shelf-notice'),
      'hot notice',
    );
    expect(hotBox.style.getPropertyValue('--cdt-notice-accent')).toBe('var(--fail-hot)');

    const warm = render(
      <NoticeSlot candidates={[notice('problems', 'r')]} dismissed={[]} onDismiss={vi.fn()} />,
    );
    const warmBox = required(
      warm.container.querySelector<HTMLElement>('.cdt-shelf-notice'),
      'warm notice',
    );
    expect(warmBox.style.getPropertyValue('--cdt-notice-accent')).toBe('var(--sig)');

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

  // §1.4's identity card is a list of tickable rows and §11.3a's residency ask is two answers
  // with no dismissal of their own. Neither fits a `body` string, and §8.0 gives one box — so
  // the owning section supplies the inside and the slot keeps the box. Without this the two
  // cards compile, pass their own tests, and can be rendered nowhere.
  it('lets the owning section supply the inside of the box', () => {
    render(
      <NoticeSlot
        candidates={[notice('identity')]}
        dismissed={[]}
        onDismiss={vi.fn()}
        renderContent={(n) => <p data-testid="card">{`inside ${n.kind}`}</p>}
      />,
    );
    expect(screen.getByTestId('card').textContent).toBe('inside identity');
    // The box is still §8.0's, and it is still a labelled region.
    expect(screen.getByRole('region', { name: 'IDENTITY' })).toBeTruthy();
    // The slot's own body and its generic DISMISS stand down: §1.4 labels its secondary
    // `LEAVE IT AS IT IS`, and a bare `notice.dismissed.residency` would suppress §11.3a's row
    // without stamping `first_run_completed_at` — the NEW chip would then never appear again.
    expect(screen.queryByText('body text')).toBeNull();
    expect(screen.queryByRole('button', { name: NOTICE_DISMISS_LABEL })).toBeNull();
  });

  it('keeps its own inside for every notice the section does not claim', () => {
    render(
      <NoticeSlot
        candidates={[notice('problems', 'r')]}
        dismissed={[]}
        onDismiss={vi.fn()}
        renderContent={(n) => (n.kind === 'identity' ? <p>never</p> : null)}
      />,
    );
    expect(screen.getByText('body text')).toBeTruthy();
    expect(screen.getByRole('button', { name: NOTICE_DISMISS_LABEL })).toBeTruthy();
  });

  it('names no destructive operation', () => {
    const { container } = render(
      <NoticeSlot candidates={[notice('problems', 'r')]} dismissed={[]} onDismiss={vi.fn()} />,
    );
    expect(container.innerHTML).not.toMatch(/FORGET/i);
    expect(NOTICE_DISMISS_LABEL).not.toMatch(/FORGET/i);
  });
});
