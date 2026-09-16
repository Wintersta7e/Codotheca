/**
 * [p2] §25.1: **presence is per project, not per phase.** The `REMOTE` tab is mounted when this
 * project has a remote and is **absent** — never disabled, never greyed — when it does not. A
 * greyed tab is exactly the dead control the standing rule exists to stop. This supersedes
 * §8.5's *"two tabs ship… Remote needs a token"*, whose scope was the whole phase.
 *
 * `HEALTH` stays absent and its phase-1 reason has not expired: nothing in phase 2 writes
 * `completion_lit` (§1.2), so the tab would have nothing to say on any project.
 *
 * §11.7: tabs cycle over the *mounted* list, which is now per project rather than a module
 * constant. The prototype's modulo-4 ring names two tabs that do not exist here, and a ring over
 * a list the page does not hold is the same defect one step along.
 */
import type { ProjectDetail } from '../../generated/protocol';

export type ProjectTab = 'overview' | 'activity' | 'remote';

export interface ProjectTabEntry {
  readonly id: ProjectTab;
  readonly label: string;
}

const OVERVIEW: ProjectTabEntry = { id: 'overview', label: 'OVERVIEW' };
const ACTIVITY: ProjectTabEntry = { id: 'activity', label: 'ACTIVITY' };
const REMOTE: ProjectTabEntry = { id: 'remote', label: 'REMOTE' };

/**
 * The two every project mounts, and what the bar draws while the detail is still loading — a
 * tab bar that empties and refills between frames is worse than one that gains a tab.
 */
export const BASE_PROJECT_TABS: readonly ProjectTabEntry[] = [OVERVIEW, ACTIVITY];

/**
 * The tabs this project mounts. Two or three, never four.
 *
 * `detail.remote` is NULL **iff** `project.remote_key` is NULL, which is the core's own presence
 * predicate — the renderer does not re-derive it from a key it would have to parse.
 */
export function tabsFor(detail: ProjectDetail): readonly ProjectTabEntry[] {
  return detail.remote === null ? BASE_PROJECT_TABS : [...BASE_PROJECT_TABS, REMOTE];
}

/** The held tab, or `overview` when it is no longer in the mounted list. */
export function fallbackTab(tabs: readonly ProjectTabEntry[], current: ProjectTab): ProjectTab {
  return tabs.some((tab) => tab.id === current) ? current : 'overview';
}

/**
 * `←`/`→` over the mounted list. A tab that is no longer mounted lands on `overview` rather than
 * stepping from an index the list does not have.
 */
export function nextTab(
  tabs: readonly ProjectTabEntry[],
  current: ProjectTab,
  delta: 1 | -1,
): ProjectTab {
  const index = tabs.findIndex((tab) => tab.id === current);
  const count = tabs.length;
  if (index < 0 || count === 0) return 'overview';
  const moved = (index + delta + count) % count;
  return tabs[moved]?.id ?? 'overview';
}
