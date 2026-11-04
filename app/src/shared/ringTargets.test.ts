/**
 * §39.12's two target ranges, mirrored for the drawer's steppers. The core refuses a value outside
 * them; a stepper bounded by a different range would offer a step the core refuses, which reads as
 * a dead control. One value in two languages, so this test reads the Rust side.
 */
import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

import { required } from './required';
import {
  FOCUS_TARGET_MAX_MINUTES,
  FOCUS_TARGET_MIN_MINUTES,
  RHYTHM_TARGET_MAX_DAYS,
  RHYTHM_TARGET_MIN_DAYS,
} from './ringTargets';

function rustSource(path: string): string {
  const source = readFileSync(new URL(`../../../core/src/${path}`, import.meta.url), 'utf8');
  // An empty read would make every lookup below fail for the wrong reason, or pass vacuously.
  expect(source.length, `core/src/${path} read as empty`).toBeGreaterThan(200);
  return source;
}

/** The value of a `u32` constant as written in the named Rust file. */
function rustU32(path: string, name: string): number {
  const match = new RegExp(`const\\s+${name}\\s*:\\s*u32\\s*=\\s*([0-9_]+)`, 'u').exec(
    rustSource(path),
  );
  expect(match, `${name} is not declared in core/src/${path}`).not.toBeNull();
  return Number(required(required(match, name)[1], 'its value').replaceAll('_', ''));
}

describe('the target ranges', () => {
  it('mirror the Focus range the core validates against', () => {
    expect(rustU32('rings/mod.rs', 'FOCUS_TARGET_MIN_MINUTES')).toBe(FOCUS_TARGET_MIN_MINUTES);
    expect(rustU32('rings/mod.rs', 'FOCUS_TARGET_MAX_MINUTES')).toBe(FOCUS_TARGET_MAX_MINUTES);
  });

  it('mirror the rhythm range the core validates against', () => {
    expect(rustU32('rings/targets.rs', 'RHYTHM_TARGET_MIN')).toBe(RHYTHM_TARGET_MIN_DAYS);
    expect(rustU32('rings/targets.rs', 'RHYTHM_TARGET_MAX')).toBe(RHYTHM_TARGET_MAX_DAYS);
  });
});
