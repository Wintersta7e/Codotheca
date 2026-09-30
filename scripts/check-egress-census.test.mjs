import assert from 'node:assert/strict';
import { mkdtempSync, readFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import {
  MARK_END,
  MARK_START,
  deriveCensus,
  httpSiteCensus,
  mirrorProblems,
  providerCensus,
} from './check-egress-census.mjs';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const read = (file) => readFileSync(join(root, file), 'utf8');

/** `README.md:126-129` as it stood on `a2fc935`, the markers added around it. */
const A2FC935_PARAGRAPH = `- **No account, and no telemetry.** There is no Codotheca account and nothing reports on you. ${MARK_START}The
  network is reached for three things only: GitHub, once you connect it; the dependency-advisory
  lookup, covered by the file-reading consent on the first-run screen; and a README's remote
  images, per project, once you allow them.${MARK_END}
`;

test('AC-P4-48-6 the census is derived, printed, and named by the README and SECURITY.md', () => {
  const census = deriveCensus(root);
  for (const entry of census.entries) {
    console.log(`${entry.source}  ${entry.category ?? '-'}  ${entry.detail}`);
  }
  console.log(
    `census: provider ${String(census.perSource.provider)}, http ${String(census.perSource.http)}, git ${String(census.perSource.git)}, shell ${String(census.perSource.shell)}; ${String(census.entries.length)} entries from ${String(census.scanned)} files`,
  );
  for (const source of ['provider', 'http', 'git', 'shell']) {
    assert.ok(census.perSource[source] > 0, `census source ${source} read nothing`);
  }
  assert.ok(census.categories.length > 0, 'the census reached no network category');
  const problems = [
    ...census.problems,
    ...mirrorProblems(read('README.md'), read('SECURITY.md'), census.categories),
  ];
  assert.deepEqual(problems, []);
});

test("AC-P4-48-6 a README paragraph that omits the pre-flight's remote read fails", () => {
  const { categories } = deriveCensus(root);
  const problems = mirrorProblems(A2FC935_PARAGRAPH, read('SECURITY.md'), categories);
  assert.ok(
    problems.some((p) => p.startsWith('README.md:') && p.includes('"Uninstall check"')),
    `expected the Uninstall check named as missing, got ${JSON.stringify(problems)}`,
  );
  assert.ok(
    problems.some((p) => p.includes('"three things"')),
    JSON.stringify(problems),
  );
});

test('AC-P4-48-6 an unclassified provider method fails', () => {
  const modRs = read('core/src/provider/mod.rs').replace(
    '"advisories",',
    '"advisories",\n    "upload_inventory",',
  );
  const { problems } = providerCensus(modRs, read('core/src/provider/github.rs'));
  assert.ok(
    problems.some((p) => p.includes('upload_inventory') && p.includes('not classified')),
    JSON.stringify(problems),
  );
});

test("AC-P4-48-6 a method classified 'none' that sends a token fails", () => {
  const githubRs = read('core/src/provider/github.rs');
  const real = providerCensus(read('core/src/provider/mod.rs'), githubRs);
  assert.deepEqual(real.problems, []);
  const tokened = githubRs.replace(
    'headers: unauthenticated_headers(),',
    'headers: request_headers(&self.token),',
  );
  assert.notEqual(tokened, githubRs, 'the fixture edit found nothing to change');
  const { problems } = providerCensus(read('core/src/provider/mod.rs'), tokened);
  assert.ok(
    problems.some((p) => p.startsWith('provider method advisories:')),
    JSON.stringify(problems),
  );
});

test('AC-P4-48-6 an HttpRequest in an unclassified file fails', () => {
  const planted = {
    path: 'core/src/stats/ping.rs',
    text: 'fn ping(t: &dyn HttpTransport) {\n    let request = HttpRequest {\n        method: "GET",\n    };\n}\n',
  };
  const inTests = {
    path: 'core/src/stats/other.rs',
    text: '#[cfg(test)]\nmod tests {\n    fn f() { let r = HttpRequest {\n    }; }\n}\n',
  };
  const { problems } = httpSiteCensus([planted, inTests]);
  assert.deepEqual(problems, [
    'core/src/stats/ping.rs: builds 1 HttpRequest, not classified in HTTP_SITE_CLASS',
  ]);
});

test('AC-P4-48-6 an empty tree fails', () => {
  const empty = mkdtempSync(join(tmpdir(), 'egress-census-'));
  const census = deriveCensus(empty);
  console.log(`scanned: ${String(census.scanned)}; problems: ${String(census.problems.length)}`);
  assert.equal(census.scanned, 0);
  assert.equal(census.entries.length, 0);
  for (const source of ['provider', 'http', 'git', 'shell']) {
    assert.ok(
      census.problems.includes(`census source ${source}: zero entries`),
      JSON.stringify(census.problems),
    );
  }
});
