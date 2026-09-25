#!/usr/bin/env node
/**
 * No release carries update metadata (§48.5).
 *
 * There is no updater, and the user ruled there will be none: nothing in a release may let one
 * find it. `latest*.yml` is the feed an updater polls and `*.blockmap` is its differential
 * download map; `publish: null` and `nsis.differentialPackage: false` stop the packager writing
 * either, and this proves it did not, over the directory the draft is made from. It counts every
 * such file under the directory, prints the count and each path, and fails when there is one.
 *
 * `scripts/check-no-updater.mjs` is the other half — the updater package itself is absent.
 */
import { readdirSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const METADATA = [/^latest.*\.yml$/u, /\.blockmap$/u];

function walk(dir, out) {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) walk(path, out);
    else if (METADATA.some((pattern) => pattern.test(entry.name))) out.push(path);
  }
  return out;
}

function main(argv) {
  const [dir, ...rest] = argv;
  if (
    dir === undefined ||
    rest.length > 0 ||
    !statSync(dir, { throwIfNoEntry: false })?.isDirectory()
  ) {
    process.stderr.write('usage: check-update-metadata.mjs <directory>\n');
    return 2;
  }
  const found = walk(dir, []);
  for (const path of found) process.stdout.write(`update metadata: ${relative(dir, path)}\n`);
  process.stdout.write(`update metadata files: ${String(found.length)}\n`);
  return found.length === 0 ? 0 : 1;
}

// `import.meta.url` is not a `file:` URL when a bundler serves this module, and `fileURLToPath`
// throws on anything else — so the scheme is checked before the path is taken.
if (
  process.argv[1] &&
  import.meta.url.startsWith('file:') &&
  fileURLToPath(import.meta.url) === process.argv[1]
)
  process.exit(main(process.argv.slice(2)));
