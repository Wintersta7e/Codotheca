#!/usr/bin/env node
/**
 * §48.4: every network destination the product can reach, derived from the code, and the two
 * published lists of them held to that derivation.
 *
 * Four sources. Each is printed with its count, and each fails at zero or on an entry nobody
 * classified — so a new request is a failure here until someone says what it is:
 *
 *   1. **Provider requests** — `PROVIDER_REQUEST_METHODS` (`core/src/provider/mod.rs`), R73's
 *      enumerated census. Each method's classification is checked against its body in
 *      `github.rs`: a method classified as sending no credential takes no `SecretToken` and
 *      builds `unauthenticated_headers()`; any other method does neither.
 *   2. **Every `HttpRequest` construction under `core/src/`**, grouped by file. This is the net for
 *      a request outside the provider census, which source 1 alone cannot see.
 *   3. **Every git intent in `Intent::ALL`** (`core/src/gitw/intent.rs`), classified as reaching
 *      the network or not.
 *   4. **The renderer CSP's remote origins** (none at the first tag) **and the shell's remote
 *      loads** in `app/src/main`.
 *
 * The README's network paragraph and SECURITY.md's line sit between `egress-census` markers
 * (invisible when rendered). Every category the code reaches must be named between them in both,
 * and every category named must be one the code reaches — so a destination added in code fails
 * here until the copy names it, and a destination removed fails until the copy drops it.
 */
import { readdirSync } from 'node:fs';
import { join, relative, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

import { readScannedFile } from './lib/read-scanned.mjs';
import { withoutComments } from './lib/without-comments.mjs';

const PROVIDER_MOD = 'core/src/provider/mod.rs';
const PROVIDER_GITHUB = 'core/src/provider/github.rs';
const INTENT_RS = 'core/src/gitw/intent.rs';
const CSP_TS = 'app/src/shared/csp.ts';
const CORE_SRC = 'core/src';
const SHELL_SRC = 'app/src/main';

export const MARK_START = '<!-- egress-census:start -->';
export const MARK_END = '<!-- egress-census:end -->';

/**
 * The network categories, with the phrase each published list uses for it. The phrases are the
 * user's words (`README.md`'s paragraph and `SECURITY.md`'s line); this table only says which
 * substring of each the check looks for.
 */
export const CENSUS_CATEGORIES = {
  github: {
    name: 'GitHub once connected — its API and its sign-in',
    readme: 'GitHub',
    security: 'GitHub',
  },
  advisories: {
    name: 'the dependency-advisory lookup',
    readme: 'advisory lookup',
    security: 'advisory lookup',
    // The first-run consent paragraph names this one too: it is the destination reached before
    // any account exists (`app/src/renderer/firstrun/consentClaims.test.tsx`).
    consent: 'public advisory database',
    carries: 'package names and versions',
  },
  readmeImages: {
    name: "a README's remote images, once allowed per project",
    readme: 'remote images',
    security: 'README images',
  },
  installClone: {
    name: "Install's clone of a chosen repository",
    readme: "Install's clone",
    security: "Install's clone",
  },
  uninstallCheck: {
    name: "the Uninstall check's read of every configured https or ssh remote",
    readme: 'Uninstall check',
    security: 'Uninstall check',
  },
};

/** Source 1: each provider request method, and whether it carries the account's token. */
export const PROVIDER_METHOD_CLASS = {
  viewer: { category: 'github', auth: 'token' },
  list_orgs: { category: 'github', auth: 'token' },
  list_repos: { category: 'github', auth: 'token' },
  lookup_repo: { category: 'github', auth: 'token' },
  repo_facts: { category: 'github', auth: 'token' },
  ci_runs: { category: 'github', auth: 'token' },
  advisories: { category: 'advisories', auth: 'none' },
};

/**
 * Source 2: each file that builds an `HttpRequest`. `provider` means its requests are source 1's
 * methods, classified there.
 */
export const HTTP_SITE_CLASS = {
  'core/src/provider/github.rs': { provider: true, detail: 'the provider methods above' },
  'core/src/accounts/device.rs': {
    category: 'github',
    detail: "GitHub's device-flow sign-in",
  },
  'core/src/readme/fetch.rs': {
    category: 'readmeImages',
    detail: "a README's remote images, per project once allowed",
  },
};

/** Source 3: each git intent. A later intent that writes locally adds `{ network: false }`. */
export const INTENT_CLASS = {
  Clone: { network: true, category: 'installClone' },
  VerifyRead: { network: true, category: 'uninstallCheck' },
};

/** Source 4: each remote-capable call in the shell, keyed `<file>#<api>`. */
export const SHELL_LOAD_CLASS = {
  'app/src/main/index.ts#loadURL': {
    network: false,
    detail:
      'the development server electron-vite names in ELECTRON_RENDERER_URL; a packaged build loads its bundle from disk',
  },
  'app/src/main/index.ts#openExternal': {
    network: false,
    detail: "hands a confirmed link to the user's own browser; the app requests nothing",
  },
  'app/src/main/dialogs/externalLink.ts#openExternal': {
    network: false,
    detail: "hands a confirmed link to the user's own browser; the app requests nothing",
  },
};

/** The shell APIs that can reach a remote origin, each found as a call. */
const SHELL_LOAD_APIS = [
  ['net.request', /\bnet\.request\(/g],
  ['net.fetch', /\bnet\.fetch\(/g],
  ['fetch', /(?<![.\w$])fetch\(/g],
  ['http.request', /\bhttps?\.(?:request|get)\(/g],
  ['loadURL', /\.loadURL\(/g],
  ['openExternal', /\bopenExternal\(/g],
  ['setSpellCheckerDictionaryDownloadURL', /\bsetSpellCheckerDictionaryDownloadURL\(/g],
  ['autoUpdater', /\bautoUpdater\b/g],
];

const NUMBER_WORDS = [
  'zero',
  'one',
  'two',
  'three',
  'four',
  'five',
  'six',
  'seven',
  'eight',
  'nine',
];

/** Production Rust only: comments blanked, and everything from the first `#[cfg(test)]` cut. */
function productionRust(text) {
  const code = withoutComments(text);
  const cut = code.indexOf('#[cfg(test)]');
  return cut === -1 ? code : code.slice(0, cut);
}

/** The string literals of the first `[...]` after `anchor`, or `null` when there is none. */
function literalsAfter(code, anchor, quote) {
  const at = code.search(anchor);
  if (at === -1) return null;
  const open = code.indexOf('[', code.indexOf('=', at));
  const close = code.indexOf(']', open);
  if (open === -1 || close === -1) return null;
  const pattern = quote === '"' ? /"([^"]*)"/g : /(['"])((?:\\.|(?!\1).)*)\1/g;
  return [...code.slice(open + 1, close).matchAll(pattern)].map((m) =>
    quote === '"' ? m[1] : m[2],
  );
}

/** `fn <name>(`'s parameter list and body, or `null` when the file declares no such method. */
function methodOf(code, name) {
  const head = new RegExp(`^([ \\t]*)fn ${name}\\(`, 'm').exec(code);
  if (head === null) return null;
  const indent = head[1];
  let depth = 0;
  let paramsEnd = -1;
  for (let i = head.index + head[0].length - 1; i < code.length; i += 1) {
    if (code[i] === '(') depth += 1;
    else if (code[i] === ')') {
      depth -= 1;
      if (depth === 0) {
        paramsEnd = i;
        break;
      }
    }
  }
  if (paramsEnd === -1) return null;
  const rest = code.slice(paramsEnd);
  const next = new RegExp(`^(?:${indent}(?:pub(?:\\([^)]*\\))? )?(?:const )?fn \\w|\\})`, 'm');
  const end = next.exec(rest.slice(1));
  return {
    params: code.slice(head.index + head[0].length, paramsEnd),
    body: end === null ? rest : rest.slice(0, end.index + 1),
  };
}

/**
 * Source 1. `modRs` is `core/src/provider/mod.rs`, `githubRs` the adapter holding every method.
 *
 * @returns {{ entries: { method: string, category: string, auth: 'token' | 'none' }[], problems: string[] }}
 */
export function providerCensus(modRs, githubRs) {
  const problems = [];
  const entries = [];
  const methods = literalsAfter(withoutComments(modRs), /PROVIDER_REQUEST_METHODS\s*:/, '"') ?? [];
  if (methods.length === 0) {
    problems.push(`${PROVIDER_MOD}: no PROVIDER_REQUEST_METHODS entry read`);
  }
  const code = productionRust(githubRs);
  for (const method of methods) {
    const cls = PROVIDER_METHOD_CLASS[method];
    if (cls === undefined) {
      problems.push(`provider method ${method}: not classified in PROVIDER_METHOD_CLASS`);
      continue;
    }
    const found = methodOf(code, method);
    if (found === null) {
      problems.push(`provider method ${method}: no \`fn ${method}(\` in ${PROVIDER_GITHUB}`);
      continue;
    }
    const takesToken = found.params.includes('SecretToken');
    const unauthenticated = found.body.includes('unauthenticated_headers()');
    if (cls.auth === 'none' && (takesToken || !unauthenticated)) {
      problems.push(
        `provider method ${method}: classified as sending no credential, but it ${
          takesToken ? 'takes a SecretToken' : 'does not build unauthenticated_headers()'
        }`,
      );
    }
    if (cls.auth === 'token' && (!takesToken || unauthenticated)) {
      problems.push(
        `provider method ${method}: classified as sending the account's token, but it ${
          unauthenticated ? 'builds unauthenticated_headers()' : 'takes no SecretToken'
        }`,
      );
    }
    entries.push({ method, category: cls.category, auth: cls.auth });
  }
  return { entries, problems };
}

/**
 * Source 2, over `{ path, text }` pairs whose paths are relative to the repository root.
 *
 * @returns {{ entries: { file: string, sites: number, category: string | null, detail: string }[], problems: string[] }}
 */
export function httpSiteCensus(files) {
  const problems = [];
  const entries = [];
  for (const { path, text } of files) {
    const sites = productionRust(text)
      .split('\n')
      .filter(
        (line) =>
          /\bHttpRequest\s*\{/.test(line) && !/\b(?:struct|for)\s+HttpRequest\s*\{/.test(line),
      ).length;
    if (sites === 0) continue;
    const cls = HTTP_SITE_CLASS[path];
    if (cls === undefined) {
      problems.push(
        `${path}: builds ${String(sites)} HttpRequest, not classified in HTTP_SITE_CLASS`,
      );
      continue;
    }
    entries.push({ file: path, sites, category: cls.category ?? null, detail: cls.detail });
  }
  return { entries, problems };
}

/**
 * Source 3. Accepts `ALL: [IntentKind; N] = [...]` and `ALL: &[IntentKind] = &[...]`.
 *
 * @returns {{ entries: { variant: string, network: boolean, category: string | null }[], problems: string[] }}
 */
export function intentCensus(intentRs) {
  const problems = [];
  const entries = [];
  const code = productionRust(intentRs);
  const all =
    /\bALL\s*:\s*(?:\[\s*IntentKind\s*;\s*\d+\s*\]|&\[\s*IntentKind\s*\])\s*=\s*&?\[([^\]]*)\]/.exec(
      code,
    );
  const variants = all === null ? [] : [...all[1].matchAll(/IntentKind::(\w+)/g)].map((m) => m[1]);
  if (variants.length === 0) problems.push(`${INTENT_RS}: no Intent::ALL variant read`);
  for (const variant of variants) {
    const cls = INTENT_CLASS[variant];
    if (cls === undefined) {
      problems.push(`git intent ${variant}: not classified in INTENT_CLASS`);
      continue;
    }
    entries.push({ variant, network: cls.network, category: cls.network ? cls.category : null });
  }
  return { entries, problems };
}

/**
 * Source 4's first half: every CSP source token outside `csp.ts`'s own allowlist is a remote
 * origin, and the census has no category for one.
 *
 * @returns {{ tokens: number, origins: string[], problems: string[] }}
 */
export function cspCensus(cspTs) {
  const problems = [];
  const code = withoutComments(cspTs);
  const allowed = literalsAfter(code, /ALLOWED_SOURCES\s*:/, "'") ?? [];
  const directives = literalsAfter(code, /CONTENT_SECURITY_POLICY\s*=/, "'") ?? [];
  if (allowed.length === 0) problems.push(`${CSP_TS}: no ALLOWED_SOURCES entry read`);
  // The allowlist is csp.ts's, read rather than copied; this keeps it from becoming the hole.
  for (const source of allowed) {
    if (/[.*]|:\/\/|^(?:https?|wss?):$/i.test(source)) {
      problems.push(`${CSP_TS}: ALLOWED_SOURCES admits ${source}, which is a remote origin`);
    }
  }
  let tokens = 0;
  const origins = [];
  for (const directive of directives) {
    for (const token of directive.trim().split(/\s+/).slice(1)) {
      tokens += 1;
      if (!allowed.includes(token)) origins.push(token);
    }
  }
  if (tokens === 0) problems.push(`${CSP_TS}: no CONTENT_SECURITY_POLICY source read`);
  for (const origin of origins) {
    problems.push(
      `${CSP_TS}: the CSP names the remote origin ${origin}; the census has no category for it`,
    );
  }
  return { tokens, origins, problems };
}

/**
 * Source 4's second half, over `{ path, text }` pairs from `app/src/main`.
 *
 * @returns {{ entries: { key: string, sites: number, network: boolean, category: string | null, detail: string }[], problems: string[] }}
 */
export function shellLoadCensus(files) {
  const problems = [];
  const entries = [];
  for (const { path, text } of files) {
    const code = withoutComments(text);
    for (const [api, pattern] of SHELL_LOAD_APIS) {
      const sites = [...code.matchAll(pattern)].length;
      if (sites === 0) continue;
      const key = `${path}#${api}`;
      const cls = SHELL_LOAD_CLASS[key];
      if (cls === undefined) {
        problems.push(`${key}: ${String(sites)} call(s), not classified in SHELL_LOAD_CLASS`);
        continue;
      }
      entries.push({
        key,
        sites,
        network: cls.network,
        category: cls.network ? cls.category : null,
        detail: cls.detail,
      });
    }
  }
  return { entries, problems };
}

function walk(dir, keep, out) {
  let names;
  try {
    names = readdirSync(dir, { withFileTypes: true });
  } catch (error) {
    if (error && error.code === 'ENOENT') return out;
    throw error;
  }
  for (const entry of names) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) walk(path, keep, out);
    else if (keep(entry.name)) out.push(path);
  }
  return out;
}

/** Read every walked file that is still there; a vanished one is skipped before it is counted. */
function readTree(root, dir, keep) {
  const files = [];
  for (const path of walk(join(root, dir), keep, [])) {
    const text = readScannedFile(path);
    if (text !== null) files.push({ path: relative(root, path).split(sep).join('/'), text });
  }
  return files;
}

function readOne(root, file, problems) {
  const text = readScannedFile(join(root, file));
  if (text === null) problems.push(`${file}: missing`);
  return text ?? '';
}

/**
 * The whole census over the tree at `root`.
 *
 * @returns {{ entries: { source: string, category: string | null, detail: string }[], perSource: Record<string, number>, scanned: number, categories: string[], problems: string[] }}
 */
export function deriveCensus(root) {
  const problems = [];
  const modRs = readOne(root, PROVIDER_MOD, problems);
  const githubRs = readOne(root, PROVIDER_GITHUB, problems);
  const intentRs = readOne(root, INTENT_RS, problems);
  const cspTs = readOne(root, CSP_TS, problems);
  const coreFiles = readTree(root, CORE_SRC, (name) => name.endsWith('.rs'));
  const shellFiles = readTree(
    root,
    SHELL_SRC,
    (name) => name.endsWith('.ts') && !name.endsWith('.test.ts') && !name.endsWith('.d.ts'),
  );

  const provider = providerCensus(modRs, githubRs);
  const http = httpSiteCensus(coreFiles);
  const intents = intentCensus(intentRs);
  const csp = cspCensus(cspTs);
  const shell = shellLoadCensus(shellFiles);
  problems.push(
    ...provider.problems,
    ...http.problems,
    ...intents.problems,
    ...csp.problems,
    ...shell.problems,
  );

  const entries = [
    ...provider.entries.map((e) => ({
      source: 'provider',
      category: e.category,
      detail: `${e.method} (${e.auth === 'none' ? 'no credential' : "the account's token"})`,
    })),
    ...http.entries.map((e) => ({
      source: 'http',
      category: e.category,
      detail: `${e.file}: ${String(e.sites)} site(s), ${e.detail}`,
    })),
    ...intents.entries.map((e) => ({
      source: 'git',
      category: e.category,
      detail: `Intent::${e.variant}${e.network ? '' : ' (local only)'}`,
    })),
    ...csp.origins.map((origin) => ({ source: 'shell', category: null, detail: `CSP ${origin}` })),
    ...shell.entries.map((e) => ({
      source: 'shell',
      category: e.category,
      detail: `${e.key}: ${String(e.sites)} call(s), ${e.detail}`,
    })),
  ];
  const perSource = { provider: 0, http: 0, git: 0, shell: 0 };
  for (const entry of entries) perSource[entry.source] += 1;
  for (const [source, count] of Object.entries(perSource)) {
    if (count === 0) problems.push(`census source ${source}: zero entries`);
  }
  const categories = [...new Set(entries.map((e) => e.category).filter((c) => c !== null))];
  for (const category of categories) {
    if (!(category in CENSUS_CATEGORIES)) {
      problems.push(`census category ${category}: not in CENSUS_CATEGORIES`);
    }
  }
  for (const category of Object.keys(CENSUS_CATEGORIES)) {
    if (!categories.includes(category)) {
      problems.push(`census category ${category}: declared, but the code reaches it nowhere`);
    }
  }
  const scanned = coreFiles.length + shellFiles.length + (cspTs === '' ? 0 : 1);
  return { entries, perSource, scanned, categories, problems };
}

/** The marked block, whitespace collapsed and emphasis dropped, or `null` when unmarked. */
function markedBlock(text) {
  const start = text.indexOf(MARK_START);
  const end = text.indexOf(MARK_END);
  if (start === -1 || end === -1 || end < start) return null;
  if (text.indexOf(MARK_START, start + 1) !== -1) return null;
  return text
    .slice(start + MARK_START.length, end)
    .replace(/\*\*/g, '')
    .replace(/\s+/g, ' ');
}

/**
 * Every network category the census reached must be named between the markers of both
 * documents, by the phrase `CENSUS_CATEGORIES` gives for that document; a number of *things*
 * the text states must equal the category count.
 *
 * @param {string} readme
 * @param {string} security
 * @param {readonly string[]} categories
 * @returns {string[]}
 */
export function mirrorProblems(readme, security, categories) {
  const problems = [];
  for (const [doc, text, key] of [
    ['README.md', readme, 'readme'],
    ['SECURITY.md', security, 'security'],
  ]) {
    const block = markedBlock(text);
    if (block === null) {
      problems.push(`${doc}: no single ${MARK_START} … ${MARK_END} block`);
      continue;
    }
    for (const category of categories) {
      const cls = CENSUS_CATEGORIES[category];
      if (cls === undefined) continue;
      if (!block.includes(cls[key])) {
        problems.push(`${doc}: the network list does not name ${cls.name} ("${cls[key]}")`);
      }
    }
    const stated = /\b(\w+) things\b/i.exec(block);
    const count = stated === null ? -1 : NUMBER_WORDS.indexOf(stated[1].toLowerCase());
    if (count !== -1 && count !== categories.length) {
      problems.push(
        `${doc}: the network list says "${stated[0]}", and the code reaches ${String(categories.length)}`,
      );
    }
  }
  return problems;
}

function main() {
  const root = join(fileURLToPath(new URL('.', import.meta.url)), '..');
  const census = deriveCensus(root);
  for (const source of ['provider', 'http', 'git', 'shell']) {
    const rows = census.entries.filter((e) => e.source === source);
    process.stdout.write(`${source}: ${String(rows.length)}\n`);
    for (const row of rows) {
      process.stdout.write(`  ${row.category ?? '-'}  ${row.detail}\n`);
    }
  }
  process.stdout.write(
    `census: ${String(census.entries.length)} entries from ${String(census.scanned)} files; categories: ${census.categories.join(', ')}\n`,
  );
  const problems = [
    ...census.problems,
    ...mirrorProblems(
      readScannedFile(join(root, 'README.md')) ?? '',
      readScannedFile(join(root, 'SECURITY.md')) ?? '',
      census.categories,
    ),
  ];
  for (const problem of problems) process.stderr.write(`egress census: ${problem}\n`);
  return problems.length === 0 ? 0 : 1;
}

// `import.meta.url` is not a `file:` URL when a bundler serves this module (the renderer test
// imports `providerCensus`), and `fileURLToPath` throws on anything else — so the scheme is
// checked before the path is taken.
if (
  process.argv[1] &&
  import.meta.url.startsWith('file:') &&
  fileURLToPath(import.meta.url) === process.argv[1]
)
  process.exit(main());
