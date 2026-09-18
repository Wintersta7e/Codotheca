import { describe, expect, it } from 'vitest';
import type { ProjectDetail, RemoteFacts } from '../../generated/protocol';
import { cascadeDelay, PAGE_ENTRY, PAGE_EXIT } from './motion';
import { fallbackTab, nextTab, tabsFor } from './tabs';

const FACTS = {
  key: 'github.com/acme/widget',
  linkable: true,
  state: 'not_observed',
  visibility: null,
  forkParentKey: null,
  stars: null,
  openIssues: null,
  goodFirstIssues: null,
  openPrs: null,
  openPrsFromUser: null,
  topics: [],
  observedAt: null,
  ci: { state: 'not_observed', runs: [], observedAt: null },
} as unknown as RemoteFacts;

import type { HealthState } from '../../generated/protocol';

/**
 * [p3] §30.7's predicate reads `detail.health.state`, so the fixture carries one. `absent` is the
 * default because that is what a project with no reading has — a fixture defaulting to `live`
 * would mount the tab for every test that never asked about health.
 */
function detail(remote: RemoteFacts | null, state: HealthState = 'absent'): ProjectDetail {
  return {
    remote,
    health: { state, scoredOpen: null, basis: null, checks: [] },
  } as unknown as ProjectDetail;
}

describe('§25.1 the tab list is per project, not per phase', () => {
  it('AC-P2-25-1 mounts two tabs with no remote and three with one, and four in neither case', () => {
    expect(tabsFor(detail(null)).map((t) => t.id)).toEqual(['overview', 'activity']);
    expect(tabsFor(detail(FACTS)).map((t) => t.id)).toEqual(['overview', 'activity', 'remote']);
    expect(tabsFor(detail(null))).toHaveLength(2);
    expect(tabsFor(detail(FACTS))).toHaveLength(3);
  });

  /**
   * [p3] §30.7 replaces §25.1's HEALTH-is-absent assertion with a **state-driven** one: the tab is
   * mounted on `frozen` and `live` and is **absent — never disabled, never greyed — otherwise**.
   * A greyed tab is the dead control §11.3a's rule exists to stop.
   */
  it('AC-P3-30-17 mounts HEALTH on frozen and live and never on absent or suppressed', () => {
    for (const state of ['absent', 'suppressed'] as const) {
      for (const remote of [null, FACTS]) {
        const labels = tabsFor(detail(remote, state)).map((t) => t.label);
        expect(labels).not.toContain('HEALTH');
      }
    }
    for (const state of ['frozen', 'live'] as const) {
      expect(tabsFor(detail(null, state)).map((t) => t.label)).toEqual([
        'OVERVIEW',
        'ACTIVITY',
        'HEALTH',
      ]);
      // **Four at most, never five**, which is what §8.5's "three at most" becomes.
      const four = tabsFor(detail(FACTS, state));
      expect(four.map((t) => t.label)).toEqual(['OVERVIEW', 'ACTIVITY', 'REMOTE', 'HEALTH']);
      expect(four).toHaveLength(4);
    }
    expect(tabsFor(detail(null)).map((t) => t.label)).toEqual(['OVERVIEW', 'ACTIVITY']);
    expect(tabsFor(detail(FACTS)).map((t) => t.label)).toEqual(['OVERVIEW', 'ACTIVITY', 'REMOTE']);
  });

  it('cycles the four-tab ring forwards and backwards', () => {
    const tabs = tabsFor(detail(FACTS, 'live'));
    expect(nextTab(tabs, 'overview', 1)).toBe('activity');
    expect(nextTab(tabs, 'activity', 1)).toBe('remote');
    expect(nextTab(tabs, 'remote', 1)).toBe('health');
    expect(nextTab(tabs, 'health', 1)).toBe('overview');
    expect(nextTab(tabs, 'overview', -1)).toBe('health');
    expect(nextTab(tabs, 'health', -1)).toBe('remote');
  });

  it('falls back to overview when a held HEALTH tab stops being mounted', () => {
    // The store came back and the reading went from `frozen` to `absent` while the page was open.
    expect(fallbackTab(tabsFor(detail(FACTS, 'absent')), 'health')).toBe('overview');
    expect(fallbackTab(tabsFor(detail(FACTS, 'live')), 'health')).toBe('health');
  });

  it('AC-P2-25-1-cycle3 cycles the three-tab ring forwards and backwards', () => {
    const tabs = tabsFor(detail(FACTS));
    expect(nextTab(tabs, 'overview', 1)).toBe('activity');
    expect(nextTab(tabs, 'activity', 1)).toBe('remote');
    expect(nextTab(tabs, 'remote', 1)).toBe('overview');
    expect(nextTab(tabs, 'overview', -1)).toBe('remote');
    expect(nextTab(tabs, 'remote', -1)).toBe('activity');
    expect(nextTab(tabs, 'activity', -1)).toBe('overview');
  });

  // Two tests, not one: a modulo bug over two elements is invisible, so the two-tab ring is
  // asserted separately rather than as a special case of the three-tab one.
  it('AC-P2-25-1-cycle2 cycles the two-tab ring forwards and backwards', () => {
    const tabs = tabsFor(detail(null));
    expect(nextTab(tabs, 'overview', 1)).toBe('activity');
    expect(nextTab(tabs, 'activity', 1)).toBe('overview');
    expect(nextTab(tabs, 'overview', -1)).toBe('activity');
    expect(nextTab(tabs, 'activity', -1)).toBe('overview');
  });

  it('falls back to overview when the held tab is no longer mounted', () => {
    const tabs = tabsFor(detail(null));
    // A project whose remote binding went away while its page was open would otherwise leave the
    // page pointing at a panel that is not in the list — the dead control §8.5 exists to stop.
    expect(fallbackTab(tabs, 'remote')).toBe('overview');
    expect(fallbackTab(tabs, 'activity')).toBe('activity');
    expect(nextTab(tabs, 'remote', 1)).toBe('overview');
  });
});

describe('the beats', () => {
  it('swaps the view at 620ms, which is what puts the roast on screen after its observation', () => {
    expect(PAGE_ENTRY.viewSwapMs).toBe(620);
    expect(PAGE_ENTRY.powerOnMs).toBe(620);
  });

  it('slides the rail and rises the column on the two durations §8.5.1 states', () => {
    expect(PAGE_ENTRY.railMs).toBe(400);
    expect(PAGE_ENTRY.riseMs).toBe(420);
  });

  it('cascades the right column on an 80ms step from 100ms', () => {
    expect(PAGE_ENTRY.cascadeStartMs).toBe(100);
    expect(PAGE_ENTRY.cascadeStepMs).toBe(80);
    expect(cascadeDelay(0)).toBe('100ms');
    expect(cascadeDelay(1)).toBe('180ms');
    expect(cascadeDelay(2)).toBe('260ms');
    // A negative step is a caller's arithmetic slip, not a reason to start before the page.
    expect(cascadeDelay(-3)).toBe('100ms');
  });

  it('lands the user back where they came from rather than at the top', () => {
    expect(PAGE_EXIT.rackOutMs).toBe(340);
    expect(PAGE_EXIT.shelfRestoredMs).toBe(300);
    expect(PAGE_EXIT.landingClearedMs).toBe(1600);
  });
});
