import assert from 'node:assert/strict';
import { mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { PACKED_LICENCE_FILES, packedLicenceFailures, readAsarFile } from './lib/asar.mjs';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');

/**
 * Writes an `app.asar` holding `entries` (name → contents) at its root, in the layout
 * electron-builder writes: an 8-byte pickle (`4`, inner size), an inner pickle (payload size,
 * JSON length, JSON, padding to four bytes), then the file bytes at their header offsets.
 */
function writeAsar(entries) {
  const files = {};
  const bodies = [];
  let offset = 0;
  for (const [name, contents] of Object.entries(entries)) {
    const body = Buffer.from(contents, 'utf8');
    files[name] = { size: body.length, offset: String(offset) };
    offset += body.length;
    bodies.push(body);
  }
  const json = Buffer.from(JSON.stringify({ files }), 'utf8');
  const payloadSize = Math.ceil((4 + json.length) / 4) * 4;
  const inner = Buffer.alloc(4 + payloadSize);
  inner.writeUInt32LE(payloadSize, 0);
  inner.writeUInt32LE(json.length, 4);
  json.copy(inner, 8);
  const outer = Buffer.alloc(8);
  outer.writeUInt32LE(4, 0);
  outer.writeUInt32LE(inner.length, 4);
  const path = join(mkdtempSync(join(tmpdir(), 'asar-')), 'app.asar');
  writeFileSync(path, Buffer.concat([outer, inner, ...bodies]));
  return path;
}

/** The bare (non-glob) paths electron-builder.yml's `files:` list packs at the archive root. */
function bareFilesEntries() {
  const yaml = readFileSync(join(root, 'electron-builder.yml'), 'utf8');
  const block = /^files:\n((?:\s*(?:#.*|-\s.+)\n)+)/mu.exec(yaml);
  assert.ok(block, 'electron-builder.yml declares a files: list');
  return block[1]
    .split('\n')
    .filter((line) => !line.trim().startsWith('#'))
    .map((line) => line.replace(/^\s*-\s*/u, '').trim())
    .filter((glob) => glob !== '' && !glob.includes('*') && !glob.includes('/'));
}

test('the archive reader returns a packed file byte for byte', () => {
  const archive = writeAsar({ 'package.json': '{"main":"x"}', LICENSE: 'GPL' });
  assert.equal(readAsarFile(archive, 'LICENSE').toString('utf8'), 'GPL');
  assert.equal(readAsarFile(archive, 'package.json').toString('utf8'), '{"main":"x"}');
});

test('AC-P4-50-3 a pack without LICENSE or notices fails', () => {
  const neither = packedLicenceFailures(writeAsar({ 'package.json': '{}' }));
  assert.equal(neither.length, 2, neither.join('\n'));
  assert.match(neither[0], /^LICENSE is not packed/u);
  assert.match(neither[1], /^THIRD-PARTY-NOTICES is not packed/u);

  const noNotices = packedLicenceFailures(writeAsar({ 'package.json': '{}', LICENSE: 'GPL' }));
  assert.deepEqual(noNotices.length, 1, noNotices.join('\n'));
  assert.match(noNotices[0], /^THIRD-PARTY-NOTICES is not packed/u);
});

test('AC-P4-50-3 a pack with an empty licence file fails', () => {
  const failures = packedLicenceFailures(
    writeAsar({ LICENSE: '', 'THIRD-PARTY-NOTICES': 'notices' }),
  );
  assert.equal(failures.length, 1, failures.join('\n'));
  assert.match(failures[0], /^LICENSE is empty/u);
});

test('AC-P4-50-3 a pack of what files: names carries both licence files', () => {
  const bare = bareFilesEntries();
  console.log(`files: packs ${String(bare.length)} bare root entries: ${bare.join(', ')}`);
  assert.ok(bare.length > 0, 'files: names no bare root entry, so this test packs nothing');
  const archive = writeAsar(Object.fromEntries(bare.map((name) => [name, `${name} contents`])));
  assert.deepEqual(packedLicenceFailures(archive), []);
  assert.deepEqual(PACKED_LICENCE_FILES, ['LICENSE', 'THIRD-PARTY-NOTICES']);
});
