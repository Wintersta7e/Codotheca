#!/usr/bin/env node
/**
 * The packaging entry point. Every `npm run package:*` goes through here.
 *
 * Two reasons it is a script rather than a bare `electron-builder` invocation.
 *
 * It resolves electron-builder from its own `bin` field and runs it through *this* Node with no
 * shell. A `.cmd` shim cannot be spawned without a shell on Windows, and a shell here would put
 * environment values on a command line.
 *
 * And it refuses to claim a signature it is not producing. There is no code-signing
 * certificate: signing happens only when one is explicitly configured, and a publisher name is
 * passed only when it has a value — an empty one would leave the config asserting a publisher
 * while verifying against nothing.
 */
import { spawnSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');

const publisher = process.env.CODOTHECA_PUBLISHER_NAME ?? '';
const args = [
  ...process.argv.slice(2),
  ...(publisher === '' ? [] : [`--config.win.publisherName=${publisher}`]),
];

const builderDir = join(root, 'node_modules', 'electron-builder');
const bin = JSON.parse(readFileSync(join(builderDir, 'package.json'), 'utf8')).bin;
const entry = typeof bin === 'string' ? bin : bin['electron-builder'];

const signing = process.env.CODOTHECA_SIGN_TOOL ? 'configured' : 'none';
console.error(`package: signing=${signing} publisher=${publisher === '' ? '(none)' : publisher}`);
const result = spawnSync(process.execPath, [join(builderDir, entry), ...args], {
  stdio: 'inherit',
});
if (result.error) throw result.error;
process.exit(result.status ?? 1);
