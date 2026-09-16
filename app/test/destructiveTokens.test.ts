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

interface Site {
  readonly path: string;
  readonly token: string;
  readonly why: string;
}

interface Gate {
  readonly SCAN_ROOTS: readonly string[];
  readonly collectFiles: (roots: readonly string[], repoRoot: string) => readonly string[];
  readonly scanFiles: (
    files: readonly string[],
    repoRoot: string,
  ) => { scanned: number; violations: readonly unknown[] };
  readonly scanSource: (source: string, path: string) => readonly unknown[];
  readonly ALLOWLIST: readonly Site[];
  readonly UNINSTALL_SITES: readonly Site[];
  readonly DELETE_SITES: readonly Site[];
}

async function importGate(): Promise<Gate> {
  const gate: unknown = await import(/* @vite-ignore */ pathToFileURL(SCRIPT).href);
  if (
    typeof gate !== 'object' ||
    gate === null ||
    !('SCAN_ROOTS' in gate) ||
    !Array.isArray(gate.SCAN_ROOTS) ||
    !gate.SCAN_ROOTS.every((root: unknown) => typeof root === 'string') ||
    !('collectFiles' in gate) ||
    typeof gate.collectFiles !== 'function' ||
    !('scanFiles' in gate) ||
    typeof gate.scanFiles !== 'function' ||
    !('scanSource' in gate) ||
    typeof gate.scanSource !== 'function' ||
    !('ALLOWLIST' in gate) ||
    !Array.isArray(gate.ALLOWLIST) ||
    !('UNINSTALL_SITES' in gate) ||
    !Array.isArray(gate.UNINSTALL_SITES) ||
    !('DELETE_SITES' in gate) ||
    !Array.isArray(gate.DELETE_SITES)
  ) {
    throw new Error('destructive-token gate does not export the shape this test drives');
  }
  return gate as unknown as Gate;
}

async function readScanRoots(): Promise<readonly string[]> {
  return (await importGate()).SCAN_ROOTS;
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

  it('bans the three removal words uppercase-exact, and permits the command names', () => {
    const banned = ['DELETE THIS COPY', 'REMOVE THE FOLDER', 'RECLAIM SPACE'];
    for (const planted of banned) {
      const result = runWithProbe(RENDERER_PROBE, `export const label = '${planted}';\n`);
      expect(result.status, planted).toBe(1);
    }
    // The other direction, and the reason the three are case-sensitive: these are shipped command
    // names, and a case-insensitive ban fails the build on the schema that declares them.
    for (const permitted of ['roots.remove', 'collections.remove']) {
      const result = runWithProbe(RENDERER_PROBE, `export const name = '${permitted}';\n`);
      expect(result.status, permitted).toBe(0);
    }
    expect(run([]).status).toBe(0);
  });

  it('keeps every site narrow: one token, one path, and nothing else', async () => {
    const gate = await importGate();

    // The one enumerated DELETE site permits DELETE **there**...
    expect(
      gate.scanSource(
        "export const x = 'PRESS AGAIN TO DELETE';",
        'app/src/renderer/collections/armedDelete.ts',
      ),
    ).toHaveLength(0);

    // ...and not the same word one file over.
    expect(
      gate.scanSource(
        "export const x = 'PRESS AGAIN TO DELETE';",
        'app/src/renderer/collections/somewhereElse.ts',
      ).length,
    ).toBeGreaterThan(0);

    // ...and not a DIFFERENT banned word at the very same path. This is what the `token` field
    // buys: without it a site would permit every banned word at that path, and FORGET is the one
    // token that is absent forever.
    expect(
      gate.scanSource(
        "export const x = 'FORGET THIS PROJECT';",
        'app/src/renderer/collections/armedDelete.ts',
      ).length,
    ).toBeGreaterThan(0);
  });

  it('keeps UNINSTALL banned outright while its site list is empty', async () => {
    const gate = await importGate();
    expect(gate.UNINSTALL_SITES).toHaveLength(0);
    expect(
      gate.scanSource("export const x = 'UNINSTALL';", 'app/src/renderer/anywhere.tsx').length,
    ).toBeGreaterThan(0);
  });

  it('keeps the escape hatch shut', async () => {
    const gate = await importGate();
    expect(gate.ALLOWLIST).toHaveLength(0);
    for (const site of [...gate.UNINSTALL_SITES, ...gate.DELETE_SITES]) {
      expect(typeof site.path, 'a site needs a path').toBe('string');
      expect(typeof site.token, 'a site without a token permits every banned word there').toBe(
        'string',
      );
      expect((site.why ?? '').length, 'a site needs a written reason').toBeGreaterThan(20);
    }
  });

  it('survives a file that vanishes between the walk and the read', async () => {
    // Measured, not hypothetical. `app/test/styleGates.test.ts:39` plants and removes
    // `app/src/renderer/styles/__probe.css` inside a scan root to prove its own gate can fail,
    // and vitest runs the node project's files in parallel — so this gate walked that path and
    // read it after it was gone, crashed with an ENOENT stack, and failed the whole app suite.
    // A gate that throws is a gate that reports nothing.
    //
    // The window is far too narrow to reproduce by racing two suites, so the walk and the read
    // are separate exports and the gap between them is opened here on purpose.
    const gate = await importGate();
    const vanishing = join(REPO, 'app/src/renderer/styles/__vanishing_probe__.css');
    writeFileSync(vanishing, '.x { color: #ffffff; }\n', 'utf8');

    let walked: readonly string[];
    try {
      walked = gate.collectFiles(['app/src/renderer/styles'], REPO);
    } finally {
      rmSync(vanishing, { force: true });
    }
    expect(walked.some((file) => file.endsWith('__vanishing_probe__.css'))).toBe(true);

    const result = gate.scanFiles(walked, REPO);
    expect(result.scanned).toBe(walked.length - 1);
    expect(result.violations).toHaveLength(0);
  });
});
