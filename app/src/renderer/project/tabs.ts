/**
 * §8.5: two tabs ship. Health needs completion scoring and nothing in phase 1 writes
 * `completion_lit`; Remote needs a token and §0 puts remotes out. Both are **absent, not
 * disabled** — a greyed tab is exactly the dead control the standing rule exists to stop.
 *
 * §11.7: tabs cycle over the *mounted* list. The prototype's modulo-4 ring names two tabs that
 * do not exist here.
 */
export type ProjectTab = 'overview' | 'activity';

export const PROJECT_TABS = [
  { id: 'overview', label: 'OVERVIEW' },
  { id: 'activity', label: 'ACTIVITY' },
] as const satisfies readonly { id: ProjectTab; label: string }[];

export function nextTab(current: ProjectTab, delta: 1 | -1): ProjectTab {
  const index = PROJECT_TABS.findIndex((tab) => tab.id === current);
  const count = PROJECT_TABS.length;
  const moved = (index + delta + count) % count;
  return PROJECT_TABS[moved]?.id ?? 'overview';
}
