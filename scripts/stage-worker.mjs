#!/usr/bin/env node
/**
 * Stages a Linux worker into the Windows installer's payload (§13).
 *
 * The ELF header is validated instead of trusting the filename. Pointing this script at a
 * Windows binary, or downloading the wrong build artifact, would otherwise ship a file the
 * subsystem cannot execute and make every affected repository quietly disappear from the shelf.
 */
import { chmodSync, closeSync, copyFileSync, mkdirSync, openSync, readSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const args = process.argv.slice(2);
const usage = 'stage-worker: usage: node scripts/stage-worker.mjs --from <path> [--arch x64|arm64]';

let from;
let requestedArch;
let usageProblem;
for (let index = 0; index < args.length; index += 1) {
  const argument = args[index];
  if (argument !== '--from' && argument !== '--arch') {
    usageProblem = `unknown argument ${argument}`;
    break;
  }

  const value = args[index + 1];
  if (value === undefined || value.startsWith('--')) {
    usageProblem = `${argument} requires a value`;
    break;
  }
  index += 1;

  if (argument === '--from') {
    if (from !== undefined) {
      usageProblem = '--from may be given only once';
      break;
    }
    from = value;
  } else {
    if (requestedArch !== undefined) {
      usageProblem = '--arch may be given only once';
      break;
    }
    requestedArch = value;
  }
}

if (from === undefined && usageProblem === undefined) usageProblem = '--from is required';
if (
  requestedArch !== undefined &&
  requestedArch !== 'x64' &&
  requestedArch !== 'arm64' &&
  usageProblem === undefined
) {
  usageProblem = `unsupported architecture ${requestedArch}`;
}
if (usageProblem !== undefined) {
  console.error(`stage-worker: ${usageProblem}`);
  console.error(usage);
  process.exit(2);
}

const header = Buffer.alloc(20);
let bytesRead;
let descriptor;
try {
  descriptor = openSync(from, 'r');
  bytesRead = readSync(descriptor, header, 0, header.length, 0);
} catch (error) {
  console.error(`stage-worker: cannot read ${from}: ${String(error)}`);
  process.exit(1);
} finally {
  if (descriptor !== undefined) closeSync(descriptor);
}

const problems = [];
if (bytesRead < header.length) {
  problems.push(`header is ${String(bytesRead)} bytes, expected at least 20`);
}
if (header[0] !== 0x7f || header[1] !== 0x45 || header[2] !== 0x4c || header[3] !== 0x46) {
  problems.push('missing ELF magic');
}
if (header[4] !== 2) problems.push(`class is ${String(header[4])}, expected 2 (64-bit)`);
if (header[5] !== 1) {
  problems.push(`data encoding is ${String(header[5])}, expected 1 (little-endian)`);
}

const machine = header.readUInt16LE(18);
const detectedArch = machine === 0x3e ? 'x64' : machine === 0xb7 ? 'arm64' : undefined;
if (detectedArch === undefined) {
  problems.push(`machine is 0x${machine.toString(16)}, expected 0x3e (x86-64) or 0xb7 (AArch64)`);
} else if (requestedArch !== undefined && requestedArch !== detectedArch) {
  problems.push(
    `requested architecture ${requestedArch} does not match detected architecture ${detectedArch}`,
  );
}

if (problems.length > 0) {
  console.error(`stage-worker: ${from} is not a linux worker: ${problems.join('; ')}`);
  process.exit(1);
}

const destination = join(root, 'build', 'worker', `linux-${detectedArch}`, 'codotheca-worker');
mkdirSync(dirname(destination), { recursive: true });
copyFileSync(from, destination);
chmodSync(destination, 0o755);
console.error(`stage-worker: staged ${from} -> ${destination} (linux-${detectedArch})`);
