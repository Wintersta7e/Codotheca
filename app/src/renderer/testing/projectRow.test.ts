import { describe, expect, it } from 'vitest';
import { makeProjectRow, notClonedRow } from './projectRow.js';

/**
 * The fixture pairing guard, TypeScript half. §23.1: `primaryLocation IS NULL` is the whole
 * predicate and `presence IS NULL` is the same predicate rendered, not a second source — so the
 * two are set together or the fixture is describing a state the product cannot produce.
 *
 * This existed because every shared renderer fixture paired a null location with `'present'`,
 * and §23.4's classifier tests the location **first**: left alone, every fixture in the tree
 * would have landed in `era:notcloned`.
 */
describe('the shared fixture builders pair location and presence', () => {
  it('gives the default row a working copy and a presence for it', () => {
    const row = makeProjectRow();
    expect(row.primaryLocation).not.toBeNull();
    expect(row.presence).not.toBeNull();
  });

  it('gives the not-cloned row neither', () => {
    const row = notClonedRow();
    expect(row.primaryLocation).toBeNull();
    expect(row.presence).toBeNull();
  });

  it('varies the pair and nothing else', () => {
    const located = makeProjectRow();
    const bare = notClonedRow();
    expect(bare.name).toBe(located.name);
    expect(bare.conditionSignal).toBeNull();
    // §23.7: `is:notcloned` needs no new projection field — `primaryLocation` already carries it.
    expect(Object.keys(bare)).toContain('primaryLocation');
  });

  it('lets a caller override, so a test can state the one field it is about', () => {
    expect(notClonedRow({ name: 'other' }).name).toBe('other');
  });
});
