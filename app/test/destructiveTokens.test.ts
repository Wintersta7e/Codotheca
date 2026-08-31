import { execFileSync } from 'node:child_process';
import { rmSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { describe, expect, it } from 'vitest';

const REPO = fileURLToPath(new URL('../..', import.meta.url));
const SCRIPT = join(REPO, 'scripts/check-destructive-tokens.mjs');
const RENDERER_PROBE = join(REPO, 'app/src/renderer/__destructive_probe__.tsx');
const TEST_PROBE = join(REPO, 'app/src/renderer/__destructive_probe__.test.ts');

function run(args: readonly string[]): { status: number; out: string } {
  try {
    const out = execFileSync('node', [SCRIPT, ...args], { cwd: REPO, encoding: 'utf8' });
    return { status: 0, out };
  } catch (error) {
    const e = error as { status?: number; stdout?: string; stderr?: string };
    return { status: e.status ?? 1, out: `${e.stdout ?? ''}${e.stderr ?? ''}` };
  }
}

function runWithProbe(path: string, source: string): { status: number; out: string } {
  try {
    writeFileSync(path, source, 'utf8');
    return run([]);
  } finally {
    rmSync(path, { force: true });
  }
}

function parseReport(out: string): { filesScanned: number; violations: unknown[] } {
  const parsed: unknown = JSON.parse(out);
  if (
    typeof parsed !== 'object' ||
    parsed === null ||
    !('filesScanned' in parsed) ||
    typeof parsed.filesScanned !== 'number' ||
    !('violations' in parsed) ||
    !Array.isArray(parsed.violations)
  ) {
    throw new Error('destructive-token report has the wrong shape');
  }
  return { filesScanned: parsed.filesScanned, violations: parsed.violations };
}

async function readScanRoots(): Promise<readonly string[]> {
  const gate: unknown = await import(/* @vite-ignore */ pathToFileURL(SCRIPT).href);
  if (typeof gate !== 'object' || gate === null || !('SCAN_ROOTS' in gate)) {
    throw new Error('destructive-token gate does not export SCAN_ROOTS');
  }
  const roots = gate.SCAN_ROOTS;
  if (!Array.isArray(roots) || !roots.every((root: unknown) => typeof root === 'string')) {
    throw new Error('destructive-token gate exports an invalid SCAN_ROOTS');
  }
  return roots;
}

describe('the destructive-token gate', () => {
  it('proves its own detector before reporting anything', () => {
    expect(run(['--self-test']).status).toBe(0);
  });

  it('reports a non-empty clean scan as JSON', () => {
    const result = run(['--json']);
    expect(result.status).toBe(0);
    const report = parseReport(result.out);
    expect(report.filesScanned).toBeGreaterThan(0);
    expect(report.violations).toHaveLength(0);
  });

  it('fails rather than reporting a clean scan for a missing root', () => {
    const result = run(['--root', 'app/src/does-not-exist']);
    expect(result.status).toBe(1);
    expect(result.out).toMatch(/does-not-exist/);
  });

  it('rejects the real gitignored generated root', () => {
    const result = run(['--root', 'app/src/generated']);
    expect(result.status).toBe(1);
    expect(result.out).toMatch(/ignored/i);
  });

  it('detects a destructive token in a rendered string', () => {
    const result = runWithProbe(RENDERER_PROBE, "export const label = 'FORGET THIS PROJECT';\n");
    expect(result.status).toBe(1);
    expect(result.out).toContain('FORGET');
    expect(result.out).toContain('__destructive_probe__');
    expect(run([]).status).toBe(0);
  });

  it('ignores a destructive token in a comment', () => {
    const result = runWithProbe(RENDERER_PROBE, '// FORGET is banned here.\nexport const x = 1;\n');
    expect(result.status).toBe(0);
  });

  it('says how much it read on stdout on a passing run', () => {
    const result = run([]);
    expect(result.status).toBe(0);
    const match = /destructive-token gate: (\d+) files, 0 violations/.exec(result.out);
    expect(match).not.toBeNull();
    expect(Number(match?.[1] ?? 0)).toBeGreaterThan(50);
  });

  it('allows a test file to name the banned token', () => {
    const result = runWithProbe(TEST_PROBE, "export const banned = 'FORGET';\n");
    expect(result.status).toBe(0);
  });

  it('keeps every scan root backed by at least one tracked file', async () => {
    const scanRoots = await readScanRoots();
    for (const root of scanRoots) {
      const tracked = execFileSync('git', ['ls-files', '--', root], {
        cwd: REPO,
        encoding: 'utf8',
      });
      expect(tracked.trim(), `${root} must hold a tracked file`).not.toBe('');
    }
  });
});
