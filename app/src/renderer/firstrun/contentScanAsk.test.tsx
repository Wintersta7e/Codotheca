/**
 * **AC-P3-29-20** — first run asks no new question.
 *
 * Criterion 12 still passes: the content-scan ask appears on **no** first-run screen. §29.8 rules
 * the ask is in context, the first time a surface would show a debt item, and no surface does in
 * wave 1 — so what lands is a control in settings and nothing here.
 *
 * Asserted over the rendered tree of every first-run screen, **printing the screen count**.
 */
import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { RootsScreen } from './RootsScreen.js';
import { ScanScreen } from './ScanScreen.js';
import { RevealScreen } from './RevealScreen.js';
import { TurnScreen } from './TurnScreen.js';
import { INITIAL_SCAN_FEED } from './scanFeed.js';
import { toRow } from './rootRows.js';
import { CONSENT_ROWS } from './copy.js';
import { CONTENT_SCAN_LABEL } from '../../shared/contentScan.js';
import type { ProjectId, Reveal, RevealBasis, RootSuggestion } from '../../generated/protocol.js';
import type { RevealDeps } from './revealModel.js';

afterEach(cleanup);

const NOW = 1_760_000_000;
const pid = (n: number): ProjectId => n as ProjectId;
const basis: RevealBasis = { projectsCovered: 4, projectsTotal: 4, historyComplete: true };

const suggestion: RootSuggestion = {
  pathDisplay: '/somewhere/dev',
  kind: 'linux',
  distro: '',
  provenance: 'gitconfig',
  provenanceDetail: 'includeif',
  hits: 3,
  preTicked: true,
};

const reveal: Reveal = {
  spanDays: { value: 400, basis },
  projectCount: { value: 4, basis },
  languageCount: { value: 2, basis },
  bestYear: { value: 2021, basis },
  playtimeSeconds: { value: 0, basis },
  oldestStillAlive: { projectId: pid(7), firstCommitAt: NOW - 400 * 86_400, basis },
};

const revealDeps: RevealDeps = {
  nowSecs: NOW,
  languageTally: [{ name: 'Rust', count: 2 }],
  referenceCount: 0,
  project: () => ({ name: 'ledger', birthYear: 2014, primaryLanguage: 'Rust' }),
};

/** Every first-run screen, each drawn the way its own suite draws it. */
const SCREENS: readonly { readonly name: string; readonly draw: () => void }[] = [
  {
    name: 'roots',
    draw: () => {
      render(
        <RootsScreen
          deps={{
            onToggleRoot: vi.fn(),
            onConsent: vi.fn(),
            onAddFolder: vi.fn(),
            onConfirmLarge: vi.fn(),
            onDig: vi.fn(),
          }}
          rows={[toRow(suggestion, true)]}
          ticked={new Set(['/somewhere/dev'])}
          consented
          pendingConfirm={null}
          tier="full"
          busy={false}
        />,
      );
    },
  },
  {
    name: 'scanning',
    draw: () => {
      render(
        <ScanScreen
          deps={{ jewelFor: () => null, onSkipAhead: vi.fn(), onOpenScanSummary: vi.fn() }}
          feed={INITIAL_SCAN_FEED}
          rootLine="<home>/code · 1 ROOT"
          milestone={null}
          tier="full"
        />,
      );
    },
  },
  {
    name: 'reveal',
    draw: () => {
      render(<RevealScreen reveal={reveal} deps={revealDeps} tier="full" onGoOn={vi.fn()} />);
    },
  },
  {
    name: 'turn',
    draw: () => {
      render(
        <TurnScreen
          counts={{ dirty: 0, unpushed: 0, interrupted: 0, total: 4 }}
          worktreeObservedAt={NOW}
          tier="full"
          onShowMe={vi.fn()}
          onNotNow={vi.fn()}
        />,
      );
    },
  },
];

it('AC-P3-29-20 no first-run screen offers the content-scan grant as a control', () => {
  // `console.warn` rather than stderr: this file is in the web project and has no node types.
  console.warn(`contentScanAsk: first-run screens rendered ${String(SCREENS.length)}`);
  expect(SCREENS.length).toBeGreaterThan(0);
  for (const screenUnderTest of SCREENS) {
    cleanup();
    screenUnderTest.draw();
    const where = screenUnderTest.name;
    expect(
      screen.queryByRole('switch', { name: CONTENT_SCAN_LABEL }),
      `${where} draws the grant as a switch`,
    ).toBeNull();
    expect(
      screen.queryByRole('checkbox', { name: CONTENT_SCAN_LABEL }),
      `${where} draws the grant as a checkbox`,
    ).toBeNull();
    expect(
      screen.queryByRole('button', { name: CONTENT_SCAN_LABEL }),
      `${where} draws the grant as a button`,
    ).toBeNull();
    // Criterion 12's own shape: first run asks zero configuration questions.
    expect(screen.queryByRole('textbox'), where).toBeNull();
    expect(screen.queryByRole('combobox'), where).toBeNull();
    expect(screen.queryByRole('radio'), where).toBeNull();
    expect(screen.queryByRole('spinbutton'), where).toBeNull();
  }
});

it('AC-P3-29-20 the consent row that names the grant takes no input', () => {
  const row = CONSENT_ROWS[1];
  expect(row?.kind).toBe('statement');
  cleanup();
  SCREENS[0]?.draw();
  // The row is drawn, and it is drawn without a control: §10.1b's rule is that every switch must
  // do something observable or be presented as a statement.
  const body = screen.getByText(row?.body ?? '__unset__');
  expect(body).toBeTruthy();
  expect(body.closest('[role="switch"], button, input')).toBeNull();
});
