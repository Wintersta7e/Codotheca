import { describe, expect, it } from 'vitest';
import { cascadeDelay, PAGE_ENTRY, PAGE_EXIT } from './motion';
import { nextTab, PROJECT_TABS } from './tabs';

describe('the tab list', () => {
  it('ships exactly two tabs — Health and Remote are absent, not disabled', () => {
    expect(PROJECT_TABS.map((t) => t.id)).toEqual(['overview', 'activity']);
    expect(PROJECT_TABS.map((t) => t.label)).toEqual(['OVERVIEW', 'ACTIVITY']);
  });

  it('names no tab phase 1 cannot fill', () => {
    const labels = PROJECT_TABS.map((t) => t.label).join(' ');
    expect(labels).not.toMatch(/HEALTH|REMOTE|COMPLETION|CONDITION/);
  });

  it('cycles over the mounted list, not a modulo-4 ring', () => {
    expect(nextTab('overview', 1)).toBe('activity');
    expect(nextTab('activity', 1)).toBe('overview');
    expect(nextTab('overview', -1)).toBe('activity');
    expect(nextTab('activity', -1)).toBe('overview');
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
