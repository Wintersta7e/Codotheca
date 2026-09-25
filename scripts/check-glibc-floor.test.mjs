import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { highestGlibc, readFloor } from './check-glibc-floor.mjs';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const checker = join(root, 'scripts', 'check-glibc-floor.mjs');

/**
 * `readelf --version-info --wide`'s shape, trimmed. The symbol-versions section names a version
 * higher than any need, and the highest need is `WEAK`: neither may be what the reader reports.
 */
const SAMPLE = `
Version symbols section '.gnu.version' contains 4 entries:
 Addr: 0x0000000000000a3e  Offset: 0x00000a3e  Link: 6 (.dynsym)
  000:   0 (*local*)       2 (GLIBC_2.2.5)   3 (GLIBC_2.40)    4 (GLIBCXX_3.4.30)

Version needs section '.gnu.version_r' contains 3 entries:
 Addr: 0x0000000000000a48  Offset: 0x00000a48  Link: 7 (.dynstr)
  000000: Version: 1  File: libstdc++.so.6  Cnt: 1
  0x0010:   Name: GLIBCXX_3.4.30  Flags: none  Version: 6
  0x0020: Version: 1  File: libgcc_s.so.1  Cnt: 1
  0x0030:   Name: GCC_3.0  Flags: none  Version: 5
  0x0040: Version: 1  File: libc.so.6  Cnt: 4
  0x0050:   Name: GLIBC_2.39  Flags: WEAK  Version: 4
  0x0060:   Name: GLIBC_2.34  Flags: none  Version: 3
  0x0070:   Name: GLIBC_2.3.4  Flags: none  Version: 7
  0x0080:   Name: GLIBC_2.2.5  Flags: none  Version: 2
`;

const ELF_MAGIC = Buffer.from([0x7f, 0x45, 0x4c, 0x46]);

/** A directory holding two files that begin like an ELF and one that does not. */
function fakePack() {
  const dir = mkdtempSync(join(tmpdir(), 'glibc-floor-'));
  mkdirSync(join(dir, 'resources', 'core'), { recursive: true });
  writeFileSync(join(dir, 'app'), Buffer.concat([ELF_MAGIC, Buffer.from('rest of a shell')]));
  writeFileSync(
    join(dir, 'resources', 'core', 'core'),
    Buffer.concat([ELF_MAGIC, Buffer.from('a core')]),
  );
  writeFileSync(join(dir, 'resources', 'app.asar'), 'not an ELF');
  return dir;
}

/** @returns {{status: number, stdout: string, stderr: string}} */
function runCli(args) {
  try {
    const stdout = execFileSync(process.execPath, [checker, ...args], {
      encoding: 'utf8',
      stdio: 'pipe',
    });
    return { status: 0, stdout, stderr: '' };
  } catch (err) {
    const e = /** @type {{status: number, stdout: string, stderr: string}} */ (err);
    return { status: e.status, stdout: String(e.stdout ?? ''), stderr: String(e.stderr ?? '') };
  }
}

function hasReadelf() {
  try {
    execFileSync('readelf', ['--version'], { stdio: 'ignore' });
    return true;
  } catch {
    return false;
  }
}

test('AC-P4-48-10 the highest non-weak GLIBC need is reported and a weak one is ignored', () => {
  assert.equal(highestGlibc(SAMPLE), '2.34');
  assert.equal(highestGlibc('Version needs section'), null, 'no GLIBC need reads as none');
});

test('AC-P4-48-10 zero binaries read fails', () => {
  const empty = mkdtempSync(join(tmpdir(), 'glibc-floor-empty-'));
  const { binaries, problems } = readFloor({ dirs: [empty], runReadelf: () => SAMPLE });
  assert.equal(binaries.length, 0);
  assert.ok(
    problems.some((p) => /no ELF binary/u.test(p)),
    problems.join(' | '),
  );
  const cli = runCli(['--dir', empty]);
  assert.equal(cli.status, 1, cli.stderr);
  assert.match(cli.stdout, /^ELF binaries read: 0$/mu);
});

test('AC-P4-48-10 enforce mode fails above the baseline; report mode prints and passes', () => {
  const dir = fakePack();
  const runReadelf = (file) => (file.endsWith('core') ? SAMPLE : 'Version needs section\n');
  const report = readFloor({ dirs: [dir], runReadelf, baseline: '2.31' });
  assert.equal(report.binaries.length, 2, 'the two ELF files are read and the asar is not');
  assert.deepEqual(
    report.binaries.map((b) => b.glibc).sort(),
    ['2.34', 'none'],
    'a binary with no GLIBC need reports none',
  );
  assert.deepEqual(report.problems, [], 'report mode fails nothing above the baseline');
  assert.deepEqual(
    report.above.map((b) => b.glibc),
    ['2.34'],
  );

  const enforced = readFloor({ dirs: [dir], runReadelf, baseline: '2.31', enforce: true });
  assert.ok(
    enforced.problems.some((p) => /core.*2\.34.*2\.31/u.test(p)),
    enforced.problems.join(' | '),
  );
  const met = readFloor({ dirs: [dir], runReadelf, baseline: '2.35', enforce: true });
  assert.deepEqual(met.problems, [], 'a figure at or below the baseline passes');
});

test('AC-P4-48-10 a real ELF is read', (t) => {
  // A small system binary, copied into a directory of its own so the walk reads exactly one
  // file. `process.execPath` would do, but it is a hundred megabytes to copy per run.
  const real = '/usr/bin/true';
  if (process.platform !== 'linux' || !hasReadelf() || !existsSync(real)) {
    t.skip(`not run: needs Linux, readelf and ${real} (platform ${process.platform})`);
    return;
  }
  const dir = mkdtempSync(join(tmpdir(), 'glibc-floor-real-'));
  copyFileSync(real, join(dir, 'true'));
  const { binaries, problems } = readFloor({ dirs: [dir] });
  assert.deepEqual(problems, []);
  assert.equal(binaries.length, 1);
  assert.match(
    binaries[0].glibc,
    /^\d+\.\d+(\.\d+)?$/u,
    'a dynamically linked binary needs a glibc',
  );
});

test('the CLI refuses --enforce with no baseline to enforce', () => {
  const { status } = runCli(['--dir', fakePack(), '--enforce']);
  assert.equal(status, 2);
});
