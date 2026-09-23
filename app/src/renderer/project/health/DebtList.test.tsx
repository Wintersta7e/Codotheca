/**
 * [p3] §33.2 and §30.7 — the `HEALTH` tab lists every item it is handed, in text, grouped by
 * layer, through the real page.
 */
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import type {
  DebtItem,
  HealthReading,
  ProjectDetail,
  ProjectId,
  Settings,
} from '../../../generated/protocol';
import { ProjectPageDepsContext, type ProjectPageDeps } from '../deps';
import { ProjectPageView } from '../ProjectPage';
import { DAY, detailFixture, NOW } from '../testFixtures';
import { DEBT_LIST_TITLE, SHOWN_ONLY_NOTE, UNVERIFIED_NOTE } from './DebtList';

afterEach(cleanup);

const SETTINGS: Settings = {
  effectsTier: 'auto',
  reducedMotionOverride: false,
  autostart: false,
  residentShortcut: null,
  roastEnabled: true,
  logLevel: 'info',
  installRootId: null,
  contentScanEnabled: true,
  healthChecks: [],
};

const LIVE: HealthReading = {
  state: 'live',
  scoredOpen: 3,
  basis: { ran: 2, eligible: 3, unknown: 1, off: 0, notApplicable: 0, observedAt: NOW - 30 },
  checks: [
    { id: 'missing_readme', outcome: 'failed', unknownReason: null },
    { id: 'dependency_advisory', outcome: 'failed', unknownReason: null },
    { id: 'todo_marker', outcome: 'unknown', unknownReason: 'unreachable' },
  ],
};

function item(over: Partial<DebtItem>): DebtItem {
  return {
    source: 'missing_readme',
    fingerprint: '',
    state: 'open',
    scoring: 'scored',
    layer: 'dust',
    pathDisplay: null,
    line: null,
    column: null,
    salientText: null,
    firstSeenAt: NOW - 30 * DAY,
    lastSeenAt: NOW - 60,
    basis: 'head',
    advisory: null,
    ...over,
  };
}

const README = item({ source: 'missing_readme' });
const LICENSE = item({ source: 'missing_license' });
const MARKER = item({
  source: 'todo_marker',
  fingerprint: 'f00d:0',
  state: 'unverified',
  layer: 'overgrowth',
  pathDisplay: 'src/main.rs',
  line: 12,
  column: 5,
  salientText: 'TODO: a shaped marker',
});
const FIXABLE = item({
  source: 'dependency_advisory',
  fingerprint: 'npm:shaped-pkg:GHSA-aaaa-bbbb-cccc',
  layer: 'rust',
  basis: 'worktree',
  advisory: {
    ecosystem: 'npm',
    packageName: 'shaped-pkg',
    advisoryId: 'GHSA-aaaa-bbbb-cccc',
    cveIds: ['CVE-2026-0001', 'CVE-2026-0002'],
    severity: 'moderate',
    fixedVersion: '2.0.1',
  },
});
const UNFIXABLE = item({
  source: 'dependency_advisory',
  fingerprint: 'npm:other-pkg:GHSA-dddd-eeee-ffff',
  scoring: 'shown_only',
  layer: 'rust',
  basis: 'worktree',
  advisory: {
    ecosystem: 'npm',
    packageName: 'other-pkg',
    advisoryId: 'GHSA-dddd-eeee-ffff',
    cveIds: [],
    severity: null,
    fixedVersion: null,
  },
});
/** An advisory item the core has not yet attached attributes to: its fingerprint names it. */
const BARE = item({
  source: 'dependency_advisory',
  fingerprint: 'pip:third-pkg:GHSA-gggg-hhhh-iiii',
  layer: 'rust',
  basis: 'worktree',
});

const DEBT = [README, MARKER, FIXABLE, LICENSE, UNFIXABLE, BARE];

function depsFor(detail: ProjectDetail): ProjectPageDeps {
  return {
    request: ((name: string) => {
      if (name === 'projects.get') return Promise.resolve(detail);
      if (name === 'settings.get') return Promise.resolve(SETTINGS);
      if (name === 'art.url') return Promise.resolve('codotheca://art/aa/hero');
      return Promise.resolve({});
    }) as unknown as ProjectPageDeps['request'],
    relocate: () => Promise.resolve({ kind: 'cancelled' }),
    uninstall: () => Promise.resolve({ kind: 'refused' as const, verdict: null }),
    installStart: () =>
      Promise.resolve({ kind: 'started' as const, start: { runId: 1, refusedBecause: null } }),
    installCancel: () => Promise.resolve({ kind: 'cancelled' as const }),
    pickRoot: () => Promise.resolve({ kind: 'cancelled' as const }),
    openRemoteLink: () => Promise.resolve({ kind: 'not_linkable' }),
    subscribe: () => () => undefined,
    now: () => NOW,
  };
}

/** Mounts the real page and opens its `HEALTH` tab. */
async function healthTab(detail: ProjectDetail): Promise<HTMLElement> {
  render(
    <ProjectPageDepsContext.Provider value={depsFor(detail)}>
      <ProjectPageView
        projectId={7 as unknown as ProjectId}
        onBack={vi.fn()}
        onOpenProject={vi.fn()}
      />
    </ProjectPageDepsContext.Provider>,
  );
  fireEvent.click(await screen.findByRole('tab', { name: 'HEALTH' }));
  return screen.findByTestId('cp-tabpanel');
}

async function debtList(debt: readonly DebtItem[] = DEBT): Promise<HTMLElement> {
  await healthTab(detailFixture({ health: LIVE, debt: [...debt] }));
  return screen.findByTestId('cp-health-debt');
}

/** The one row whose text carries `text`, which each fixture item has uniquely. */
const rowWith = (list: HTMLElement, text: string): HTMLElement => {
  const rows = [...list.querySelectorAll<HTMLElement>('.cp-health-debt-item')].filter((row) =>
    (row.textContent ?? '').includes(text),
  );
  expect(rows, text).toHaveLength(1);
  return rows[0] as HTMLElement;
};

describe('§33.2 the HEALTH tab lists every item, in text', () => {
  it('lists every item grouped by layer, and a layer with no items draws no group', async () => {
    const list = await debtList();
    const rows = list.querySelectorAll('.cp-health-debt-item');
    console.warn(
      `§33.2 items handed: ${String(DEBT.length)}; rows rendered: ${String(rows.length)}`,
    );
    expect(DEBT.length).toBeGreaterThan(0);
    expect(rows).toHaveLength(DEBT.length);

    const layers = [...list.querySelectorAll('[data-layer]')].map((g) =>
      g.getAttribute('data-layer'),
    );
    expect(layers).toEqual(['dust', 'rust', 'overgrowth']);
    // Absent, not empty: no heading, no list, nothing at all for a layer nothing lit.
    expect(list.querySelector('[data-layer="cobwebs"]')).toBeNull();
    expect(list.querySelector('[data-layer="cracks"]')).toBeNull();
    expect(list.textContent ?? '').not.toMatch(/COBWEBS|CRACKS/u);

    for (const entry of DEBT) {
      const group = list.querySelector(`[data-layer="${entry.layer}"]`);
      expect(group?.textContent ?? '', entry.fingerprint).toContain(entry.source);
    }
    expect(list.querySelector('h3')?.textContent).toBe(DEBT_LIST_TITLE);

    const marker = rowWith(list, 'todo_marker');
    expect(marker.textContent).toContain('src/main.rs:12:5');
    expect(marker.textContent).toContain('TODO: a shaped marker');
    // A content fingerprint is a hash; the path and the text say more, so it stays unrendered.
    expect(marker.textContent).not.toContain(MARKER.fingerprint);
    // An advisory item with no attributes attached yet is named by its fingerprint.
    rowWith(list, BARE.fingerprint);
  });

  // The renderer half of the criterion — *a `shown_only` item renders*. The core half, that it
  // is counted by nothing and pays nothing, is `core/tests/debt_lifecycle.rs`.
  it('AC-P3-28-14 shown_only and unverified items render and say they are not counted', async () => {
    const list = await debtList();
    const counted = rowWith(list, 'missing_readme');
    const unverified = rowWith(list, 'todo_marker');
    const shownOnly = rowWith(list, 'other-pkg');
    expect(UNVERIFIED_NOTE).not.toBe('');
    expect(SHOWN_ONLY_NOTE).not.toBe('');

    expect(unverified.textContent).toContain(UNVERIFIED_NOTE);
    expect(unverified.getAttribute('data-state')).toBe('unverified');
    expect(shownOnly.textContent).toContain(SHOWN_ONLY_NOTE);
    expect(shownOnly.getAttribute('data-scoring')).toBe('shown_only');
    for (const note of [UNVERIFIED_NOTE, SHOWN_ONLY_NOTE]) {
      expect(counted.textContent).not.toContain(note);
    }
    expect(counted.getAttribute('data-state')).toBe('open');
    expect(counted.getAttribute('data-scoring')).toBe('scored');
  });

  it('an advisory item shows its advisory id, severity as spelled, fix version and every CVE id', async () => {
    const list = await debtList();
    const fixable = rowWith(list, 'shaped-pkg');
    for (const part of [
      'shaped-pkg',
      'GHSA-aaaa-bbbb-cccc',
      'CVE-2026-0001',
      'CVE-2026-0002',
      'severity moderate',
      'fixed in 2.0.1',
    ]) {
      expect(fixable.textContent, part).toContain(part);
    }
    // No CVE id to name: the advisory is named by its own id, and nothing claims a fix.
    const unfixable = rowWith(list, 'other-pkg');
    expect(unfixable.textContent).toContain('GHSA-dddd-eeee-ffff');
    expect(unfixable.textContent).not.toMatch(/CVE-|fixed in|severity/u);
  });

  it('renders no count, and never the words clean healthy none or all', async () => {
    const list = await debtList();
    const headings = [...list.querySelectorAll('h3, h4')];
    expect(headings.length).toBeGreaterThan(0);
    for (const heading of headings) expect(heading.textContent ?? '').not.toMatch(/\d/u);

    const text = list.textContent ?? '';
    expect(text).not.toBe('');
    const names = [...list.querySelectorAll('*'), list]
      .map((n) => n.getAttribute('aria-label') ?? '')
      .join(' ');
    const classes = [...list.querySelectorAll('*'), list].map((n) => n.className).join(' ');
    for (const banned of ['clean', 'healthy', 'none', 'all']) {
      const pattern = new RegExp(`\\b${banned}\\b`, 'iu');
      expect(text).not.toMatch(pattern);
      expect(names).not.toMatch(pattern);
      expect(classes).not.toMatch(pattern);
    }
  });

  it('draws no list when there is no item, and none on a page with no HEALTH tab', async () => {
    const panel = await healthTab(detailFixture({ health: LIVE, debt: [] }));
    await screen.findByTestId('cp-health');
    expect(panel.querySelector('[data-testid="cp-health-debt"]')).toBeNull();
    cleanup();

    // §30.7: an `absent` reading mounts no tab, and the items reach no other panel.
    render(
      <ProjectPageDepsContext.Provider value={depsFor(detailFixture({ debt: [README, MARKER] }))}>
        <ProjectPageView
          projectId={7 as unknown as ProjectId}
          onBack={vi.fn()}
          onOpenProject={vi.fn()}
        />
      </ProjectPageDepsContext.Provider>,
    );
    await screen.findByTestId('cp-condition');
    expect(screen.queryByRole('tab', { name: 'HEALTH' })).toBeNull();
    expect(document.querySelector('[data-testid="cp-health-debt"]')).toBeNull();
  });
});
