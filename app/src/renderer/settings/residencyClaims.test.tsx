/**
 * §48.3 rows 12–14 (PA46.6): this build quits when its window closes, keeps no tray icon and
 * registers no login item. So no residency claim the drawer or a notice renders may stand
 * unqualified: each is absent, carries `NOT IN THIS BUILD`, or is backed by a production caller
 * that `p4-42` drives through the production path when it makes residency real — and extends
 * this file's sources to do so.
 */
import { cleanup, render, screen, within } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import type { Settings } from '../../generated/protocol.js';
import * as noticeCopy from '../notices/copy.js';
import { NOT_IN_THIS_BUILD } from './groupsData.js';
import { MotionGroups } from './groupsMotion.js';

afterEach(cleanup);

const CLAIM =
  /thirty minutes|30 minutes|30-minute|tray|start with the system|login item|destroyed|MB EMPTY|MB WITH/iu;

const settings: Settings = {
  effectsTier: 'auto',
  reducedMotionOverride: false,
  autostart: false,
  residentShortcut: null,
  roastEnabled: true,
  logLevel: 'info',
  installRootId: null,
  contentScanEnabled: false,
  healthChecks: [],
};

/** Every string a zero-argument notice or a string constant of `notices/copy.ts` renders. */
function noticeStrings(): string[] {
  const out: string[] = [];
  for (const value of Object.values(noticeCopy)) {
    if (typeof value === 'string') out.push(value);
    if (typeof value === 'function' && value.length === 0) {
      const copy: unknown = (value as () => unknown)();
      if (typeof copy !== 'object' || copy === null) continue;
      for (const field of Object.values(copy)) {
        if (typeof field === 'string') out.push(field);
      }
    }
  }
  return out;
}

it('AC-P4-48-9 no residency claim the drawer or a notice renders is unqualified', () => {
  render(
    <MotionGroups
      settings={settings}
      shortcut={{ chord: null, registered: false }}
      recording={false}
      onPatch={vi.fn()}
      onRecordChord={vi.fn()}
      onChordCaptured={vi.fn()}
    />,
  );
  const group = screen.getByRole('group', { name: 'RESIDENCY' });
  const switches = within(group).queryAllByRole('switch');
  const texts: string[] = [];
  const walker = document.createTreeWalker(group, NodeFilter.SHOW_TEXT);
  for (let node = walker.nextNode(); node !== null; node = walker.nextNode()) {
    const text = node.textContent?.trim() ?? '';
    if (text !== '') texts.push(text);
  }
  const strings = [...texts, ...noticeStrings()];

  const switchClaims = switches
    .map((s) => s.getAttribute('aria-label') ?? '')
    .filter((name) => CLAIM.test(name));
  const claims = strings.filter((s) => CLAIM.test(s));
  const unqualified = claims.filter((s) => !s.includes(NOT_IN_THIS_BUILD));
  console.warn(
    `strings scanned: ${String(strings.length)}, claims: ${String(claims.length)}, ` +
      `qualified: ${String(claims.length - unqualified.length)}`,
  );

  expect(strings.length, 'a scan of nothing proves nothing').toBeGreaterThan(0);
  expect(texts.length, 'the RESIDENCY group rendered no text').toBeGreaterThan(0);
  expect(switchClaims, 'a residency switch writes a setting nothing in this build reads').toEqual(
    [],
  );
  expect(unqualified).toEqual([]);
});
