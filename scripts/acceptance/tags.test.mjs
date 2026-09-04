import assert from 'node:assert/strict';
import test from 'node:test';

import { TAG, TAG_P2, tagsIn } from './tags.mjs';

test('a phase-2 test name carries its criterion tag in either spelling', () => {
  // Rust identifiers cannot hold a hyphen, so the underscore form is the same tag.
  assert.deepEqual(tagsIn('ac_p2_24_3_flag_denylist'), ['P2-24-3']);
  assert.deepEqual(tagsIn('AC-P2-24-3 the flag denylist bites'), ['P2-24-3']);
  assert.deepEqual(tagsIn('AC-P2-25-10-ddl reads the CHECK out of the migration'), ['P2-25-10']);
});

test('the phase-1 grammar is untouched', () => {
  assert.deepEqual(tagsIn('acceptance_recovery::ac_14_schema_from_the_future'), ['14']);
  assert.deepEqual(tagsIn('AC-45b the rank column contains no node'), ['45b']);
  assert.deepEqual(tagsIn('ac_12balance'), []);
  assert.deepEqual(tagsIn('nothing here'), []);
});

test('a name carrying both phases carries both tags', () => {
  assert.deepEqual(tagsIn('AC-24 and AC-P2-24-3 together'), ['24', 'P2-24-3']);
});

test('§26 owns no criterion, so no tag is minted for one', () => {
  assert.deepEqual(tagsIn('ac_p2_26_1_the_register'), []);
});

// The reason the two grammars stay two. `\bac[-_ ]?(\d…)` cannot reach past `p2`, so before
// TAG_P2 existed a phase-2 test produced **zero** tags — and `joinResults` reports a result as
// untagged only when it carries at least one tag, so the net that catches a renamed test was
// blind to the whole phase. Committed so nobody later folds them into one.
test('the phase-1 grammar cannot see a phase-2 name', () => {
  // A non-global clone: `TAG` carries `g`, and `.test` on it advances `lastIndex`.
  assert.equal(new RegExp(TAG.source, 'u').test('AC-P2-24-3'), false);
  assert.equal(new RegExp(TAG.source, 'u').test('ac_p2_24_3'), false);
  assert.equal(new RegExp(TAG_P2.source, 'u').test('AC-14'), false);
});
