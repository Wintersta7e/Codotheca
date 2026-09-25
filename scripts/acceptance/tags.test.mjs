import assert from 'node:assert/strict';
import test from 'node:test';

import { TAG, TAG_P2, TAG_P3, TAG_P4, tagsIn } from './tags.mjs';

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

test('a phase-3 test name carries its criterion tag, letter and all', () => {
  // Both suffixed ids, because a test carrying one of them passes while the other is dropped —
  // and `TAG_P2`'s trailing guard would eat the letter on exactly these two names.
  assert.deepEqual(tagsIn('ac_p3_30_11a_the_suffixed_one'), ['P3-30-11a']);
  assert.ok(!tagsIn('ac_p3_30_11a_the_suffixed_one').includes('P3-30-11'));
  assert.deepEqual(tagsIn('ac_p3_28_18a_the_name_ban'), ['P3-28-18a']);
  assert.ok(!tagsIn('ac_p3_28_18a_the_name_ban').includes('P3-28-18'));
  // The bare id beside it is a different criterion and both exist.
  assert.deepEqual(tagsIn('AC-P3-30-11 the bare one'), ['P3-30-11']);
  assert.deepEqual(tagsIn('AC-P3-28-1 a debt item opens once'), ['P3-28-1']);
});

test('§36 owns no criterion and §27 is the scope section, so neither mints a tag', () => {
  assert.deepEqual(tagsIn('ac_p3_36_1_the_register'), []);
  assert.deepEqual(tagsIn('ac_p3_27_1'), []);
  // A name that names no criterion tags nothing rather than tagging the nearest one.
  assert.deepEqual(tagsIn('ac_p3_30_11abc'), []);
});

test('a name carrying all three phases carries all three tags', () => {
  assert.deepEqual(tagsIn('AC-14 and AC-P2-24-3 and AC-P3-28-1'), ['14', 'P2-24-3', 'P3-28-1']);
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

// The same blindness one phase on, both directions. A phase-3 name reached neither of the first
// two grammars, so `tagsIn` returned `[]` and the untagged net could never fire for phase 3.
test('neither earlier grammar can see a phase-3 name, and TAG_P3 sees only its own', () => {
  assert.equal(new RegExp(TAG.source, 'u').test('AC-P3-28-1'), false);
  assert.equal(new RegExp(TAG.source, 'u').test('ac_p3_28_1'), false);
  assert.equal(new RegExp(TAG_P2.source, 'u').test('AC-P3-28-1'), false);
  assert.equal(new RegExp(TAG_P3.source, 'u').test('AC-14'), false);
  assert.equal(new RegExp(TAG_P3.source, 'u').test('AC-P2-24-3'), false);
});

// [p4] The fourth grammar. Without it a phase-4 test name yields zero tags and the untagged net is
// blind to the whole phase, the blindness this file already records twice.
test('a phase-4 test name carries its criterion tag in either spelling', () => {
  assert.deepEqual(tagsIn('ac_p4_38_1_kinds'), ['P4-38-1']);
  // The trailing guard keeps a two-digit number from reading as its one-digit prefix.
  assert.deepEqual(tagsIn('ac_p4_38_10_x'), ['P4-38-10']);
  assert.ok(!tagsIn('ac_p4_38_10_x').includes('P4-38-1'));
  assert.deepEqual(tagsIn('AC-P4-42-25 the portable build'), ['P4-42-25']);
  // A node join key: the file part carries no tag, the test name does.
  assert.deepEqual(tagsIn('scripts/acceptance/release.test.mjs::ac_p4_48_27 x'), ['P4-48-27']);
});

test('§37 and §49 own no criterion and phase 4 has no letter, so none mints a tag', () => {
  assert.deepEqual(tagsIn('ac_p4_49_1'), []);
  assert.deepEqual(tagsIn('ac_p4_37_2'), []);
  assert.deepEqual(tagsIn('ac_p4_38_1a'), []);
});

test('a name carrying all four phases carries all four tags, in order', () => {
  assert.deepEqual(tagsIn('AC-14 and AC-P2-24-3 and AC-P3-28-1 and AC-P4-45-1'), [
    '14',
    'P2-24-3',
    'P3-28-1',
    'P4-45-1',
  ]);
});

test('no earlier grammar can see a phase-4 name, and TAG_P4 sees only its own', () => {
  for (const grammar of [TAG, TAG_P2, TAG_P3]) {
    assert.equal(new RegExp(grammar.source, 'u').test('AC-P4-45-1'), false, grammar.source);
    assert.equal(new RegExp(grammar.source, 'u').test('ac_p4_45_1'), false, grammar.source);
  }
  for (const name of ['AC-14', 'AC-P2-24-3', 'AC-P3-28-1']) {
    assert.equal(new RegExp(TAG_P4.source, 'u').test(name), false, name);
  }
});
