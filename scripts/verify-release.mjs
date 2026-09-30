#!/usr/bin/env node
/**
 * A downloaded release draft, verified file by file (§48.2 b).
 *
 * The draft is **downloaded, never rebuilt**: a rebuild is not byte-identical, so it proves
 * nothing about what a user gets. Every file `SHA256SUMS` lists must hash to its line, every file
 * in the directory must be listed, and every listed file must pass `gh attestation verify` for
 * the repository. It prints one line per file and `files verified: <n>`, and fails at zero files,
 * on a missing `SHA256SUMS`, on a mismatch, on an unlisted file and on a failed attestation.
 *
 * The repository comes from `--repo` or `GH_REPO`; this script names no owner of its own.
 */
import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { createReadStream, existsSync, readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const SUMS = 'SHA256SUMS';
/** `sha256sum`'s line: the digest, a space, then ` ` (text mode) or `*` (binary mode), the name. */
const SUM_LINE = /^([0-9a-f]{64}) [ *](.+)$/u;

function sha256(path) {
  return new Promise((resolve, reject) => {
    const hash = createHash('sha256');
    createReadStream(path)
      .on('data', (chunk) => hash.update(chunk))
      .on('error', reject)
      .on('end', () => resolve(hash.digest('hex')));
  });
}

/** `gh <args>`, spawned with an argv and no shell. */
function spawnGh(args) {
  return new Promise((resolve) => {
    const child = spawn('gh', args, { stdio: ['ignore', 'pipe', 'pipe'] });
    let stdout = '';
    let stderr = '';
    child.stdout.on('data', (d) => (stdout += String(d)));
    child.stderr.on('data', (d) => (stderr += String(d)));
    child.on('error', (error) => resolve({ status: 127, stdout, stderr: String(error) }));
    child.on('close', (status) => resolve({ status: status ?? 1, stdout, stderr }));
  });
}

/**
 * @param {{ dir: string, repo: string, runGh?: (args: string[]) => Promise<{ status: number, stdout: string, stderr: string }> }} opts
 * @returns {Promise<{ files: { name: string, sha256: 'ok' | 'mismatch', attestation: 'ok' | 'failed' }[], problems: string[] }>}
 */
export async function verifyRelease({ dir, repo, runGh = spawnGh }) {
  const problems = [];
  const files = [];
  const sumsPath = join(dir, SUMS);
  if (!existsSync(sumsPath)) {
    return { files, problems: [`no ${SUMS} in ${dir}: a draft is verified against its own sums`] };
  }
  const listed = new Map();
  for (const line of readFileSync(sumsPath, 'utf8').split('\n')) {
    if (line.trim() === '') continue;
    const m = SUM_LINE.exec(line);
    if (m === null) {
      problems.push(`${SUMS}: an unreadable line: ${line}`);
      continue;
    }
    listed.set(m[2], m[1]);
  }
  for (const name of readdirSync(dir)) {
    if (name !== SUMS && !listed.has(name)) {
      problems.push(`${name}: downloaded with the draft, not listed in ${SUMS}`);
    }
  }
  for (const [name, digest] of listed) {
    const path = join(dir, name);
    const actual = existsSync(path) ? await sha256(path) : null;
    const sum = actual === digest ? 'ok' : 'mismatch';
    if (actual === null) problems.push(`${name}: listed in ${SUMS}, not downloaded`);
    else if (sum === 'mismatch')
      problems.push(`${name}: hashes to ${actual}, ${SUMS} says ${digest}`);
    const gh = await runGh(['attestation', 'verify', path, '--repo', repo]);
    const attestation = gh.status === 0 ? 'ok' : 'failed';
    if (attestation === 'failed') {
      problems.push(
        `${name}: attestation verify failed (${gh.stderr.trim() || `exit ${String(gh.status)}`})`,
      );
    }
    files.push({ name, sha256: sum, attestation });
  }
  if (files.length === 0) problems.push(`${SUMS} lists zero files`);
  return { files, problems };
}

async function main(argv) {
  let dir = null;
  let repo = process.env.GH_REPO ?? null;
  for (let i = 0; i < argv.length; i += 1) {
    if (argv[i] === '--dir') dir = argv[(i += 1)] ?? null;
    else if (argv[i] === '--repo') repo = argv[(i += 1)] ?? null;
    else dir = null;
  }
  if (dir === null || repo === null || repo === '') {
    process.stderr.write(
      'usage: verify-release.mjs --dir <downloaded draft> [--repo <owner>/<name>] (or GH_REPO)\n',
    );
    return 2;
  }
  const { files, problems } = await verifyRelease({ dir, repo });
  for (const f of files) {
    process.stdout.write(`${f.name}  sha256 ${f.sha256}  attestation ${f.attestation}\n`);
  }
  const verified = files.filter((f) => f.sha256 === 'ok' && f.attestation === 'ok').length;
  process.stdout.write(`files verified: ${String(verified)}\n`);
  for (const problem of problems) process.stderr.write(`verify-release: ${problem}\n`);
  return problems.length === 0 && verified > 0 ? 0 : 1;
}

// `import.meta.url` is not a `file:` URL when a bundler serves this module, and `fileURLToPath`
// throws on anything else — so the scheme is checked before the path is taken.
if (
  process.argv[1] &&
  import.meta.url.startsWith('file:') &&
  fileURLToPath(import.meta.url) === process.argv[1]
)
  process.exit(await main(process.argv.slice(2)));
