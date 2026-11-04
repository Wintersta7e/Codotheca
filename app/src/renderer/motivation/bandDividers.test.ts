import { describe, expect, test } from 'vitest';

import { type BandItem, strandedDividers } from './bandDividers';

/** A band child at `top`, `height` tall; a divider when `divider` is set. */
const at = (top: number, height: number, divider = false): BandItem => ({
  divider,
  top,
  bottom: top + height,
});

describe('§43.2: a divider separates two groups only while both share its row', () => {
  test('one row: no divider is stranded', () => {
    expect(strandedDividers([at(0, 80), at(18, 44, true), at(17, 46)])).toEqual([null, null, null]);
  });

  test('the group after it wrapped: the divider closes row one', () => {
    expect(strandedDividers([at(0, 80), at(18, 44, true), at(106, 46), at(106, 56)])).toEqual([
      null,
      'end',
      null,
      null,
    ]);
  });

  test('the divider itself wrapped: it opens row two', () => {
    expect(strandedDividers([at(0, 80), at(107, 44, true), at(106, 46)])).toEqual([
      null,
      'start',
      null,
    ]);
  });

  test('a divider alone on its row opens it', () => {
    expect(strandedDividers([at(0, 80), at(106, 44, true), at(176, 46)])).toEqual([
      null,
      'start',
      null,
    ]);
  });

  test('the spacer and anything not laid out are nobody’s neighbour', () => {
    // The spacer sits between the rings and the rhythm with no height; it does not end a row.
    expect(
      strandedDividers([at(0, 80), at(18, 44, true), at(40, 0), at(17, 46), at(106, 56)]),
    ).toEqual([null, null, null, null, null]);
    // A divider before a group not laid out has nothing after it on its row.
    expect(strandedDividers([at(0, 80), at(18, 44, true), at(0, 0)])).toEqual([null, 'end', null]);
  });

  test('a divider not laid out — no layout at all, as in a DOM without one — is left alone', () => {
    expect(strandedDividers([at(0, 0), at(0, 0, true), at(0, 0)])).toEqual([null, null, null]);
  });
});
