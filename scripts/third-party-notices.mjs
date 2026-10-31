#!/usr/bin/env node
/**
 * Writes `THIRD-PARTY-NOTICES` at package time: every crate the shipped core links and every npm
 * package the shipped bundle carries, each with its version, its licence expression and — where
 * the package ships one — its licence text. The file is generated, gitignored and packed into
 * `app.asar` by `electron-builder.yml`'s `files:`; `check-packaging.mjs` asserts it arrived.
 *
 * Crates come from `cargo metadata --locked`, once per target the core is built for, walked from
 * the core over normal dependencies only: a build or dev dependency never reaches the binary.
 * npm packages come from the renderer's chunk map — the list `check-bundle.mjs` reads — plus
 * `electron`, whose runtime is the shell; the main and preload bundles import no npm package.
 *
 * Fails at zero crates or zero packages: an empty notice satisfies the packer and informs nobody.
 */
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync, readdirSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

/** The generated file's name, at the repository root and at the root of `app.asar`. */
export const NOTICES_FILE = 'THIRD-PARTY-NOTICES';

/** Every target a shipped core is built for: the Windows core and the two WSL workers. */
export const CORE_TARGETS = [
  'x86_64-pc-windows-msvc',
  'x86_64-unknown-linux-gnu',
  'aarch64-unknown-linux-gnu',
];

const LICENCE_FILE = /^(?:licen[cs]e|copying|notice)/iu;

/** Licence-text files at the top of a package directory, sorted, as absolute paths. */
export function licenceFiles(dir, declared) {
  const names = existsSync(dir)
    ? readdirSync(dir).filter(
        (name) => LICENCE_FILE.test(name) && statSync(join(dir, name)).isFile(),
      )
    : [];
  if (
    typeof declared === 'string' &&
    !names.includes(declared) &&
    existsSync(join(dir, declared))
  ) {
    names.push(declared);
  }
  return names.sort().map((name) => join(dir, name));
}

/**
 * The crates reachable from `resolve.root` over normal dependencies, the root itself excluded.
 * Takes one `cargo metadata` document per target and returns their union, sorted by name then
 * version.
 */
export function reachableCrates(metadataDocuments) {
  const byId = new Map();
  for (const metadata of metadataDocuments) {
    const packages = new Map(metadata.packages.map((pkg) => [pkg.id, pkg]));
    const nodes = new Map(metadata.resolve.nodes.map((node) => [node.id, node]));
    const rootId = metadata.resolve.root;
    const seen = new Set([rootId]);
    const queue = [rootId];
    while (queue.length > 0) {
      const node = nodes.get(queue.shift());
      for (const dep of node?.deps ?? []) {
        if (seen.has(dep.pkg)) continue;
        if (!dep.dep_kinds.some((kind) => kind.kind === null)) continue;
        seen.add(dep.pkg);
        queue.push(dep.pkg);
        const pkg = packages.get(dep.pkg);
        if (pkg !== undefined) byId.set(pkg.id, pkg);
      }
    }
  }
  return [...byId.values()]
    .map((pkg) => ({
      name: pkg.name,
      version: pkg.version,
      license: pkg.license ?? '(none declared)',
      texts: licenceFiles(dirname(pkg.manifest_path), pkg.license_file ?? undefined),
    }))
    .sort((a, b) => a.name.localeCompare(b.name) || a.version.localeCompare(b.version));
}

/** Every npm package name the renderer's chunk map says a chunk carries, plus `electron`. */
export function bundledPackageNames(chunkMap) {
  const names = new Set(['electron']);
  for (const chunk of chunkMap.chunks ?? [])
    for (const name of chunk.packages ?? []) names.add(name);
  return [...names].sort();
}

/** One npm package's notice entry, read from the first `node_modules` that holds it. */
export function npmPackage(name, moduleDirs) {
  for (const modules of moduleDirs) {
    const dir = join(modules, name);
    const manifestPath = join(dir, 'package.json');
    if (!existsSync(manifestPath)) continue;
    const manifest = JSON.parse(readFileSync(manifestPath, 'utf8'));
    const license =
      typeof manifest.license === 'string'
        ? manifest.license
        : (manifest.license?.type ?? '(none declared)');
    return { name, version: String(manifest.version), license, texts: licenceFiles(dir) };
  }
  throw new Error(`bundled package ${name} is installed in none of ${moduleDirs.join(', ')}`);
}

/**
 * The notices document. Identical licence texts are printed once and referred to by number,
 * since most crates ship the same two texts. Throws at zero crates or zero packages.
 */
export function renderNotices(crates, packages) {
  if (crates.length === 0) throw new Error('third-party-notices: zero crates — refusing to write');
  if (packages.length === 0)
    throw new Error('third-party-notices: zero npm packages — refusing to write');

  const texts = new Map();
  const textNumber = (path) => {
    const body = readFileSync(path, 'utf8');
    const key = createHash('sha256').update(body).digest('hex');
    if (!texts.has(key)) texts.set(key, { number: texts.size + 1, body });
    return texts.get(key).number;
  };
  const entry = (item) => {
    const refs = item.texts.map(textNumber);
    const textLine =
      refs.length === 0
        ? 'no licence text shipped'
        : `text ${refs.map((n) => `[${String(n)}]`).join(' ')}`;
    return `${item.name} ${item.version} — ${item.license} — ${textLine}`;
  };

  const lines = [
    'Third-party notices',
    '',
    'Codotheca is licensed under the GNU General Public License v3.0 (see LICENSE). It ships the',
    'following third-party software, each under its own licence.',
    '',
    `Rust crates linked into the core (${String(crates.length)}):`,
    '',
    ...crates.map(entry),
    '',
    `npm packages carried by the shell (${String(packages.length)}):`,
    '',
    ...packages.map(entry),
    '',
    'Licence texts',
  ];
  for (const { number, body } of texts.values()) {
    lines.push('', `[${String(number)}]`, '', body.trimEnd());
  }
  return `${lines.join('\n')}\n`;
}

function cargoMetadata(root, target) {
  const result = spawnSync(
    'cargo',
    [
      'metadata',
      '--locked',
      '--format-version',
      '1',
      '--manifest-path',
      join(root, 'core/Cargo.toml'),
      '--filter-platform',
      target,
    ],
    { encoding: 'utf8', maxBuffer: 256 * 1024 * 1024 },
  );
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(
      `cargo metadata for ${target} exited ${String(result.status)}: ${result.stderr}`,
    );
  }
  return JSON.parse(result.stdout);
}

function main() {
  const root = join(dirname(fileURLToPath(import.meta.url)), '..');
  const crates = reachableCrates(CORE_TARGETS.map((target) => cargoMetadata(root, target)));
  const chunkMapPath = join(root, 'app/out/renderer/.chunk-map.json');
  if (!existsSync(chunkMapPath)) {
    throw new Error('app/out/renderer/.chunk-map.json is missing — run `npm run build:app` first');
  }
  const moduleDirs = [join(root, 'app/node_modules'), join(root, 'node_modules')];
  const packages = bundledPackageNames(JSON.parse(readFileSync(chunkMapPath, 'utf8'))).map((name) =>
    npmPackage(name, moduleDirs),
  );
  writeFileSync(join(root, NOTICES_FILE), renderNotices(crates, packages));
  console.error(
    `third-party-notices: ${String(crates.length)} crates, ${String(packages.length)} npm packages → ${NOTICES_FILE}`,
  );
}

if (process.argv[1] === fileURLToPath(import.meta.url)) main();
