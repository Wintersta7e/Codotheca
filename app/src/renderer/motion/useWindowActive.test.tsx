import { act, cleanup, render } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { useWindowActive } from './useWindowActive';

// Testing Library registers its own cleanup only when a global `afterEach` exists, and this
// project runs vitest with `globals: false`. Without this the first render stays in the document
// and the second test finds two `active` nodes — which reads as a query bug, not a leak.
afterEach(cleanup);

function Probe(): JSX.Element {
  return <span data-testid="active">{String(useWindowActive())}</span>;
}

describe('window activity', () => {
  it('is false once the window loses focus and true when it regains it', () => {
    // jsdom does not move `document.hasFocus()` in response to the events — it answers false
    // until something focuses the document — so the focus half of the gate is unobservable
    // without stubbing it. The production path is untouched: the hook re-reads `hasFocus()` on
    // every event either way.
    const hasFocus = vi.spyOn(document, 'hasFocus');
    const view = render(<Probe />);

    hasFocus.mockReturnValue(false);
    act(() => {
      window.dispatchEvent(new Event('blur'));
    });
    expect(view.getByTestId('active').textContent).toBe('false');

    hasFocus.mockReturnValue(true);
    act(() => {
      window.dispatchEvent(new Event('focus'));
    });
    expect(view.getByTestId('active').textContent).toBe('true');
  });

  it('is false when the document is hidden even while focused', () => {
    // "even while focused" is half the claim, so the test has to establish it. jsdom answers
    // `hasFocus()` with **false** until something focuses the document — measured, against the
    // plan's note that it defaults to true — so without this the assertion would pass on the
    // wrong reason and the visibility half would go untested.
    vi.spyOn(document, 'hasFocus').mockReturnValue(true);
    const view = render(<Probe />);
    expect(view.getByTestId('active').textContent).toBe('true');
    Object.defineProperty(document, 'visibilityState', { value: 'hidden', configurable: true });
    act(() => {
      document.dispatchEvent(new Event('visibilitychange'));
    });
    expect(view.getByTestId('active').textContent).toBe('false');
    Object.defineProperty(document, 'visibilityState', { value: 'visible', configurable: true });
    act(() => {
      document.dispatchEvent(new Event('visibilitychange'));
    });
    expect(view.getByTestId('active').textContent).toBe('true');
  });
});
