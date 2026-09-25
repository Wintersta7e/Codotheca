import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import type { ComponentProps } from 'react';
import { afterEach, expect, test, vi } from 'vitest';
import type { Mock } from 'vitest';
import { TurnScreen } from './TurnScreen';
import { FETCH_QUALIFIER } from './copy';

afterEach(cleanup);

const OBSERVED = new Date(2026, 7, 25, 14, 32, 0).getTime() / 1000;

interface Rendered {
  readonly onShowMe: Mock<(query: string) => void>;
  readonly onNotNow: Mock<() => void>;
}

function renderTurn(over: Partial<ComponentProps<typeof TurnScreen>> = {}): Rendered {
  const onShowMe = vi.fn<(query: string) => void>();
  const onNotNow = vi.fn<() => void>();
  render(
    <TurnScreen
      counts={{ unpushed: 4, dirty: 9, interrupted: 0, total: 212 }}
      worktreeObservedAt={OBSERVED}
      tier="full"
      onShowMe={onShowMe}
      onNotNow={onNotNow}
      {...over}
    />,
  );
  return { onShowMe, onNotNow };
}

test('the line and its qualifier are both drawn', () => {
  renderTurn();
  expect(screen.getByText('You have 4 projects with unpushed work.')).toBeTruthy();
  expect(screen.getByText(FETCH_QUALIFIER)).toBeTruthy();
});

// §10.4a: rung 4 has no qualifier, and an empty element in its place would still take space.
test('rung 4 renders no qualifier element at all', () => {
  renderTurn({ counts: { unpushed: 0, dirty: 0, interrupted: 0, total: 212 } });
  expect(document.querySelector('.cdt-fr-turn-qualifier')).toBeNull();
  expect(screen.getByText('212 projects, most recently touched first.')).toBeTruthy();
});

// Criterion 12: first run asks zero configuration questions. Both questions were moved to the
// shelf (§1.4, §11.3a) precisely so this screen keeps two controls.
test('the screen carries exactly two controls and no input of any kind', () => {
  renderTurn();
  expect(screen.getAllByRole('button')).toHaveLength(2);
  expect(document.querySelectorAll('input, select, textarea')).toHaveLength(0);
});

test('SHOW ME hands up the rung its line was drawn from', () => {
  const { onShowMe } = renderTurn();
  fireEvent.click(screen.getByRole('button', { name: 'SHOW ME' }));
  expect(onShowMe).toHaveBeenCalledWith('is:unpushed');
});

// Rung 4's query is empty, and the handler still fires: `SHOW ME` on a tidy library lands on the
// unfiltered shelf rather than doing nothing.
test('rung 4 hands up an empty query rather than swallowing the press', () => {
  const { onShowMe } = renderTurn({ counts: { unpushed: 0, dirty: 0, interrupted: 0, total: 9 } });
  fireEvent.click(screen.getByRole('button', { name: 'SHOW ME' }));
  expect(onShowMe).toHaveBeenCalledWith('');
});

test('NOT NOW leaves without a query', () => {
  const { onNotNow } = renderTurn();
  fireEvent.click(screen.getByRole('button', { name: 'NOT NOW' }));
  expect(onNotNow).toHaveBeenCalledTimes(1);
});

test('focus lands on SHOW ME so the keyboard has somewhere to be', () => {
  renderTurn();
  expect(document.activeElement).toBe(screen.getByRole('button', { name: 'SHOW ME' }));
});

// §11.6: a static equivalent at every tier. The tier is on the root under the name the other
// three beats already use, so `firstRun.css`'s clamp selects this screen too.
test('the resolved tier reaches the stylesheet under the attribute the sheet reads', () => {
  renderTurn({ tier: 'off' });
  const root = document.querySelector('.cdt-fr-view--turn');
  expect(root?.getAttribute('data-effects-tier')).toBe('off');
});

// "Roasting appears only inside an opened project card", and nothing on this beat may reward
// volume: no commit count, no line count, no percentage.
test('the turn states no percentage and no volume figure', () => {
  renderTurn();
  const text = document.body.textContent;
  expect(text).not.toMatch(/%/);
  expect(text).not.toMatch(/commits?\b/i);
  expect(text).not.toMatch(/lines?\s+of\s+code/i);
});
