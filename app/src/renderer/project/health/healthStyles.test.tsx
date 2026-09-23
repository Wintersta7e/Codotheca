/**
 * [p3] §30.7 — the `HEALTH` tab under its stylesheet, through the real page: the design's list
 * idiom with no list markers, one form per outcome, and the grant asked in the app's own button.
 *
 * jsdom resolves declarations and does no layout, so this reads what each element is given — the
 * token names, not the pixels. The pixels are checked by driving the app.
 */
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import type {
  CheckOutcome,
  CompletionDetail,
  DebtItem,
  HealthReading,
  ProjectDetail,
  ProjectId,
  Settings,
} from '../../../generated/protocol';
import projectPageCss from '../../styles/projectPage.css?raw';
import { ProjectPageDepsContext, type ProjectPageDeps } from '../deps';
import { ProjectPageView } from '../ProjectPage';
import { DAY, detailFixture, NOW } from '../testFixtures';
import { COMPLETION_TITLE } from '../completion/CompletionChecklist';
import { DEBT_LIST_TITLE } from './DebtList';
import { HEALTH_TITLE } from './HealthTab';

afterEach(cleanup);

const style = document.createElement('style');
beforeAll(() => {
  expect(projectPageCss.length, 'projectPage.css?raw imported as an empty string').toBeGreaterThan(
    0,
  );
  style.textContent = projectPageCss;
  document.head.append(style);
});
afterAll(() => {
  style.remove();
});

/** The source grant withheld and one check switched off: both causes of `off` on one tab. */
const SETTINGS: Settings = {
  effectsTier: 'auto',
  reducedMotionOverride: false,
  autostart: false,
  residentShortcut: null,
  roastEnabled: true,
  logLevel: 'info',
  installRootId: null,
  contentScanEnabled: false,
  healthChecks: [
    { check: 'todo_marker', enabled: true },
    { check: 'missing_tests', enabled: false },
  ],
};

/** One check per outcome, so every form is on the page at once. */
const HEALTH: HealthReading = {
  state: 'live',
  scoredOpen: 1,
  basis: { ran: 2, eligible: 4, unknown: 1, off: 2, notApplicable: 1, observedAt: NOW - 30 },
  checks: [
    { id: 'missing_readme', outcome: 'ok', unknownReason: null },
    { id: 'missing_license', outcome: 'failed', unknownReason: null },
    { id: 'dependency_advisory', outcome: 'unknown', unknownReason: 'unreachable' },
    { id: 'missing_tests', outcome: 'off', unknownReason: null },
    { id: 'todo_marker', outcome: 'off', unknownReason: null },
    { id: 'ci_red', outcome: 'notApplicable', unknownReason: null },
  ],
};

const LICENSE: DebtItem = {
  source: 'missing_license',
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
};

const COMPLETION: CompletionDetail = {
  lit: 1,
  evaluable: 2,
  unknown: 1,
  checks: [
    { key: 'readme', state: 'pass', userNa: null, unknownReason: null, observedAt: NOW },
    { key: 'license', state: 'fail', userNa: null, unknownReason: null, observedAt: NOW },
    {
      key: 'pushed',
      state: 'unknown',
      userNa: null,
      unknownReason: 'notObserved',
      observedAt: NOW,
    },
    { key: 'release', state: 'na', userNa: null, unknownReason: null, observedAt: NOW },
  ],
};

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

/** Mounts the real page with every block of the tab present and opens `HEALTH`. */
async function healthTab(): Promise<HTMLElement> {
  const detail = detailFixture({ health: HEALTH, debt: [LICENSE], completion: COMPLETION });
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
  // The ask needs the settings reply, so waiting for it waits for the whole tab.
  await screen.findByTestId('cp-health-ask');
  return screen.findByTestId('cp-tabpanel');
}

function one(root: ParentNode, selector: string): HTMLElement {
  const found = root.querySelectorAll<HTMLElement>(selector);
  expect(found, selector).toHaveLength(1);
  return found[0] as HTMLElement;
}

describe('§30.7 the HEALTH tab wears the design, not the browser default', () => {
  it('draws its three lists with no list markers', async () => {
    const panel = await healthTab();
    const lists = ['.cp-health-checks', '.cp-health-debt-items', '.cp-completion-checks'];
    for (const selector of lists) {
      expect(getComputedStyle(one(panel, selector)).listStyleType, selector).toBe('none');
    }
    console.warn(`[healthStyles] lists scanned: ${String(lists.length)}`);
  });

  it('heads its blocks as the design heads a section, and the separators give way to layout', async () => {
    const panel = await healthTab();
    const heads = [...panel.querySelectorAll('h2, h3')].map((head) => head.textContent);
    expect(heads).toEqual([HEALTH_TITLE, DEBT_LIST_TITLE, COMPLETION_TITLE]);
    for (const selector of ['.cp-health-title', '.cp-health-debt-title', '.cp-completion-title']) {
      expect(getComputedStyle(one(panel, selector)).fontFamily, selector).toContain(
        '--font-display',
      );
    }
    const separators = [...panel.querySelectorAll<HTMLElement>('.cp-sep')];
    expect(separators.length).toBeGreaterThan(0);
    for (const separator of separators) expect(getComputedStyle(separator).display).toBe('none');
  });

  it('gives every outcome its own form, and none but ok the passing one', async () => {
    const panel = await healthTab();
    const formOf = (outcome: CheckOutcome, check: string): string => {
      const word = one(panel, `[data-check="${check}"] .cp-health-check-word`);
      expect(word.closest('[data-outcome]')?.getAttribute('data-outcome')).toBe(outcome);
      const css = getComputedStyle(word);
      // jsdom drops a `var()` inside the four-side `border-color`, so a border's colour is read
      // in pixels; its style is a longhand here and resolves.
      return [css.color, css.backgroundColor, css.borderTopStyle].join('|');
    };
    const forms = new Map<string, string>([
      ['ok', formOf('ok', 'missing_readme')],
      ['failed', formOf('failed', 'missing_license')],
      ['unknown', formOf('unknown', 'dependency_advisory')],
      ['off', formOf('off', 'missing_tests')],
      ['notApplicable', formOf('notApplicable', 'ci_red')],
    ]);
    // Both causes of `off` are one outcome, and one form.
    expect(formOf('off', 'todo_marker')).toBe(forms.get('off'));
    expect(new Set(forms.values()).size, [...forms].join('\n')).toBe(forms.size);
    expect(forms.get('ok')).toContain('--pass');
    expect(forms.get('failed')).toContain('--fail');
    expect(forms.get('unknown')).toContain('--unknown');
    for (const outcome of ['failed', 'unknown', 'off', 'notApplicable']) {
      expect(forms.get(outcome), outcome).not.toContain('--pass');
    }
  });

  it("asks for the grant in the app's own button, not the platform's", async () => {
    const panel = await healthTab();
    const action = getComputedStyle(one(panel, '.cp-health-ask-action'));
    expect(action.backgroundColor).toContain('--surface-3');
    expect(action.borderTopStyle).toBe('solid');
    expect(action.fontFamily).toContain('--font-mono');
  });
});
