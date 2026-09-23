/**
 * [p3] §32.6: both sites that name the lock files are built from `shared/lockfileNames.ts`.
 *
 * The text tests beside each site pass just as well against a sentence that spells the names out
 * by hand, so they cannot say whether the list reaches the copy. Here the list is replaced, and
 * both sentences have to follow it.
 */
import { expect, test, vi } from 'vitest';

vi.mock('../../shared/lockfileNames', () => ({
  LOCKFILE_NAMES: ['first.lock', 'second.lock', 'third.lock'],
  LOCKFILE_DEPTH_TEXT: 'one directory deep',
  LOCKFILE_CAP_TEXT: '2 MB each',
}));

const { CONSENT_PARAGRAPH } = await import('./copy');
const { SCANNING_STATEMENTS } = await import('../settings/groupsScan');

test('the consent paragraph names the shared list, its depth and its cap', () => {
  expect(CONSENT_PARAGRAPH).toContain(
    'lock files — first.lock, second.lock and third.lock — up to one directory deep and up to ' +
      '2 MB each, to check',
  );
});

test('the settings row names the shared list, its depth and its cap', () => {
  const row = SCANNING_STATEMENTS.find((s) => s.label.includes('lock files'));
  expect(row?.label).toBe('Your lock files, up to one directory deep');
  expect(row?.note).toBe(
    'FIRST.LOCK · SECOND.LOCK · THIRD.LOCK · 2 MB EACH · CHECKED AGAINST PUBLISHED ADVISORIES',
  );
});
