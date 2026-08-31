#!/usr/bin/env node
// Criterion 60. 7px is the floor; every font-size is a member of §8.7's completed scale; and a
// size that needs tracking ships with it — at these sizes the tracking is what buys the size.
//
// It scans renderer CSS *and* the inline `style={{…}}` objects in renderer `.tsx`, since the card
// sets per-project geometry inline and a 6.5px there is just as unreadable.
import { readdirSync, statSync } from 'node:fs';
import { dirname, join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import { readScannedFile } from './lib/read-scanned.mjs';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const rendererDir = join(root, 'app/src/renderer');
const typeSource = readScannedFile(join(rendererDir, 'theme/type.ts')) ?? '';

const scaleFrom = (name) => {
  const block = new RegExp(`${name}: readonly number\\[\\] = \\[([^\\]]*)\\]`).exec(typeSource);
  if (block === null) throw new Error(`theme/type.ts does not export ${name}`);
  return new Set(
    block[1]
      .split(',')
      .map((v) => Number(v.trim()))
      .filter((v) => !Number.isNaN(v)),
  );
};
const everySize = new Set([
  ...scaleFrom('DISPLAY_SIZES'),
  ...scaleFrom('BODY_SIZES'),
  ...scaleFrom('MONO_SIZES'),
]);

const failures = [];
function files(dir, extensions) {
  const out = [];
  for (const entry of readdirSync(dir)) {
    const full = join(dir, entry);
    if (statSync(full).isDirectory()) out.push(...files(full, extensions));
    else if (extensions.some((e) => entry.endsWith(e))) out.push(full);
  }
  return out;
}

// `fontSize: 8.5` and `font-size: 8.5px` alike. A unitless number in a style object is px.
const CSS_SIZE = /font-size\s*:\s*(\d+(?:\.\d+)?)px/g;
const TSX_SIZE = /fontSize\s*:\s*['"]?(\d+(?:\.\d+)?)(?:px)?['"]?/g;

const scanned = [];
for (const file of files(rendererDir, ['.css', '.tsx', '.ts'])) {
  if (file.endsWith('theme/type.ts') || file.endsWith('.test.ts') || file.endsWith('.test.tsx')) {
    continue;
  }
  const text = readScannedFile(file);
  // Not counted when it has vanished, so the empty-scan guard below still means what it says.
  if (text === null) continue;
  scanned.push(file);
  const seen = [];
  for (const [, px] of text.matchAll(CSS_SIZE)) seen.push(Number(px));
  for (const [, px] of text.matchAll(TSX_SIZE)) seen.push(Number(px));
  for (const px of seen) {
    if (px < 7) failures.push(`${relative(root, file)}: ${px}px is below the 7px floor`);
    else if (!everySize.has(px)) {
      failures.push(`${relative(root, file)}: ${px}px is not on the scale`);
    }
  }
  if (text.includes('6.5px') || /fontSize\s*:\s*6\.5/.test(text)) {
    failures.push(`${relative(root, file)}: 6.5px appears, and it renders nowhere`);
  }
  // Mono is always tracked; so is display at 14px and below. The declaration that sets the
  // family and the size must set letter-spacing in the same rule or the same style object.
  //
  // Blocks are split on `}` alone. Splitting on `;` as well — which is what a rule-per-line
  // reading suggests — puts `font-family` and `font-size` in *different* blocks, since CSS
  // writes one declaration per line, and the whole check then fires on nothing but a rule
  // written on a single line.
  for (const block of text.split(/\}/)) {
    const size = /font(?:-s|S)ize\s*:\s*['"]?(\d+(?:\.\d+)?)/.exec(block);
    if (size === null) continue;
    const mono = /--font-mono|JetBrains Mono/.test(block);
    const smallDisplay = /--font-display|Rajdhani/.test(block) && Number(size[1]) <= 14;
    if ((mono || smallDisplay) && !/letter-?[sS]pacing/.test(block)) {
      failures.push(`${relative(root, file)}: ${size[1]}px ships without its tracking`);
    }
  }
}

// A gate whose passing run reads nothing is a failing gate that looks green.
if (scanned.length === 0) {
  console.error(`check-type-scale: found no source file under ${relative(root, rendererDir)}`);
  process.exit(1);
}

const summary = `${String(scanned.length)} file(s), ${String(everySize.size)} scale member(s)`;

if (failures.length > 0) {
  for (const line of failures) console.error(`check-type-scale: ${line}`);
  console.error(`check-type-scale: ${String(failures.length)} problem(s) over ${summary}`);
  process.exit(1);
}
console.error(`check-type-scale: ok — ${summary}`);
