import { expect, test } from 'vitest';
import { TURN_QUERIES, turnLine, turnModel, worktreeQualifier, type TurnCounts } from './turn';
import { FETCH_QUALIFIER, OBSERVED_QUALIFIER } from './copy';

const OBSERVED = 1_760_000_000;

function counts(over: Partial<TurnCounts> = {}): TurnCounts {
  return { unpushed: 0, dirty: 0, interrupted: 0, total: 0, ...over };
}

// §10.4a: first rung with a count >= 1 wins, so a library with all three conditions still
// closes on the most consequential one.
test('the first rung with a count wins', () => {
  const model = turnModel(counts({ unpushed: 4, dirty: 9, interrupted: 2, total: 30 }), OBSERVED);
  expect(model.rung).toBe(1);
  expect(model.count).toBe(4);
  expect(model.line).toBe('You have 4 projects with unpushed work.');
  expect(model.query).toBe('is:unpushed');
});

test('each rung has its own line and its own query', () => {
  expect(turnModel(counts({ dirty: 3, total: 30 }), OBSERVED).line).toBe(
    'You have 3 projects with uncommitted changes.',
  );
  expect(turnModel(counts({ dirty: 3, total: 30 }), OBSERVED).query).toBe('is:dirty');
  expect(turnModel(counts({ interrupted: 2, total: 30 }), OBSERVED).line).toBe(
    'You have 2 projects with a merge or rebase left half-finished.',
  );
  expect(turnModel(counts({ interrupted: 2, total: 30 }), OBSERVED).query).toBe('is:interrupted');
});

// §10.4a: rung 4 is unconditional, and it is why a tidy library never reads `0 projects`.
test('a tidy library lands on rung 4 with an empty query', () => {
  const model = turnModel(counts({ total: 212 }), OBSERVED);
  expect(model.rung).toBe(4);
  expect(model.line).toBe('212 projects, most recently touched first.');
  expect(model.query).toBe('');
  expect(model.qualifier).toBeNull();
  expect(model.line).not.toContain('0 projects');
});

test('one collapses the plural on every rung', () => {
  expect(turnLine(1, 1)).toBe('You have 1 project with unpushed work.');
  expect(turnLine(2, 1)).toBe('You have 1 project with uncommitted changes.');
  expect(turnLine(3, 1)).toBe('You have 1 project with a merge or rebase left half-finished.');
  expect(turnLine(4, 1)).toBe('1 project, most recently touched first.');
});

// §10.4a: `ahead` is measured against the last fetch and phase 1 never fetches. Bare, rung 1 is
// the most prominent unverifiable claim in the product. R12: the string is `copy.ts`'s and this
// module states it nowhere — the test reads the owner so a second spelling cannot creep in.
test('rung 1 states that the app never fetches', () => {
  expect(turnModel(counts({ unpushed: 2, total: 9 }), OBSERVED).qualifier).toBe(FETCH_QUALIFIER);
  expect(FETCH_QUALIFIER).toBe(
    'AHEAD IS MEASURED AGAINST YOUR LAST FETCH · CODOTHECA NEVER FETCHES',
  );
});

// Criterion 23: no value is presented as current without its observation time.
test('rungs two and three carry the time the worktree was observed at', () => {
  const observedAt = new Date(2026, 7, 25, 14, 32, 0).getTime() / 1000;
  const model = turnModel(counts({ dirty: 3, total: 30 }), observedAt);
  expect(model.qualifier).toBe('AS OBSERVED AT 14:32 · WORKTREE STATE IS NEVER CACHED');
  expect(model.qualifier).toBe(OBSERVED_QUALIFIER('14:32'));
  expect(worktreeQualifier(observedAt)).toBe(model.qualifier);
});

// Criterion 23, the branch that is easy to miss: with no observation time the rung cannot be
// stated honestly, so it is skipped rather than printed without its qualifier.
test('a worktree rung with no observation time falls through instead of claiming', () => {
  const model = turnModel(counts({ dirty: 3, interrupted: 1, total: 30 }), null);
  expect(model.rung).toBe(4);
  expect(model.qualifier).toBeNull();
  expect(model.line).toBe('30 projects, most recently touched first.');
});

test('rung 1 still stands without a worktree observation, because ahead is not a worktree fact', () => {
  expect(turnModel(counts({ unpushed: 2, total: 9 }), null).rung).toBe(1);
});

test('every rung has a query and only rung 4 has none', () => {
  expect(TURN_QUERIES).toEqual({ 1: 'is:unpushed', 2: 'is:dirty', 3: 'is:interrupted', 4: '' });
});
