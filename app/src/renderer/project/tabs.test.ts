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

function detail(remote: RemoteFacts | null): ProjectDetail {
  return { remote } as unknown as ProjectDetail;
}

describe('AC-P2-25-1 the tab list is per project, not per phase', () => {
  it('mounts two tabs with no remote and three with one, and four in neither case', () => {
    expect(tabsFor(detail(null)).map((t) => t.id)).toEqual(['overview', 'activity']);
    expect(tabsFor(detail(FACTS)).map((t) => t.id)).toEqual(['overview', 'activity', 'remote']);
    expect(tabsFor(detail(null))).toHaveLength(2);
    expect(tabsFor(detail(FACTS))).toHaveLength(3);
  });

  it('names HEALTH in neither, because nothing in phase 2 writes completion_lit', () => {
    for (const list of [tabsFor(detail(null)), tabsFor(detail(FACTS))]) {
      expect(list.map((t) => t.label).join(' ')).not.toMatch(/HEALTH|COMPLETION|CONDITION/u);
    }
    expect(tabsFor(detail(null)).map((t) => t.label)).toEqual(['OVERVIEW', 'ACTIVITY']);
    expect(tabsFor(detail(FACTS)).map((t) => t.label)).toEqual([
      'OVERVIEW',
      'ACTIVITY',
      'REMOTE',
    ]);
  });

  it('cycles the three-tab ring forwards and backwards', () => {
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
  it('cycles the two-tab ring forwards and backwards', () => {
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
