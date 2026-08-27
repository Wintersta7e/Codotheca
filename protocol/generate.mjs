#!/usr/bin/env node
// Generates the core<->shell contract for BOTH languages from one schema, and records a digest
// of what it produced. Hand-writing either side is how the two drift at the same version.
import { createHash } from 'node:crypto';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { loadSchema } from './lib/schema.mjs';
import { emitTypeScript } from './lib/emit-ts.mjs';
import { emitRust } from './lib/emit-rust.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, '..');
const schemaPath = join(here, 'schema/protocol.json');
const lockPath = join(here, 'generated.lock');
const check = process.argv.includes('--check');

const schema = loadSchema(schemaPath);
const sha = (text) => `sha256-${createHash('sha256').update(text, 'utf8').digest('hex')}`;

const outputs = {
  'app/src/generated/protocol.ts': emitTypeScript(schema),
  'core/src/protocol.rs': emitRust(schema),
};

const lock = {
  protocolVersion: schema.version,
  schema: sha(readFileSync(schemaPath, 'utf8')),
  commands: schema.commands.length,
  topics: Object.keys(schema.topics).length,
  events: Object.values(schema.topics).reduce((n, t) => n + Object.keys(t).length, 0),
  types: Object.keys(schema.types).length,
  files: Object.fromEntries(Object.entries(outputs).map(([p, text]) => [p, sha(text)])),
};
const lockText = `${JSON.stringify(lock, null, 2)}\n`;

if (check) {
  const stale = [];
  let recorded = '';
  try {
    recorded = readFileSync(lockPath, 'utf8');
  } catch {
    stale.push('protocol/generated.lock is missing');
  }
  if (recorded && recorded !== lockText) stale.push('protocol/generated.lock is stale');
  for (const [rel, text] of Object.entries(outputs)) {
    let onDisk = '';
    try {
      onDisk = readFileSync(join(root, rel), 'utf8');
    } catch {
      stale.push(`${rel} is missing`);
      continue;
    }
    if (onDisk !== text) stale.push(`${rel} does not match the schema`);
  }
  if (stale.length > 0) {
    console.error(
      `protocol codegen is out of date:\n  ${stale.join('\n  ')}\nRun \`npm run gen\`.`,
    );
    process.exit(1);
  }
  console.error('protocol codegen is up to date');
} else {
  // Bindings are generated INTO each consumer's source tree, so each language's own build
  // (tsc rootDir, cargo module resolution) sees them as ordinary local sources.
  for (const [rel, text] of Object.entries(outputs)) {
    mkdirSync(dirname(join(root, rel)), { recursive: true });
    writeFileSync(join(root, rel), text);
  }
  writeFileSync(lockPath, lockText);
  console.error(
    `protocol v${schema.version}: ${lock.commands} commands, ${lock.topics} topics, ` +
      `${lock.events} events, ${lock.types} types`,
  );
}
