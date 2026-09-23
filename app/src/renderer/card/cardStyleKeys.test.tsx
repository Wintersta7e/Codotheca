/**
 * The frame gap and the gold notch take their fill from an inline style, and React names style
 * keys in camelCase: a hyphenated key draws `Unsupported style property` and is the kind a later
 * React may stop applying. The fill has to arrive, and arrive without the warning.
 *
 * A file of its own because React warns once per key per module instance — in a file that has
 * already drawn a card, the warning is spent before this test starts.
 */
import { cleanup, render } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { appearanceFor } from '../art/appearance';
import { Card, type CardProps } from './Card';
import { uncomputedRank } from './completion';

afterEach(cleanup);

const props = (over: Partial<CardProps['bands']>, notched: boolean): CardProps => ({
  surface: 'card',
  appearance: appearanceFor({ seedBasename: 'atlas', rerollOffset: 0 }, 0, 'Rust'),
  frameToken: 'unknown',
  density: 186,
  isArchived: false,
  isReference: false,
  art: null,
  notched,
  bands: {
    languageCode: 'RS',
    designation: 'RS-42 / MK-III',
    hazard: false,
    conditionSignal: 'dormant',
    rank: null,
    chips: [],
    pin: null,
    ...over,
  },
  halo: { shadow: 'none', opacity: 1 },
  hovered: false,
  focused: false,
  selected: false,
  children: null,
});

it('fills the gap and the gold notch through a style key React supports', () => {
  const errors = vi.spyOn(console, 'error').mockImplementation(() => undefined);
  const rank = uncomputedRank('gridCard', {
    completionLit: null,
    isReference: false,
    hasWorkingCopy: true,
    density: 186,
  });
  const gap = render(<Card {...props({ rank }, false)} />).container.querySelector<HTMLElement>(
    '.cdt-frame-gap',
  );
  const notch = render(<Card {...props({}, true)} />).container.querySelector<HTMLElement>(
    '.cdt-frame-notch',
  );

  expect(gap?.style.getPropertyValue('background-color')).toBe('var(--surface-1)');
  expect(notch?.style.getPropertyValue('background-color')).toBe('var(--surface-1)');
  const unsupported = errors.mock.calls.filter((call) =>
    call.some((arg) => String(arg).includes('Unsupported style property')),
  );
  expect(unsupported).toEqual([]);
});
