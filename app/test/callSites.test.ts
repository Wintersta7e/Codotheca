import { spawnSync } from 'node:child_process';
import { existsSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

const REPO = fileURLToPath(new URL('../..', import.meta.url));
const SCRIPT = join(REPO, 'scripts/check-call-sites.mjs');
const RULES = join(REPO, 'acceptance/callsites.json');
const RULE_ID = 'no-unaudited-git-spawn';

/**
 * The three non-git spawns, each with the line its `allowLines` entry must match.
 *
 * Deliberately **not** paired with `allowLines` by array index. Pairing by position assumes the
 * JSON's order matches this file's, so reordering the JSON would silently re-point every
 * justification at a different line while the test stayed green — a test bound to a list position
 * rather than to the property that picks the element. Each anchor is instead required to be
 * matched by *some* entry, with the counts asserted equal so an entry that justifies nothing
 * cannot hide either.
 */
const NON_GIT_SPAWNS = [
  { path: 'core/src/launch/spawn.rs', line: 37, program: 'an editor the user chose' },
  { path: 'core/src/launch/probe_linux.rs', line: 22, program: 'a launch probe' },
  { path: 'core/src/wsl/distros.rs', line: 111, program: 'wsl.exe' },
] as const;

/** Only the two fields this file joins on; the register's own validator owns the rest. */
interface RegisteredCheck {
  readonly id: string;
  readonly test?: string;
}

interface CallSiteRule {
  readonly id: string;
  readonly patterns: readonly string[];
  readonly allow: readonly string[];
  readonly allowLines?: readonly string[];
  readonly pendingRegistryEntry?: { readonly criterion?: string };
}

interface CallSiteRegistry {
  readonly rules: readonly CallSiteRule[];
}

function rule(): CallSiteRule {
  const registry = JSON.parse(readFileSync(RULES, 'utf8')) as CallSiteRegistry;
  const found = registry.rules.find(({ id }) => id === RULE_ID);
  if (found === undefined) {
    throw new Error(`${RULE_ID} is not declared in acceptance/callsites.json`);
  }
  return found;
}

function lineAt(path: string, line: number): string {
  return readFileSync(join(REPO, path), 'utf8').split('\n')[line - 1] ?? '';
}

describe('the call-site gate', () => {
  it('reports a non-empty clean scan', () => {
    const result = spawnSync(process.execPath, [SCRIPT], { cwd: REPO, encoding: 'utf8' });
    const output = String(result.stdout) + String(result.stderr);
    expect(result.status, output).toBe(0);
    const match = /check-call-sites: \d+ rules, 0 violated, (\d+) file reads/.exec(output);
    expect(match, output).not.toBeNull();
    expect(Number(match?.[1] ?? 0)).toBeGreaterThan(0);
  });

  it('pins the three audited git spawn files, each of which exists', () => {
    const { allow } = rule();
    expect(allow).toHaveLength(3);
    for (const allowed of allow) {
      expect(existsSync(join(REPO, allowed)), `${allowed} must exist`).toBe(true);
    }
  });

  /**
   * Half this codebase's spawns are written `std::process::Command::new(…)` and half are bare
   * `Command::new(…)`. The rule's pattern is a substring and catches both; a matcher that saw
   * only one spelling would let four real spawns past while reporting green, including the one
   * that launches `wsl.exe`.
   */
  it('matches both spellings of a spawn, not just the bare one', () => {
    const [pattern] = rule().patterns;
    expect(pattern).toBeDefined();
    const re = new RegExp(pattern ?? '', 'u');
    expect(re.test('let mut command = Command::new(&inv.program);')).toBe(true);
    expect(re.test('let out = std::process::Command::new(program)')).toBe(true);
  });

  it('keeps every non-git spawn exception attached to its real line', () => {
    const { allowLines = [] } = rule();
    expect(allowLines).toHaveLength(NON_GIT_SPAWNS.length);

    for (const source of allowLines) {
      expect(() => new RegExp(source, 'u')).not.toThrow();
    }

    const unmatched = new Set(allowLines);
    for (const { path, line, program } of NON_GIT_SPAWNS) {
      const text = lineAt(path, line);
      const hit = allowLines.find((source) => new RegExp(source, 'u').test(text));
      expect(
        hit,
        `${path}:${line} (${program}) is matched by no allowLines entry: ${text}`,
      ).toBeDefined();
      if (hit !== undefined) unmatched.delete(hit);
    }

    expect(
      [...unmatched],
      'every allowLines entry must justify one of the three real spawns; an entry matching none of them permits a line nobody enumerated',
    ).toEqual([]);
  });

  /**
   * `validateCallSites` returns EXIT_CANNOT_RUN for a rule no criterion claims, so this rule
   * shipped naming the criterion it belonged to until the register held the real check. Now that
   * `AC-P2-24-3` is registered the escape is discharged, and a rule carrying **both** an escape
   * and a claiming check is a rule with two answers.
   */
  it('carries no escape, now that a registered check claims it', () => {
    expect(rule().pendingRegistryEntry).toBeUndefined();
    const registry = JSON.parse(readFileSync(join(REPO, 'acceptance/criteria.json'), 'utf8')) as {
      readonly criteria: readonly { readonly checks: readonly RegisteredCheck[] }[];
    };
    const claims = registry.criteria
      .flatMap((entry) => entry.checks)
      .filter((check) => check.test === `check-call-sites:${RULE_ID}`);
    expect(claims.map((check) => check.id)).toEqual(['AC-P2-24-3']);
  });
});
