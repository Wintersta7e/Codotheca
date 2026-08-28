#!/usr/bin/env node
// Fails if anything in the core writes to stdout outside the one module that owns it.
// stdout carries protocol frames and nothing else (spec §2.1); a stray println! corrupts
// the frame stream and the shell kills the connection with no useful diagnosis.
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { dirname, join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = join(dirname(fileURLToPath(import.meta.url)), '..');
const CORE = join(ROOT, 'core', 'src');
const OWNER = join('core', 'src', 'proto', 'transport.rs');
const BANNED = [
  { re: /\bprintln!\s*\(/, why: 'println! writes to stdout' },
  { re: /\bprint!\s*\(/, why: 'print! writes to stdout' },
  { re: /\bdbg!\s*\(/, why: 'dbg! ships debug output' },
  { re: /std::io::stdout\s*\(/, why: 'stdout is owned by proto::transport::claim_stdout' },
];

function* rustFiles(dir) {
  for (const entry of readdirSync(dir)) {
    const full = join(dir, entry);
    if (statSync(full).isDirectory()) yield* rustFiles(full);
    else if (entry.endsWith('.rs')) yield full;
  }
}

const findings = [];
for (const file of rustFiles(CORE)) {
  const rel = relative(ROOT, file);
  if (rel === OWNER) continue;
  readFileSync(file, 'utf8')
    .split('\n')
    .forEach((line, i) => {
      if (line.trimStart().startsWith('//')) return;
      for (const { re, why } of BANNED) {
        if (re.test(line)) findings.push(`${rel}:${i + 1}: ${why}`);
      }
    });
}

if (findings.length > 0) {
  console.error('stdout carries protocol frames and nothing else (spec §2.1):');
  for (const f of findings) console.error(`  ${f}`);
  process.exit(1);
}
console.error(`stdout discipline: clean, ${OWNER} is the only writer`);
