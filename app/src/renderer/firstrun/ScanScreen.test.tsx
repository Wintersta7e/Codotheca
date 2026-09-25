import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, expect, test, vi } from 'vitest';
import { ScanScreen, announcement } from './ScanScreen';
import { INITIAL_SCAN_FEED, scanFeedReducer, scanLineText } from './scanFeed';
import { SKIP_AHEAD_LABEL } from '../a11y/names';
import * as copy from './copy';
import css from './firstRun.css?raw';
import type { ScanFeedEvent, ScanFeedState } from './scanFeed';
import type { RevealPanel } from './revealModel';
import type { ProjectId } from '../../generated/protocol';
import { required } from '../../shared/required';

afterEach(cleanup);

const pid = (n: number): ProjectId => n as ProjectId;

const feedWith = (langs: readonly (string | null)[], indexed = langs.length): ScanFeedState => {
  const events: ScanFeedEvent[] = [
    ...langs.map((lang, i) => ({
      kind: 'upserted' as const,
      id: pid(i + 1),
      name: `p${String(i)}`,
      primaryLanguage: lang,
    })),
    { kind: 'flush' as const },
    {
      kind: 'progress' as const,
      indexedProjects: indexed,
      walkedDirs: 214_903,
      foundRepos: 147,
    },
  ];
  return events.reduce(scanFeedReducer, INITIAL_SCAN_FEED);
};

const milestone: RevealPanel = {
  key: 'languageCount',
  label: 'LANGUAGES',
  value: '9',
  wide: false,
  coverage: null,
  caption: '',
  evidence: '',
  signalEdge: false,
  delaySec: 0,
};

function draw(over: Partial<Parameters<typeof ScanScreen>[0]> = {}): {
  jewelFor: (tile: { primaryLanguage: string | null }) => string | null;
  onSkipAhead: ReturnType<typeof vi.fn>;
  onOpenScanSummary: ReturnType<typeof vi.fn>;
} {
  const deps = {
    jewelFor: (tile: { primaryLanguage: string | null }) =>
      tile.primaryLanguage === null ? null : 'oklch(0.7 0.12 250)',
    onSkipAhead: vi.fn(),
    onOpenScanSummary: vi.fn(),
  };
  render(
    <ScanScreen
      deps={deps}
      feed={feedWith(['Rust', null])}
      rootLine={`/somewhere/dev · /somewhere/src · 2 ${copy.ROOTS_SUFFIX}`}
      milestone={null}
      tier="full"
      {...over}
    />,
  );
  return deps;
}

// §10.2 and criterion 12: a count, never a percentage. A bar retreating from 80% to 40%
// poisons every number in the reveal.
test('nothing on the screen is a percentage or a bar', () => {
  draw();
  expect(document.body.textContent).not.toMatch(/%/);
  expect(screen.queryByRole('progressbar')).toBeNull();
});

// §11.7 and §10.3a: present in the first rendered frame, a real button, and that string is also
// its accessible name.
test('SKIP AHEAD is in the first frame even with an empty feed', () => {
  const deps = draw({ feed: INITIAL_SCAN_FEED });
  const button = screen.getByRole('button', { name: SKIP_AHEAD_LABEL });
  expect(button.tagName).toBe('BUTTON');
  fireEvent.click(button);
  expect(deps.onSkipAhead).toHaveBeenCalledTimes(1);
});

// §10.2 is superseded by §10.3a wherever it appears, and must not survive as a label no sighted
// user can see.
test('the superseded escape sentence appears nowhere, visible or not', () => {
  draw();
  expect(document.body.innerHTML).not.toContain("show me what you've got");
});

// §10.3a: the 64px headline is the project count. Directories walked and repositories live on
// the scan line and never in it.
test('the headline is the project count and the scan line carries the rest', () => {
  draw({ feed: feedWith(['Rust', null], 212) });
  expect(screen.getByTestId('fr-count-big').textContent).toBe('212');
  expect(screen.getByTestId('fr-count-big').textContent).not.toContain('214');
  expect(screen.getByText(scanLineText(214_903, 147))).toBeTruthy();
});

test('the scan line is the second entry point to the summary', () => {
  const deps = draw();
  fireEvent.click(screen.getByText(scanLineText(214_903, 147)));
  expect(deps.onOpenScanSummary).toHaveBeenCalledTimes(1);
});

// §10.3a requirement 1: unlit is absent-grey, never missing. An absent-grey stripe reads
// *language not yet known*; a missing stripe reads *no language*.
test('an unclassified tile keeps its stripe and paints it absent', () => {
  draw();
  const stripes = screen.getAllByTestId('fr-tile-stripe');
  expect(stripes).toHaveLength(2);
  expect(stripes[0]?.getAttribute('data-lit')).toBe('true');
  expect(stripes[1]?.getAttribute('data-lit')).toBe('false');
  expect(stripes[1]?.style.background).toBe('');
});

// §10.3a / §7.7: the scan tile sits outside the allocation entirely and is not a card.
test('a scan tile carries no card furniture', () => {
  draw();
  const tile = required(screen.getAllByTestId('fr-tile')[0], 'first tile');
  expect(tile.className).toContain('cdt-fr-tile');
  expect(tile.className).not.toContain('cdt-card');
  expect(tile.querySelector('.cdt-chip')).toBeNull();
  expect(tile.querySelector('.cdt-dot')).toBeNull();
  expect(tile.querySelector('.cdt-langplate')).toBeNull();
});

// §11.7: the count line is role="status" and its text changes only at milestones and at
// completion, so the reading is paced rather than flooded.
test('the live region speaks at milestones and at completion, and not between them', () => {
  const quiet = feedWith(['Rust'], 7);
  expect(announcement(quiet, null)).toBeNull();
  expect(announcement(quiet, milestone)).toBe('7 found · LANGUAGES 9 SO FAR');
  const done = scanFeedReducer(feedWith(['Rust'], 212), { kind: 'finished' });
  expect(announcement(done, null)).toBe('212 found');
});

test('the 64px figure is hidden from the reading, so it is announced once and not per batch', () => {
  draw();
  expect(screen.getByTestId('fr-count-big').getAttribute('aria-hidden')).toBe('true');
  expect(screen.getByRole('status')).toBeTruthy();
});

// §10.3a: mandatory at every coverage, never suppressed.
test('a milestone always carries SO FAR', () => {
  draw({ milestone });
  expect(screen.getByText(copy.SO_FAR_QUALIFIER)).toBeTruthy();
  expect(screen.getByTestId('fr-milestone').textContent).toContain('9');
});

// §10.3a: a milestone whose figure is not yet computable is skipped silently — no zero, no
// dash, no placeholder.
test('an uncomputable milestone renders nothing at all', () => {
  draw({ milestone: { ...milestone, value: null } });
  expect(screen.queryByText(copy.SO_FAR_QUALIFIER)).toBeNull();
  expect(screen.queryByTestId('fr-milestone')).toBeNull();
});

// §10.3a: the beam is decorative, and its period is unrelated to the walk.
test('the beam is inert', () => {
  draw();
  const beam = screen.getByTestId('fr-beam');
  expect(beam.getAttribute('aria-hidden')).toBe('true');
  expect(beam.className).toContain('cdt-fr-beam');
});

// §11.6: the clamp is verified by resolving a style on a real element under each tier, never by
// matching the stylesheet's text — a clamp whose selector names a class nothing renders matches
// nothing, and no text-matching gate says so.
test('the beam stops travelling at the two lower tiers, resolved on the element', () => {
  const style = document.createElement('style');
  style.textContent = css;
  document.head.append(style);
  try {
    const resolved = (tier: 'full' | 'reduced' | 'off'): string => {
      cleanup();
      draw({ tier });
      return getComputedStyle(screen.getByTestId('fr-beam')).animation;
    };
    // The control: at `full` the beam really does resolve a travelling animation. Without this
    // the two assertions below would pass on a stylesheet jsdom never applied.
    expect(resolved('full')).toContain('frBeam');
    expect(resolved('reduced')).not.toContain('frBeam');
    expect(resolved('off')).not.toContain('frBeam');
  } finally {
    style.remove();
  }
});
