/**
 * §43.11's closed list: what quiet mode (`Settings.motivationShown = false`) hides — a game about
 * the work — in **one** place the drawer, the renderer gate and the shell all read.
 *
 * Everything not listed is a fact about the work and is kept: the retroactive stats, playtime, the
 * Activity chart, completion and health, quests, the Amnesty invitation, the notifications. The
 * core strips its own payload at the source and needs no copy of this list.
 *
 * It lives under `shared/` for the reason `notificationCopy.ts` does: the shell cannot import a
 * `.tsx` file, and a list the shell restated would be a second list.
 */

/** One hidden row of §43.11's table. */
export type QuietEntryId =
  'level' | 'xp' | 'rings' | 'rhythm' | 'badges' | 'line' | 'identityLevelClause';

export interface QuietEntry {
  readonly id: QuietEntryId;
  /** The row as §43.11's table names it; the drawer's note is built from these and only these. */
  readonly label: string;
}

/** §43.11's hidden column, in its order. Each label is spelled here and nowhere else. */
export const QUIET_LIST: readonly QuietEntry[] = [
  { id: 'level', label: 'THE LEVEL' },
  { id: 'xp', label: 'XP IN ANY FORM' },
  { id: 'rings', label: 'RINGS IN ANY FORM' },
  { id: 'rhythm', label: 'RHYTHM WHEREVER DRAWN' },
  { id: 'badges', label: 'BADGES WHEREVER SHOWN' },
  { id: 'line', label: 'THE LINE AND THE FLIP' },
  { id: 'identityLevelClause', label: "THE IDENTITY CARD'S LEVEL CLAUSE" },
];

/**
 * The attribute every element the list covers carries, valued with its entry's id, so a count
 * of `[data-motivation]` on a surface is the count of what quiet mode must remove there.
 */
export const DATA_MOTIVATION = 'data-motivation';

/** The quiet switch's note: the list itself, read from the entries, never restated. */
export function quietNote(): string {
  return QUIET_LIST.map((entry) => entry.label).join(' · ');
}
