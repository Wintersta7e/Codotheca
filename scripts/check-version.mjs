#!/usr/bin/env node
/**
 * One version, stated everywhere the workspace declares one, and equal to the tag being released.
 *
 * The list of declarations is derived, never written down: a hand list is exactly the thing that
 * goes stale, and the first count of these sites was already one short (the lockfile's top-level
 * `version`). Three rules, one owner per source:
 *
 *   1. `package.json`'s `version`, and each `workspaces[i]/package.json`'s. A glob in `workspaces`
 *      fails, naming it — nothing here expands one silently.
 *   2. `package-lock.json`'s top-level `version`, `packages[""]`, and `packages[<ws>]` per workspace.
 *   3. Every tracked `Cargo.toml` holding a `[package]` table, and that package's entry in the
 *      tracked `Cargo.lock` beside it. A package with no tracked lock beside it fails.
 *
 * Versions written into test fixtures are not declarations, and none is read: nothing is read that
 * the three rules do not name. A value the reader cannot parse is a failure, never a skip.
 *
 * No prerelease suffix scheme exists, so a tag that is not `v<major>.<minor>.<patch>` fails.
 */
import { execFileSync } from 'node:child_process';
import { existsSync, readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const GLOB = /[*?[\]{}!]/u;
const TAG = /^v(\d+\.\d+\.\d+)$/u;

function readJson(root, file, problems) {
  const path = join(root, file);
  if (!existsSync(path)) {
    problems.push(`${file}: missing`);
    return null;
  }
  try {
    return JSON.parse(readFileSync(path, 'utf8'));
  } catch (error) {
    problems.push(`${file}: not readable as JSON (${String(error)})`);
    return null;
  }
}

function take(sites, problems, site, value) {
  if (typeof value === 'string' && value !== '') sites.push({ site, value });
  else problems.push(`${site}: no version string where one is declared`);
}

/**
 * The `key = "value"` lines of each TOML table, in order, keyed by header (`package`,
 * `[package]` for an array-of-tables entry). Enough for a manifest and a lockfile; a line under a
 * header that is not a comment, blank, or a plain string/array continuation is left alone, and the
 * caller fails on any value it needed and could not find as a plain string.
 */
function tomlTables(text) {
  const tables = [];
  let current = null;
  for (const line of text.split(/\r?\n/u)) {
    const header = /^\s*(\[\[?)\s*([^\]]+?)\s*\]\]?\s*(#.*)?$/u.exec(line);
    if (header) {
      current = { header: header[1] === '[[' ? `[${header[2]}]` : header[2], keys: new Map() };
      tables.push(current);
      continue;
    }
    const pair = /^\s*([A-Za-z0-9_.-]+)\s*=\s*(.*?)\s*$/u.exec(line);
    if (pair && current) current.keys.set(pair[1], pair[2]);
  }
  return tables;
}

/** A TOML value that is a plain basic string, or `null`. */
function tomlString(raw) {
  const m = /^"([^"\\]*)"\s*(#.*)?$/u.exec(raw ?? '');
  return m ? m[1] : null;
}

function cargoSites(root, sites, problems, cargoManifests) {
  let tracked;
  try {
    tracked = execFileSync('git', ['ls-files', '-z', '--', '*Cargo.toml', '*Cargo.lock'], {
      cwd: root,
      encoding: 'utf8',
      stdio: ['ignore', 'pipe', 'pipe'],
    })
      .split('\0')
      .filter(Boolean);
  } catch (error) {
    problems.push(`git ls-files could not list the tracked Cargo manifests: ${String(error)}`);
    return;
  }
  const locks = new Set(tracked.filter((p) => /(^|\/)Cargo\.lock$/u.test(p)));
  const manifests = cargoManifests ?? tracked.filter((p) => /(^|\/)Cargo\.toml$/u.test(p));
  for (const manifest of manifests) {
    const pkg = tomlTables(readFileSync(join(root, manifest), 'utf8')).find(
      (t) => t.header === 'package',
    );
    if (!pkg) continue; // a virtual workspace manifest declares no version
    const name = tomlString(pkg.keys.get('name'));
    const version = tomlString(pkg.keys.get('version'));
    if (name === null) {
      problems.push(`${manifest}#package.name: not a plain string`);
      continue;
    }
    take(sites, problems, `${manifest}#package.version`, version ?? undefined);
    const dir = dirname(manifest);
    const lock = dir === '.' ? 'Cargo.lock' : `${dir}/Cargo.lock`;
    if (!locks.has(lock)) {
      problems.push(`${manifest}: package "${name}" has no tracked Cargo.lock beside it`);
      continue;
    }
    const entries = tomlTables(readFileSync(join(root, lock), 'utf8')).filter(
      (t) => t.header === '[package]' && tomlString(t.keys.get('name')) === name,
    );
    const site = `${lock}#package["${name}"].version`;
    if (entries.length !== 1) {
      problems.push(`${site}: ${String(entries.length)} entries named "${name}", expected one`);
      continue;
    }
    take(sites, problems, site, tomlString(entries[0].keys.get('version')) ?? undefined);
  }
}

function derive(root, opts = {}) {
  const sites = [];
  const problems = [];
  if (existsSync(join(root, 'package.json'))) {
    const pkg = readJson(root, 'package.json', problems);
    const workspaces = pkg?.workspaces ?? [];
    if (!Array.isArray(workspaces) || workspaces.some((w) => typeof w !== 'string')) {
      problems.push('package.json#workspaces: not a list of paths');
    } else if (pkg) {
      take(sites, problems, 'package.json#version', pkg.version);
      const plain = workspaces.filter((ws) => {
        if (!GLOB.test(ws)) return true;
        problems.push(
          `package.json#workspaces: "${ws}" is a glob, which this check does not expand`,
        );
        return false;
      });
      for (const ws of plain) {
        const file = `${ws}/package.json`;
        take(sites, problems, `${file}#version`, readJson(root, file, problems)?.version);
      }
      const lock = readJson(root, 'package-lock.json', problems);
      if (lock) {
        take(sites, problems, 'package-lock.json#version', lock.version);
        for (const key of ['', ...plain]) {
          take(
            sites,
            problems,
            `package-lock.json#packages["${key}"].version`,
            lock.packages?.[key]?.version,
          );
        }
      }
    }
  }
  cargoSites(root, sites, problems, opts.cargoManifests);
  if (sites.length === 0) problems.push('no version declaration was read');
  return { sites, problems };
}

/**
 * Every version declaration the workspace holds, each spelled `<file>#<field>`. Throws, naming
 * each, when a declaration cannot be read.
 */
export function declarationSites(root, opts) {
  const { sites, problems } = derive(root, opts);
  if (problems.length > 0) throw new Error(problems.join('\n'));
  return sites;
}

/**
 * The derived sites, the one version they agree on (`null` when they do not), and every problem:
 * an unreadable site, zero sites, a disagreement, or a tag that is malformed or unequal to them.
 */
export function checkVersion({ root, tag }) {
  const { sites, problems } = derive(root);
  const values = [...new Set(sites.map((s) => s.value))];
  const version = values.length === 1 ? values[0] : null;
  if (values.length > 1) {
    const counts = values.map((v) => [v, sites.filter((s) => s.value === v).length]);
    const majority = counts.reduce((a, b) => (b[1] > a[1] ? b : a))[0];
    for (const s of sites) {
      if (s.value !== majority) problems.push(`${s.site} = ${s.value} disagrees with ${majority}`);
    }
  }
  if (tag !== null) {
    const m = TAG.exec(tag);
    if (!m) {
      problems.push(`tag ${tag} is not v<major>.<minor>.<patch>; no prerelease scheme exists`);
    } else {
      for (const s of sites) {
        if (s.value !== m[1]) problems.push(`${s.site} = ${s.value}, but the tag is ${tag}`);
      }
    }
  }
  return { sites, version, problems };
}

function main(argv) {
  let tag = null;
  for (let i = 0; i < argv.length; i += 1) {
    if (argv[i] === '--tag' && argv[i + 1] !== undefined && argv[i + 1] !== '') {
      tag = argv[i + 1];
      i += 1;
    } else {
      process.stderr.write(`usage: check-version.mjs [--tag <tag>] (unexpected: ${argv[i]})\n`);
      return 2;
    }
  }
  const refType = process.env.GITHUB_REF_TYPE ?? '';
  const refName = process.env.GITHUB_REF_NAME ?? '';
  if (tag === null && refType === 'tag') tag = refName;

  const root = join(dirname(fileURLToPath(import.meta.url)), '..');
  const { sites, problems } = checkVersion({ root, tag });
  for (const { site, value } of sites) process.stdout.write(`version site: ${site} = ${value}\n`);
  process.stdout.write(`version sites: ${String(sites.length)}\n`);
  if (problems.length > 0) {
    for (const p of problems) process.stderr.write(`${p}\n`);
    return 1;
  }
  process.stdout.write(
    tag === null
      ? `no tag compared (${refType || 'no ref type'} ${refName || 'no ref name'})\n`
      : `tag: ${tag} — matches\n`,
  );
  return 0;
}

// `import.meta.url` is not a `file:` URL when a bundler serves this module, and `fileURLToPath`
// throws on anything else — so the scheme is checked before the path is taken.
if (
  process.argv[1] &&
  import.meta.url.startsWith('file:') &&
  fileURLToPath(import.meta.url) === process.argv[1]
)
  process.exit(main(process.argv.slice(2)));
