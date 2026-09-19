/**
 * [p3] `AC-P3-35-7` — §35.8.6. **No health-derived figure leaves the window.**
 *
 * The gate reports co-location within one file, because a per-expression dataflow claim is not
 * something a scanner can honestly make. This drives it through its CLI and proves it bites in
 * both directions: a probe carrying an escape call **and** a health identifier is a violation, and
 * a probe carrying the escape call alone is not — a gate that flagged every notification would be
 * deleted by the first person who needed one.
 */
import { execFileSync } from 'node:child_process';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

const REPO = fileURLToPath(new URL('../..', import.meta.url));
const SCRIPT = join(REPO, 'scripts/check-health-escape.mjs');

interface Hit {
  readonly path: string;
  readonly line: number;
  readonly token: string;
}
interface Report {
  readonly filesScanned: number;
  readonly identifiers: readonly string[];
  readonly escapeSites: readonly Hit[];
  readonly violations: readonly { path: string }[];
}

function run(args: readonly string[]): { status: number; out: string } {
  try {
    return {
      status: 0,
      out: execFileSync('node', [SCRIPT, ...args], { cwd: REPO, encoding: 'utf8' }),
    };
  } catch (error) {
    const e = error as { status?: number; stdout?: string; stderr?: string };
    return { status: e.status ?? 1, out: `${e.stdout ?? ''}${e.stderr ?? ''}` };
  }
}

function report(args: readonly string[]): { status: number; report: Report } {
  const result = run(['--json', ...args]);
  const parsed: unknown = JSON.parse(result.out);
  if (
    typeof parsed !== 'object' ||
    parsed === null ||
    !('filesScanned' in parsed) ||
    typeof parsed.filesScanned !== 'number' ||
    !('identifiers' in parsed) ||
    !Array.isArray(parsed.identifiers) ||
    !('escapeSites' in parsed) ||
    !Array.isArray(parsed.escapeSites) ||
    !('violations' in parsed) ||
    !Array.isArray(parsed.violations)
  ) {
    throw new Error('the health-escape report has the wrong shape');
  }
  return { status: result.status, report: parsed as unknown as Report };
}

/**
 * Probes live in a directory of their own, **never inside `app/src/renderer`**.
 *
 * Vitest runs the node project's files in parallel, so a probe planted in a tree other gates are
 * walking is read by one of them mid-life — and a probe carrying `new Notification(` is a live
 * violation of `check-notification-origin.mjs` for as long as it sits there.
 * `app/test/notificationOrigin.test.ts:24-26` records the same decision.
 */
function withProbes(files: Record<string, string>): { status: number; report: Report } {
  const dir = mkdtempSync(join(tmpdir(), 'codotheca-health-escape-'));
  try {
    for (const [name, source] of Object.entries(files)) {
      writeFileSync(join(dir, name), source, 'utf8');
    }
    return report(['--root', dir]);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

describe('AC-P3-35-7 no health-derived figure leaves the window', () => {
  it('states what it scanned and finds nothing', () => {
    const { status, report: found } = report([]);
    expect(found.filesScanned, 'a run that scanned nothing proves nothing').toBeGreaterThan(0);
    expect(
      found.identifiers.length,
      'a renamed type would make a gate with no identifiers pass over everything',
    ).toBeGreaterThan(0);
    expect(found.violations).toEqual([]);
    expect(status).toBe(0);
  });

  it('derives its identifiers from the schema rather than listing them', () => {
    const { report: found } = report([]);
    // The two type names and the carrier a shelf row reaches the value through.
    expect(found.identifiers).toContain('HealthSummary');
    expect(found.identifiers).toContain('healthSummary');
    expect(found.identifiers).toContain('scoredOpen');
    // `state` is a field name of other wire types too, so matching it bare names nothing about
    // health — and it would report the one legitimate notification site in the shell.
    expect(found.identifiers).not.toContain('state');
  });

  it('does not report activateFromTray, which is the trap the criterion names', () => {
    // The banned substring must not also match the thing it is meant to allow: this export
    // contains `Tray` and is entirely legitimate, so the gate finds no escape site in its file.
    const { report: found } = report([]);
    expect(
      found.escapeSites.filter((hit) => hit.path === 'app/src/main/paletteShortcut.ts'),
    ).toEqual([]);
  });

  it('reports a health figure beside an escape call, and names the file', () => {
    const { status, report: found } = withProbes({
      'probe.ts': 'export const x = new Notification(String(row.healthSummary.scoredOpen));\n',
    });
    expect(status).toBe(1);
    expect(found.violations.map((v) => v.path)).toEqual(['probe.ts']);
  });

  it('does not report an escape call that carries no health figure', () => {
    const { status, report: found } = withProbes({
      'probe.ts': "export const x = new Notification('a scan finished');\n",
    });
    expect(status).toBe(0);
    expect(found.violations).toEqual([]);
    // It is still an escape site — the gate reports what it found, and the reviewer reads it.
    expect(found.escapeSites.map((hit) => hit.token)).toContain('new Notification(');
  });

  it('does not report a health figure that reaches no escape call', () => {
    const { status, report: found } = withProbes({
      'probe.ts': 'export const n = row.healthSummary.scoredOpen;\n',
    });
    expect(status).toBe(0);
    expect(found.violations).toEqual([]);
  });

  it('ignores a comment, because a comment is not a rendered figure', () => {
    const { status, report: found } = withProbes({
      'probe.ts':
        '// new Notification(row.healthSummary.scoredOpen) is banned here.\nexport const x = 1;\n',
    });
    expect(status).toBe(0);
    expect(found.violations).toEqual([]);
  });

  it('refuses to pass on a scan it did not run', () => {
    // No JSON on this path, deliberately: the guard fires before a report exists, so the only
    // honest output is the reason on stderr.
    const dir = mkdtempSync(join(tmpdir(), 'codotheca-health-escape-'));
    try {
      const result = run(['--json', '--root', dir]);
      expect(result.status).toBe(1);
      expect(result.out).toMatch(/0 files scanned/u);
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });
});
