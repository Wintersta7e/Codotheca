import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, expect, test, vi } from 'vitest';
import type { Mock } from 'vitest';
import type { IdentityConfirm, IdentityId, IdentityRow } from '../../generated/protocol';
import { IdentityCard, identityEffectLine, identitySourceLine } from './IdentityCard';
import * as copy from './copy';

afterEach(cleanup);

function identity(over: Partial<IdentityRow> = {}): IdentityRow {
  return {
    id: 1 as IdentityId,
    email: 'you@example.invalid',
    name: 'You',
    isUser: true,
    source: 'gitconfig',
    aliasReason: null,
    primaryEmail: null,
    repositories: null,
    commits: 4471,
    projects: 12,
    confirmedAt: null,
    ...over,
  };
}

interface Rendered {
  readonly onConfirm: Mock<(emails: readonly string[]) => void>;
  readonly onLeaveAsIs: Mock<() => void>;
  readonly onPreview: Mock<(emails: readonly string[]) => void>;
}

function renderCard(
  rows: readonly IdentityRow[],
  preview: IdentityConfirm | null = null,
): Rendered {
  const onConfirm = vi.fn<(emails: readonly string[]) => void>();
  const onLeaveAsIs = vi.fn<() => void>();
  const onPreview = vi.fn<(emails: readonly string[]) => void>();
  render(
    <IdentityCard
      rows={rows}
      preview={preview}
      onPreview={onPreview}
      onConfirm={onConfirm}
      onLeaveAsIs={onLeaveAsIs}
    />,
  );
  return { onConfirm, onLeaveAsIs, onPreview };
}

test('the card says what the tick already did, in §1.4 words', () => {
  renderCard([identity()]);
  expect(screen.getByText(copy.IDENTITY_TITLE)).toBeTruthy();
  expect(screen.getByText(copy.IDENTITY_BODY_1)).toBeTruthy();
  expect(screen.getByText(copy.IDENTITY_BODY_2)).toBeTruthy();
  // §1.4: beneath the actions, because the card does not come back.
  expect(screen.getByText(copy.IDENTITY_FOOTNOTE)).toBeTruthy();
});

// §1.4: the seeded set is the set already in force — the scan computed `authored_by_user` from
// it before the reveal drew a figure, so a row arriving unticked would claim the shelf behind
// the card was computed some other way.
test('every seeded row arrives ticked', () => {
  renderCard([identity(), identity({ id: 2 as IdentityId, email: 'other@example.invalid' })]);
  const ticks = screen.getAllByRole('checkbox');
  expect(ticks).toHaveLength(2);
  expect(ticks.every((tick) => (tick as HTMLInputElement).checked)).toBe(true);
});

test('unticking a row removes it from what confirm sends', () => {
  const { onConfirm } = renderCard([
    identity(),
    identity({ id: 2 as IdentityId, email: 'not-you@example.invalid' }),
  ]);
  fireEvent.click(screen.getByRole('checkbox', { name: /not-you@example\.invalid/ }));
  fireEvent.click(screen.getByRole('button', { name: copy.IDENTITY_CONFIRM_LABEL }));
  expect(onConfirm).toHaveBeenCalledWith(['you@example.invalid']);
});

// §1.4: "the statement of effect is mandatory and precedes the write" — being told afterwards
// is not being told. `identity.confirm {apply:false}` writes nothing and returns the delta, so
// a tick change asks for one.
test('a tick change asks for the preview before anything is written', () => {
  const { onPreview, onConfirm } = renderCard([
    identity(),
    identity({ id: 2 as IdentityId, email: 'not-you@example.invalid' }),
  ]);
  fireEvent.click(screen.getByRole('checkbox', { name: /not-you@example\.invalid/ }));
  expect(onPreview).toHaveBeenCalledWith(['you@example.invalid']);
  expect(onConfirm).not.toHaveBeenCalled();
});

// §1.4's row weight: "without it the row is unjudgeable — an address with four commits and one
// with nine thousand look identical, and the entire cost of unticking is which projects fall
// out". Plan 16c's body forbids this figure; the spec states it and `copy.ts` already carries
// the zero case, so the spec is what is built. Recorded as a deviation.
test('the row carries the weight that makes a tick judgeable', () => {
  renderCard([identity({ commits: 4471, projects: 12 })]);
  expect(screen.getByText(/4,471 COMMITS IN 12 PROJECTS/)).toBeTruthy();
});

// §1.4: worded rather than printed as `0 COMMITS`, because a zero beside counts in the
// thousands reads as a failed lookup. It is a real zero, not an uncomputed one.
test('an address that has authored nothing here says so in words', () => {
  renderCard([identity({ commits: 0, projects: 0 })]);
  expect(screen.getByText(new RegExp(copy.IDENTITY_NO_COMMITS))).toBeTruthy();
  expect(document.body.textContent ?? '').not.toContain('0 COMMITS');
});

test('one collapses the plural on the weight', () => {
  renderCard([identity({ commits: 1, projects: 1 })]);
  expect(screen.getByText(/1 COMMIT IN 1 PROJECT\b/)).toBeTruthy();
});

// §1.4's provenance table, row for row. The second line is what decides a tick.
test('the second line per row names why the address was seeded', () => {
  expect(identitySourceLine(identity())).toContain('GIT CONFIG · GLOBAL');
  expect(identitySourceLine(identity({ repositories: 3 }))).toContain(
    'GIT CONFIG · 3 REPOSITORIES',
  );
  expect(identitySourceLine(identity({ source: 'noreply' }))).toContain('NOREPLY ADDRESS');
  expect(
    identitySourceLine(
      identity({
        source: 'inferred',
        aliasReason: 'local_part',
        primaryEmail: 'you@example.invalid',
      }),
    ),
  ).toContain('SAME LOCAL PART AS you@example.invalid');
  expect(
    identitySourceLine(identity({ source: 'inferred', aliasReason: 'coauthor', repositories: 2 })),
  ).toContain('CO-AUTHORED WITH YOU IN 2 REPOSITORIES');
  expect(identitySourceLine(identity({ source: 'manual' }))).toContain('ADDED BY YOU');
});

// §1.4's four-row effect table. `<n>` is the previewed count and never a literal: a template
// printing a fabricated 3 for every other count is a worse statement than none.
test('the effect line states the previewed delta, and one is one', () => {
  const preview = (movedToReference: number): IdentityConfirm => ({
    movedToReference,
    commitDaysRemoved: 0,
    applied: false,
  });
  expect(identityEffectLine(preview(3), 1)).toBe('Recomputing — 3 projects moved to Reference.');
  expect(identityEffectLine(preview(1), 1)).toBe('Recomputing — 1 project moved to Reference.');
  expect(identityEffectLine(preview(0), 1)).toBe(
    'Nothing moves — no commits in this library are from that address.',
  );
  expect(identityEffectLine(preview(0), 2)).toBe(
    'Nothing moves — no commits in this library are from those addresses.',
  );
});

// §1.4: "set unchanged | none. The action confirms the set as seeded." A line about a change
// nobody made is a claim the card has no basis for.
test('an unchanged set states no effect at all', () => {
  expect(
    identityEffectLine({ movedToReference: 0, commitDaysRemoved: 0, applied: false }, 0),
  ).toBeNull();
  expect(identityEffectLine(null, 2)).toBeNull();
  renderCard([identity()]);
  expect(document.querySelector('.cdt-fr-identity-effect')).toBeNull();
});

test('the effect is drawn between the rows and the actions once it has arrived', () => {
  renderCard([identity(), identity({ id: 2 as IdentityId, email: 'not-you@example.invalid' })], {
    movedToReference: 3,
    commitDaysRemoved: 0,
    applied: false,
  });
  fireEvent.click(screen.getByRole('checkbox', { name: /not-you@example\.invalid/ }));
  expect(screen.getByText('Recomputing — 3 projects moved to Reference.')).toBeTruthy();
});

// §1.4: the secondary "is not labelled NOT NOW: this card does not come back, and a label that
// promises a later ask is a lie told in two words."
test('the second action is not a deferral', () => {
  const { onLeaveAsIs } = renderCard([identity()]);
  fireEvent.click(screen.getByRole('button', { name: copy.IDENTITY_LEAVE_LABEL }));
  expect(onLeaveAsIs).toHaveBeenCalledTimes(1);
  expect(screen.queryByRole('button', { name: 'NOT NOW' })).toBeNull();
  expect(screen.queryByRole('button', { name: /LATER/i })).toBeNull();
});

// §1.4: the card narrows and never widens — adding an address is `source = manual` and belongs
// to the settings entry, which is also the safe direction.
test('the card offers no way to add an address', () => {
  renderCard([identity()]);
  expect(screen.queryByRole('textbox')).toBeNull();
  expect(screen.queryByRole('button', { name: /ADD/i })).toBeNull();
});

// Phase 1 has no destructive operation at all: the token `FORGET` appears in no rendered
// string or accessible name, and no control here destroys anything. The body's "Nothing is
// deleted" is the opposite claim and is the one place the word belongs.
test('nothing here is destructive', () => {
  renderCard([identity()]);
  const text = document.body.textContent ?? '';
  expect(text).not.toMatch(/FORGET/i);
  for (const button of screen.getAllByRole('button')) {
    expect(button.textContent ?? '').not.toMatch(/FORGET|DELETE|REMOVE|CLEAN/i);
  }
  expect(text).toContain('Nothing is deleted.');
});
