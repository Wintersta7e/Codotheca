import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { GIT_FLOOR } from './gitFloor';

// R24: a value that lives in both languages is correct only while a test reads the other
// language's source. Without this the failure window can state a floor the core does not
// enforce, which is worse than no window — the user acts on it.
const RUST = readFileSync(
  fileURLToPath(new URL('../../../core/src/git/version.rs', import.meta.url)),
  'utf8',
);

describe('the git version floor', () => {
  it('equals the constant the core actually refuses to run below', () => {
    const m = /pub const GIT_FLOOR: \(u32, u32\) = \((\d+), (\d+)\);/.exec(RUST);
    expect(
      m,
      'core/src/git/version.rs no longer declares GIT_FLOOR in the expected form',
    ).not.toBeNull();
    expect(GIT_FLOOR).toEqual([Number(m?.[1]), Number(m?.[2])]);
  });
});
