import { execFileSync } from 'node:child_process';
import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs';
import { extname, join, relative, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const REPO_ROOT = fileURLToPath(new URL('..', import.meta.url));

/**
 * Where a rendered string, an accessible name or a command name can live.
 *
 * `app/src/generated` is deliberately absent: it is gitignored, and a check that greps an
 * ignored path is the exact defect this gate was written against. Command names are checked at
 * their tracked source of truth, `protocol/schema/protocol.json`, instead.
 */
export const SCAN_ROOTS = [
  'app/src/renderer',
  'app/src/shared',
  'app/src/main',
  'app/src/preload',
  'protocol/schema/protocol.json',
];

const EXTENSIONS = new Set(['.ts', '.tsx', '.js', '.mjs', '.css', '.json', '.html']);

export const DESTRUCTIVE_TOKENS = [
  { token: 'FORGET', pattern: /\bforget\b/i, why: 'FORGET became RELOCATE' },
  {
    token: 'UNINSTALL',
    pattern: /\buninstall\b/i,
    why: 'phase 1 admits no destructive operation',
  },
  {
    token: 'clean',
    pattern: /\bclean\b/i,
    why: 'absence of dirty is "no changes as of T", never "clean"',
  },
  { token: 'git push', pattern: /\bgit\s+push\b/i, why: 'every git invocation is read-only' },
  {
    token: 'git checkout',
    pattern: /\bgit\s+checkout\b/i,
    why: 'every git invocation is read-only',
  },
  { token: 'git clean', pattern: /\bgit\s+clean\b/i, why: 'every git invocation is read-only' },
];

/** Empty, and a new entry needs all three fields. An escape hatch is how a gate dies quietly. */
export const ALLOWLIST = [];

/**
 * String literals and JSX text only. Identifiers and comments are out of scope by design: the
 * ban is on what a user reads, and a comment naming the ban must not trip it.
 */
export function extractStrings(source) {
  const withoutComments = source
    .replace(/\/\*[\s\S]*?\*\//g, (comment) => comment.replace(/[^\r\n]/g, ' '))
    .replace(/(^|[^:])\/\/[^\r\n]*/g, (comment, lead) => {
      return lead + ' '.repeat(comment.length - lead.length);
    });
  const literal = /'([^'\\\n]|\\.)*'|"([^"\\\n]|\\.)*"|`([^`\\]|\\.)*`|>([^<>{}]+)</g;
  const found = [];
  for (const match of withoutComments.matchAll(literal)) {
    const line = withoutComments.slice(0, match.index).split('\n').length;
    found.push({ text: match[0], line });
  }
  return found;
}

export function scanSource(source, path) {
  const violations = [];
  for (const { text, line } of extractStrings(source)) {
    for (const rule of DESTRUCTIVE_TOKENS) {
      if (!rule.pattern.test(text)) continue;
      if (ALLOWLIST.some((entry) => entry.path === path && entry.token === rule.token)) continue;
      violations.push({
        path,
        line,
        token: rule.token,
        text: text.trim().slice(0, 120),
        why: rule.why,
      });
    }
  }
  return violations;
}

export function collectFiles(roots, repoRoot) {
  const files = [];

  // The kind comes from the directory entry `readdirSync` already read, never from a second
  // `statSync` on a path the walk reached — a probe removed in that gap throws ENOENT and takes
  // the gate down, which is the failure `scanFiles` below guards one step later at the read. Only
  // the roots are stat'd, and a root is a fixed directory nobody plants a probe over.
  const walk = (absolute, isDirectory) => {
    if (!isDirectory) {
      // The ban is on what a user reads. A test that asserts the ban must be able to name the
      // token, exactly the same carve-out the gate makes for comments.
      if (
        absolute.endsWith('.test.ts') ||
        absolute.endsWith('.test.tsx') ||
        absolute.endsWith('.test.js') ||
        absolute.endsWith('.test.mjs')
      ) {
        return;
      }
      if (EXTENSIONS.has(extname(absolute))) files.push(absolute);
      return;
    }

    for (const entry of readdirSync(absolute, { withFileTypes: true })) {
      if (entry.name === 'node_modules' || entry.name === 'dist') continue;
      walk(join(absolute, entry.name), entry.isDirectory());
    }
  };

  for (const root of roots) {
    const full = join(repoRoot, root);
    walk(full, statSync(full).isDirectory());
  }
  return files.sort();
}

/**
 * The read half, separated from the walk so the gap between them is testable.
 *
 * A file can vanish in that gap: `app/test/styleGates.test.ts` plants and removes a probe
 * stylesheet inside one of these roots to prove its own gate can fail, and vitest runs it in
 * parallel with this gate's suite. Reading the walk's list unguarded crashed this gate with an
 * ENOENT stack and failed the whole app suite — a gate that throws is a gate that reports
 * nothing. A file that is no longer there carries no rendered string, so it is skipped and
 * deliberately not counted, which keeps the "scanned nothing" guard meaning what it says. Any
 * other read error is a real problem and is raised.
 */
export function scanFiles(files, repoRoot) {
  const violations = [];
  let scanned = 0;
  for (const file of files) {
    let source;
    try {
      source = readFileSync(file, 'utf8');
    } catch (error) {
      if (error && error.code === 'ENOENT') continue;
      throw error;
    }
    scanned += 1;
    violations.push(...scanSource(source, relative(repoRoot, file).split(sep).join('/')));
  }
  return { scanned, violations };
}

function assertRootIsReadable(root) {
  const absolute = join(REPO_ROOT, root);
  if (!existsSync(absolute)) {
    return `scan root does not exist: ${root} — the gate refuses to pass on a scan it did not run`;
  }

  try {
    execFileSync('git', ['check-ignore', '-q', '--', root], {
      cwd: REPO_ROOT,
      stdio: 'ignore',
    });
    return `scan root is gitignored: ${root} — grepping an ignored path is a check that can never fail`;
  } catch {
    // Non-zero from `check-ignore` means "not ignored", which is what this gate requires.
  }

  const tracked = execFileSync('git', ['ls-files', '--', root], {
    cwd: REPO_ROOT,
    encoding: 'utf8',
  })
    .split('\n')
    .filter(Boolean);
  if (tracked.length === 0) return `scan root holds no tracked file: ${root}`;
  return null;
}

/** Proves the detector still detects before a clean report is believed. */
export function selfTest() {
  const failures = [];
  for (const rule of DESTRUCTIVE_TOKENS) {
    const planted = `const label = 'do not ${rule.token} this';`;
    const detected = scanSource(planted, 'fixture.ts').some(
      (violation) => violation.token === rule.token,
    );
    if (!detected) failures.push(`self-test: ${rule.token} was not detected in a string literal`);
  }

  const firstToken = DESTRUCTIVE_TOKENS[0].token;
  if (scanSource(`// ${firstToken} is banned here\n`, 'fixture.ts').length !== 0) {
    failures.push('self-test: a comment tripped the gate; the ban is on rendered strings');
  }
  if (scanSource("const ok = 'relocate this copy';", 'fixture.ts').length !== 0) {
    failures.push('self-test: a clean fixture reported a violation');
  }
  if (assertRootIsReadable('app/src/no-such-root') === null) {
    failures.push('self-test: a missing root did not fail the guard');
  }
  if (assertRootIsReadable('app/src/generated') === null) {
    failures.push('self-test: the gitignored generated root did not fail the guard');
  }
  return failures;
}

function main(argv) {
  if (argv.includes('--self-test')) {
    const failures = selfTest();
    for (const failure of failures) process.stderr.write(`${failure}\n`);
    return failures.length === 0 ? 0 : 1;
  }

  let json = false;
  let selectedRoot = null;
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (argument === '--json') {
      json = true;
      continue;
    }
    if (argument === '--root') {
      selectedRoot = argv[index + 1] ?? null;
      index += 1;
      if (selectedRoot === null) {
        process.stderr.write('--root requires a path\n');
        return 1;
      }
      continue;
    }
    process.stderr.write(`unknown argument: ${argument}\n`);
    return 1;
  }

  const roots = selectedRoot === null ? SCAN_ROOTS : [selectedRoot];
  const guardFailures = roots.map(assertRootIsReadable).filter((failure) => failure !== null);
  if (guardFailures.length > 0) {
    for (const failure of guardFailures) process.stderr.write(`${failure}\n`);
    return 1;
  }

  const { scanned, violations } = scanFiles(collectFiles(roots, REPO_ROOT), REPO_ROOT);

  if (scanned === 0) {
    process.stderr.write(
      'destructive-token gate: 0 files scanned — the gate refuses to pass on a scan it did not run\n',
    );
    return 1;
  }

  if (json) {
    process.stdout.write(`${JSON.stringify({ filesScanned: scanned, violations }, null, 2)}\n`);
    return violations.length === 0 ? 0 : 1;
  }

  for (const violation of violations) {
    process.stderr.write(
      `${violation.path}:${violation.line}: ${violation.token} — ${violation.why}\n  ${violation.text}\n`,
    );
  }
  if (violations.length > 0) return 1;
  process.stdout.write(`destructive-token gate: ${String(scanned)} files, 0 violations\n`);
  return 0;
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1])
  process.exit(main(process.argv.slice(2)));
