/**
 * [p2] §25.1: **presence is per project, not per phase.** The `REMOTE` tab is mounted when this
 * project has a remote and is **absent** — never disabled, never greyed — when it does not. A
 * greyed tab is exactly the dead control the standing rule exists to stop. This supersedes
 * §8.5's *"two tabs ship… Remote needs a token"*, whose scope was the whole phase.
 *
 * [p3] §30.7: **`HEALTH` is mounted when the reading's state is `frozen` or `live`, and is
 * absent — never disabled, never greyed — otherwise.** The phase-1 reason (*"nothing writes
 * `completion_lit`"*) has expired: the tab renders **the reading**, and a checklist with every
 * row `unknown` is exactly the surface §31.8 argues must not be hidden.
 *
 * An `absent` reading mounts no tab; a `suppressed` one mounts no tab, and after §30.5's
 * `acknowledged_at` writer that can only mean `is_archived = 1` — the settled *no health nagging
 * for an archived project*, satisfied by construction rather than by a second rule.
 *
 * **§30.7 is the tab's only mount predicate** (R130/F7). §31 carries no second one.
 *
 * §8.5's *"Three at most, never four"* becomes **four at most, never five**.
 *
 * §11.7: tabs cycle over the *mounted* list, which is now per project rather than a module
 * constant. The prototype's modulo-4 ring names two tabs that do not exist here, and a ring over
 * a list the page does not hold is the same defect one step along.
 */
import type { ProjectDetail } from '../../generated/protocol';

export type ProjectTab = 'overview' | 'activity' | 'remote' | 'health';

export interface ProjectTabEntry {
  readonly id: ProjectTab;
  readonly label: string;
}

const OVERVIEW: ProjectTabEntry = { id: 'overview', label: 'OVERVIEW' };
const ACTIVITY: ProjectTabEntry = { id: 'activity', label: 'ACTIVITY' };
const REMOTE: ProjectTabEntry = { id: 'remote', label: 'REMOTE' };
const HEALTH: ProjectTabEntry = { id: 'health', label: 'HEALTH' };

/**
 * The two every project mounts, and what the bar draws while the detail is still loading — a
 * tab bar that empties and refills between frames is worse than one that gains a tab.
 */
export const BASE_PROJECT_TABS: readonly ProjectTabEntry[] = [OVERVIEW, ACTIVITY];

/**
 * The tabs this project mounts. Two to four, never five.
 *
 * `detail.remote` is NULL **iff** `project.remote_key` is NULL, which is the core's own presence
 * predicate — the renderer does not re-derive it from a key it would have to parse.
 *
 * [p3] §30.7's predicate is `detail.health.state`, read and never re-derived: `HealthState` says
 * which reading this page is looking at, and a second derivation of *absent* or *suppressed* here
 * is the re-applied freeze §30.2 rules against.
 */
export function tabsFor(detail: ProjectDetail): readonly ProjectTabEntry[] {
  const tabs = [...BASE_PROJECT_TABS];
  if (detail.remote !== null) tabs.push(REMOTE);
  if (detail.health.state === 'frozen' || detail.health.state === 'live') tabs.push(HEALTH);
  return tabs;
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
