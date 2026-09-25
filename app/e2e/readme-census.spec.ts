import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import * as path from 'node:path';

import { _electron as electron, expect, test, type ElectronApplication } from '@playwright/test';

// Playwright transpiles specs to CommonJS, so `__dirname` is correct here and `import.meta` is
// not — the opposite of every vitest file in this repo.
const appDir = path.resolve(__dirname, '..');
const repoRoot = path.resolve(appDir, '..');

/**
 * **AC-P2-25-18 — the request census, in the built app.**
 *
 * R88: a criterion about something a user sees is satisfied by the artefact, never by the call
 * that would produce it. So this counts what the **frame actually requested** through Electron's
 * own `webRequest` and what the frame **actually rendered**, and prints both numbers. A
 * zero-request run that rendered nothing is the failure this pairing exists to catch, which is
 * why the second number is not optional.
 *
 * R89: `app/e2e/*` launches `out/`, not `src/`, so this runs after `npm run build:app`.
 *
 * **The remote host is `.invalid`** (RFC 2606), which resolves nowhere on any machine. That is
 * deliberate: after consent the core *attempts* the fetch and fails, which is observable as the
 * placeholder's state moving from `blocked` to `unreachable` — and the Electron counter staying
 * at zero through both halves is what proves every request was the core's, in another process.
 */
const coreBinary = path.join(
  repoRoot,
  'core',
  'target',
  'release',
  process.platform === 'win32' ? 'codotheca-core.exe' : 'codotheca-core',
);

const EMAIL = 'e2e@example.invalid';
const BADGE = 'https://badges.example.invalid/build.svg';

const README = [
  '# widget',
  '',
  'A paragraph with a [link](https://example.invalid/docs) in it.',
  '',
  `![build](${BADGE})`,
  '',
  '![diagram](docs/diagram.png)',
  '',
  '| column | value |',
  '| --- | --- |',
  '| one | 1 |',
  '',
  '```rust',
  'fn main() {}',
  '```',
  '',
  'The identity $x^2 + y^2 = z^2$ holds.',
  '',
].join('\n');

/** One repository whose README exercises every branch §25.5 names. */
function seedHome(): string {
  const home = mkdtempSync(path.join(tmpdir(), 'codotheca-census-'));
  writeFileSync(
    path.join(home, '.gitconfig'),
    `[user]\n\tname = E2E\n\temail = ${EMAIL}\n`,
    'utf8',
  );
  const src = path.join(home, 'src');
  mkdirSync(src);
  const dir = path.join(src, 'widget');
  mkdirSync(dir);
  writeFileSync(path.join(dir, 'README.md'), README, 'utf8');
  mkdirSync(path.join(dir, 'docs'));
  // A real PNG signature, which is what the core sniffs — the extension decides nothing.
  const png = Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    Buffer.alloc(64, 0x42),
  ]);
  writeFileSync(path.join(dir, 'docs', 'diagram.png'), png);
  const git = (...args: string[]): void => {
    execFileSync('git', ['-c', 'user.name=E2E', '-c', `user.email=${EMAIL}`, ...args], {
      cwd: dir,
      stdio: 'ignore',
    });
  };
  git('init', '-q', '-b', 'main');
  git('add', '.');
  // `--no-verify`: a developer machine may carry global `core.hooksPath` hooks, and this
  // fixture's one-word message is not a commit anybody reviews. Measured: a global commit-msg
  // hook refused it and the spec failed in `seedHome` before the app ever launched.
  git('commit', '--no-verify', '-qm', 'seed');
  return home;
}

function launch(home: string): Promise<ElectronApplication> {
  const userData = mkdtempSync(path.join(tmpdir(), 'codotheca-census-data-'));
  return electron.launch({
    args: ['.', `--user-data-dir=${userData}`],
    cwd: appDir,
    env: { ...process.env, HOME: home, USERPROFILE: home, CODOTHECA_DATA_DIR: userData },
  });
}

/** Schemes the product's own documents and subresources legitimately use. */
const LOCAL_SCHEMES = ['file:', 'codotheca:', 'devtools:', 'about:', 'blob:', 'data:'];

test('AC-P2-25-18 the request census with consent absent', async () => {
  test.skip(
    !existsSync(coreBinary),
    'the release core binary is not built, so there is no core to scan with',
  );
  test.setTimeout(300_000);

  const home = seedHome();
  const app = await launch(home);
  try {
    const window = await app.firstWindow();
    await window.waitForLoadState('domcontentloaded');

    // Arm the census in the main process, before anything the panel does.
    await app.evaluate(({ session }, schemes) => {
      const offenders: string[] = [];
      (globalThis as unknown as { __census: string[] }).__census = offenders;
      session.defaultSession.webRequest.onBeforeRequest((details, callback) => {
        if (!schemes.some((scheme) => details.url.startsWith(scheme))) offenders.push(details.url);
        callback({});
      });
      return true;
    }, LOCAL_SCHEMES);

    await window.waitForSelector('.cdt-fr-view--roots', { timeout: 60_000 });
    const corpusRow = window
      .locator('[data-testid="fr-root-row"][data-tickable="true"]')
      .filter({ hasText: home });
    await expect(corpusRow).toHaveCount(1);
    await window.getByRole('button', { name: 'DIG', exact: true }).click();

    const goOn = window.getByRole('button', { name: 'GO ON', exact: true });
    await goOn.waitFor({ state: 'visible', timeout: 150_000 });
    await goOn.click();
    const notNow = window.getByRole('button', { name: 'NOT NOW', exact: true });
    await notNow.waitFor({ state: 'visible', timeout: 60_000 });
    await notNow.click();
    await expect(window.locator('.cdt-card')).toHaveCount(1, { timeout: 60_000 });

    await window
      .locator('.cdt-card')
      .first()
      .click({ modifiers: ['Shift'] });
    await window.waitForSelector('[data-testid="cp-tabpanel"]', { timeout: 60_000 });

    // **The artefact.** The frame exists and its document rendered.
    const frame = window.locator('[data-testid="cp-readme-frame"]');
    await expect(frame).toHaveCount(1, { timeout: 60_000 });

    // **AC-P2-25-16 off the real element, in the real engine.** The attribute present and exactly
    // empty is the whole threat model: an opaque origin with every flag off. Asserted here as well
    // as in jsdom because this is the one place it is read by the Chromium that enforces it.
    await expect(frame).toHaveAttribute('sandbox', '');
    interface FrameContents {
      blocks: number;
      images: number;
      dataImages: number;
      placeholders: string[];
      table: number;
      code: number;
      math: number;
    }
    /**
     * **Read the document the panel has settled on, named rather than waited for.**
     *
     * The panel destroys and recreates the `iframe` for each document — Chromium will not
     * re-navigate a sandboxed `srcdoc` frame when the attribute is replaced, so a new document is
     * a new element. "The frame" is therefore not one object over time, and a handle taken before
     * a transition points at a frame that is detached after it: `frame.evaluate: Frame was
     * detached`, which this spec produced in **two runs of four** before this was written.
     *
     * R72 says force the condition rather than wait for it, so nothing here polls the *contents*
     * until they look right. The element carries `data-revision` — which document it is showing —
     * and the count is arithmetic rather than a guess: **one document per `srcdoc` the panel
     * assigns.** Before consent that is 2 (the placeholder-only first paint, then the assets); the
     * grant re-runs the same pair, so it is 4. Waiting for the number and *then* reading means the
     * transition is over by construction, and a flow that stops producing exactly those documents
     * fails loudly here instead of racing.
     */
    const readRevision = async (revision: number): Promise<FrameContents> => {
      const settled = window.locator(
        `[data-testid="cp-readme-frame"][data-revision="${String(revision)}"]`,
      );
      await expect(settled).toHaveCount(1, { timeout: 60_000 });
      const handle = await settled.elementHandle();
      const inner = await handle.contentFrame();
      if (inner === null) {
        throw new Error(`document ${String(revision)} has no content frame`);
      }
      return inner.evaluate(() => ({
        blocks: document.body.querySelectorAll('p, h1, h2, h3, table, pre, ul, ol, blockquote')
          .length,
        images: document.body.querySelectorAll('img[src]').length,
        dataImages: document.body.querySelectorAll('img[src^="data:image/"]').length,
        placeholders: [...document.body.querySelectorAll('[data-asset-state]')].map(
          (element) => element.getAttribute('data-asset-state') ?? '?',
        ),
        table: document.body.querySelectorAll('table td').length,
        code: document.body.querySelectorAll('pre code').length,
        math: document.body.querySelectorAll('math').length,
      }));
    };

    /** One document per `srcdoc` assignment: first paint, then the assets applied to it. */
    const BEFORE_CONSENT = 2;
    /** The grant re-runs the same pair, so the settled document after it is the fourth. */
    const AFTER_CONSENT = 4;

    // The **first** document — every reference a placeholder nobody has asked about — is asserted
    // in jsdom with its own mutation proof (`ReadmePanel.test.tsx`), because in the built app the
    // asset round trip is local and completes before this spec could read it. What cannot race is
    // the request count, which is this file's subject.
    //
    // An `ok` asset **stops being a placeholder**: it is replaced by the image the core produced,
    // so document 2 carries one image and one placeholder, not two of either.
    const rendered = await readRevision(BEFORE_CONSENT);
    expect(rendered.placeholders).toEqual(['blocked']);

    const offenders = await app.evaluate(
      () => (globalThis as unknown as { __census: string[] }).__census,
    );
    /* eslint-disable no-console -- this spec exists to print two numbers */
    console.log(`CENSUS requests to other schemes: ${String(offenders.length)}`);
    console.log(`CENSUS rendered block elements: ${String(rendered.blocks)}`);
    console.log(
      `CENSUS placeholders: ${rendered.placeholders.join(', ')} · table cells ${String(rendered.table)} · fences ${String(rendered.code)} · math ${String(rendered.math)}`,
    );
    /* eslint-enable no-console */

    // 1. Nothing left this machine, and nothing the frame holds was fetched by the frame.
    expect(offenders, `requests to other schemes: ${offenders.join(', ')}`).toEqual([]);
    // 2. …and it rendered. Both numbers, because a zero-request run that rendered nothing would
    //    satisfy the first assertion perfectly.
    expect(rendered.blocks).toBeGreaterThan(0);
    expect(rendered.table).toBeGreaterThan(0);
    expect(rendered.code).toBeGreaterThan(0);
    expect(rendered.math).toBeGreaterThan(0);
    // 3. **Exactly one image, and it is the one the core produced.** The local file arrived as
    //    bytes over the protocol; the remote badge is `blocked`, which is the consent state and
    //    is why the count above is 1 and not 2. Every `img` in the frame therefore carries a
    //    `data:` URI, so none of them is a request either.
    expect(rendered.images).toBe(1);
    expect(rendered.dataImages).toBe(1);

    // --- the consent half ---------------------------------------------------------------
    const consent = window.locator('[data-testid="cp-readme-consent"] button');
    await expect(consent).toHaveCount(1);
    // **A DOM-level click, and the reason is measured.** A pointer click through the driver was
    // observed not reaching React's handler here: the panel re-renders while the frame element is
    // being replaced, and a `mousedown` and `mouseup` that land on different nodes produce no
    // click event at all. `el.click()` dispatches one event at one node, which is what the
    // handler under test is bound to. The handler itself is exercised by a pointer press in
    // `ReadmePanel.test.tsx`.
    await consent.evaluate((element: HTMLElement) => {
      element.click();
    });
    // The consent block belongs to the blocked state, so it going away is the grant landing.
    await expect(consent).toHaveCount(0, { timeout: 30_000 });

    // After the grant the core attempts the fetch. The host is `.invalid`, so it cannot
    // resolve — and `unreachable` is a state only a consent-passed attempt can produce.
    const afterGrant = await readRevision(AFTER_CONSENT);
    expect(afterGrant.placeholders).toEqual(['unreachable']);
    // …and the local image is still there, so the grant re-read everything rather than losing it.
    expect(afterGrant.images).toBe(1);

    const afterConsent = await app.evaluate(
      () => (globalThis as unknown as { __census: string[] }).__census,
    );
    // eslint-disable-next-line no-console -- the second half of the census
    console.log(`CENSUS requests after consent: ${String(afterConsent.length)}`);
    // **Still zero.** The core fetches over its own client in another process, so its requests
    // never pass through this session at all — which is exactly what makes this assertion the
    // proof that the frame issued none of them.
    expect(afterConsent, `requests after consent: ${afterConsent.join(', ')}`).toEqual([]);
  } finally {
    await app.close();
  }
});
