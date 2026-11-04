/**
 * §43's copy: every string the motivation layer renders, in one module, and no other file spells
 * one. The badge's own name and sentence are §40's (`../badgeAward/copy.ts`), read, never
 * restated here.
 *
 * Every string is interim: its constraint text, until the user writes the words. A clause naming a
 * word the constraint bans is left out of the rendered string — §39.10a keeps the banned word out
 * of every rendered string and accessible name — and each export's comment keeps the constraint
 * whole. A formatter renders the constraint, then the figures it is handed; an unknown figure is
 * §7.7a's U+2014, never a zero.
 */
import type { MotivationReason, RhythmDay, RingStatus, XpTerm } from '../../generated/protocol';
import { quietNote } from '../../shared/motivationQuiet';

/** §7.7a's glyph for an unknown figure. */
export const UNKNOWN = '—';

const figure = (value: number | null): string => (value === null ? UNKNOWN : String(value));

/**
 * §43.2, the level block's XP line while the level is `partial`. Constraint in full: the XP figure
 * of `computed`, then a qualifier naming the `pendingProjects` still being read and saying the
 * level can only grow; never presents the number as final.
 */
// ‹COPY› interim — the user's words replace this before v1.0.0
export const XP_LINE_PARTIAL = (remaining: number, pendingProjects: number): string =>
  `‹COPY: the XP figure of computed, then a qualifier naming the projects still being read and saying the level can only grow; never presents the number as final› ${String(remaining)} · ${String(pendingProjects)}`;

/**
 * §43.2, the level block's XP line while the level is `computed`. Constraint in full: one figure —
 * the XP remaining to the next level, `nextLevelAt − xp`; never a total, never a percentage.
 */
// ‹COPY› interim — the user's words replace this before v1.0.0
export const XP_LINE_COMPUTED = (remaining: number): string =>
  `‹COPY: one figure, the XP remaining to the next level; never a total, never a percentage› ${String(remaining)}`;

/**
 * §43.2, the level block's line while the floor holds the level. Constraint in full: says the
 * floor holds this level; names no XP figure.
 */
// ‹COPY› interim — the user's words replace this before v1.0.0
export const XP_LINE_FLOORED = '‹COPY: says the floor holds this level; names no XP figure›';

/**
 * §43.2 and §39.10a, a ring that does not apply. Constraint in full: one short word or mark
 * stating the ring does not apply; never a zero, never an empty ring.
 */
// ‹COPY› interim — the user's words replace this before v1.0.0
export const RING_NOT_APPLICABLE = '‹COPY: one short word or mark stating the ring does not apply›';

/**
 * §43.2 and §39.10a, the Tend ring's `nothingToTend` sub-line. Constraint in full: §39's closed
 * state, never read as "nothing done"; says nothing is left to tend *of what is read*; must not
 * claim the library is clean; never celebrates — whether it names its exclusions is the user's
 * call. The two clauses quoting a banned reading are left out of the rendered string.
 */
// ‹COPY› interim — the user's words replace this before v1.0.0
export const TEND_NOTHING_TO_TEND =
  '‹COPY: says nothing is left to tend of what is read; never celebrates›';

/**
 * §43.2 and §39.10a, day one's one sentence in place of the three rings' sub-lines. Constraint in
 * full: one sentence; says the rings start tomorrow; never implies a miss; implies no failure and
 * names no absence.
 */
// ‹COPY› interim — the user's words replace this before v1.0.0
export const DAY_ONE_SENTENCE =
  '‹COPY: one sentence; says the rings start tomorrow; implies no failure and names no absence›';

/**
 * §43.2 and §39.10a, the `RHYTHM` line. Constraint in full: active days of target this week, and
 * best rhythm in weeks; the word is "rhythm", never "streak", in every rendered string and
 * accessible name. The clause naming the banned word is left out of the rendered string. A target
 * not yet set and a best rhythm not yet known render as U+2014.
 */
// ‹COPY› interim — the user's words replace this before v1.0.0
export const RHYTHM_LINE = (
  activeDays: number,
  target: number | null,
  bestWeeks: number | null,
): string =>
  `‹COPY: active days of target this week, and best rhythm in weeks› ${String(activeDays)} · ${figure(target)} · ${figure(bestWeeks)}`;

/**
 * The `RHYTHM` line's qualifier while `RhythmState.gitCoverage` is `partial`. Constraint in full:
 * qualifies the rhythm line while the history it counts is still being read.
 */
// ‹COPY› interim — the user's words replace this before v1.0.0
export const RHYTHM_PARTIAL =
  '‹COPY: qualifies the rhythm line while the history it counts is still being read›';

/**
 * §43.2, after `LVL <n>` in the condensed top-bar block while a line waits. Constraint in full: at
 * most two words; says a line waits in the band; never a count. The constraint text is longer than
 * the two words it allows, so the interim is its content in two.
 */
// ‹COPY› interim — the user's words replace this before v1.0.0
export const CONDENSED_LINE_WAITS = 'LINE WAITS';

/**
 * §43.4, the `arrival` line, drawn after its level. Constraint in full: the level, that it was
 * computed from history, the coverage qualifier while partial; whether it names where quiet mode
 * lives is the user's call.
 */
// ‹COPY› interim — the user's words replace this before v1.0.0
export const LINE_ARRIVAL =
  '‹COPY: the level, that it was computed from history, the coverage qualifier while partial›';

/**
 * §43.4, the `arrival` line's coverage qualifier while the level is `partial`, after the level.
 * Constraint in full: the coverage qualifier while partial — the projects still being read.
 */
// ‹COPY› interim — the user's words replace this before v1.0.0
export const LINE_COVERAGE_PARTIAL = (pendingProjects: number): string =>
  `‹COPY: the coverage qualifier while partial› ${String(pendingProjects)}`;

/**
 * §43.4, the `rise` line. Constraint in full: the level, then the reason — one transition named
 * from its receipt, several counted; no word after the number, no chips.
 */
// ‹COPY› interim — the user's words replace this before v1.0.0
export const LINE_RISE = (level: number | null, reason: MotivationReason | null): string => {
  const head = `‹COPY: the level, then the reason — one transition named from its receipt, several counted› ${figure(level)}`;
  if (reason === null) return head;
  if (reason.transitions > 1) return `${head} · ${String(reason.transitions)}`;
  const receipt = [reason.projectName, reason.on].filter((part) => part !== null);
  return receipt.length === 0 ? head : `${head} · ${receipt.join(' · ')}`;
};

/**
 * §43.4, the `return` line, drawn after its level. Constraint in full: the level alone; no
 * reason, no "welcome back". The clause quoting the banned phrase is left out.
 */
// ‹COPY› interim — the user's words replace this before v1.0.0
export const LINE_RETURN = '‹COPY: the level alone; no reason›';

/** §43.11, the drawer group's title. Constraint in full: a group of its own after `MOTION`. */
// ‹COPY› interim — the user's words replace this before v1.0.0
export const QUIET_GROUP_TITLE = '‹COPY: the title of a group of its own after MOTION›';

/**
 * §43.11, the quiet switch's label. Constraint in full: one real switch, checked meaning the
 * motivation layer is shown; quiet is the switch off.
 */
// ‹COPY› interim — the user's words replace this before v1.0.0
export const QUIET_SWITCH_LABEL =
  '‹COPY: the switch’s label; checked means the motivation layer is shown›';

/**
 * §43.11, the quiet switch's note. Constraint in full: the note names the list, read from the list
 * module, never restated — so it is the list, and nothing written here.
 */
export const QUIET_SWITCH_NOTE = quietNote();

/** §43.11's Focus target row. Constraint in full: Focus minutes. The range is the stepper's. */
// ‹COPY› interim — the user's words replace this before v1.0.0
export const FOCUS_TARGET_LABEL = '‹COPY: Focus minutes›';

/** §43.11's rhythm target row. Constraint in full: rhythm days. The range is the stepper's. */
// ‹COPY› interim — the user's words replace this before v1.0.0
export const RHYTHM_TARGET_LABEL = '‹COPY: rhythm days›';

/**
 * §43.7, the reveal's `LEVEL` panel caption. Constraint in full: true of the ledger it describes —
 * never "you did not start at zero" over a ledger that holds nothing. The clause quoting the
 * banned sentence is left out.
 */
// ‹COPY› interim — the user's words replace this before v1.0.0
export const LEVEL_PANEL_CAPTION = '‹COPY: true of the ledger it describes›';

/**
 * §43.13, the evidence's line when `floored`. Constraint in full: says the floor holds the number.
 */
// ‹COPY› interim — the user's words replace this before v1.0.0
export const FLOOR_HOLDS_LINE = '‹COPY: says the floor holds the number›';

/**
 * §43.10, `DIG`'s inert note once suggestions have answered and no root would be committed.
 * Constraint in full: one line; says a root must be ticked or added; the brief interval before
 * suggestions answer needs no note.
 */
// ‹COPY› interim — the user's words replace this before v1.0.0
export const DIG_NO_ROOT_NOTE = '‹COPY: one line; says a root must be ticked or added›';

/**
 * §43.12, the first-ask identity card's level clause. Constraint in full: one clause; on a set
 * never confirmed before, says the level is recomputed from the addresses kept; on the re-opened
 * card, says the level cannot fall; says badges stay; states no figure. No surface re-opens the
 * card with this clause yet, so only the first-ask form exists.
 */
// ‹COPY› interim — the user's words replace this before v1.0.0
export const IDENTITY_LEVEL_CLAUSE =
  '‹COPY: one clause; says the level is recomputed from the addresses kept, and that badges stay›';

/*
 * §43.2's labels and the words each ring and cell's accessible name carries. These are the
 * prototype's labels and §39's state names, not ‹COPY› slots: no constraint stands in for them.
 */

/** §43.2's label under the level number — the only thing naming the figure (§10.4a). */
export const LEVEL_LABEL = 'LEVEL';

/** §7.7a's label, as the level block's line while nothing has computed the level. */
export const LEVEL_NOT_COMPUTED = 'NOT COMPUTED';

/** §39.1's three rings, in §39's order. */
export const RING_LABELS = { focus: 'FOCUS', ship: 'SHIP', tend: 'TEND' } as const;

/** §43.2's label over the week's seven cells. */
export const RHYTHM_LABEL = 'RHYTHM';

/** §43.13's evidence: each ledger term by §38.3's own name for its source. */
export const XP_TERM_WORDS: Readonly<Record<XpTerm, string>> = {
  commitDay: 'COMMIT-DAYS',
  revival: 'DEAD PROJECTS REVIVED',
  release: 'RELEASES TAGGED',
  language: 'NEW LANGUAGES',
  focus: 'FOCUSED SESSION TIME',
  debtDay: 'HEALTH ITEMS FIXED',
  completionCheck: 'COMPLETION CHECKS',
  amnestyVerdict: 'AMNESTY VERDICTS',
  cleanArchive: 'ARCHIVED CLEANLY',
};

/** §43.13's evidence row that adds the terms up, so the sum is read beside the level's XP. */
export const TERMS_SUM_LABEL = 'SUM';

/** §43.13's second section: every latched badge. */
export const BADGES_LABEL = 'BADGES';

/** Profile's way back, the project page's own word for the surface it returns to. */
export const PROFILE_BACK_LABEL = 'SHELF';

/** A binary ring's centre mark while §39's `nothingToTend` closes it — a mark, not a count. */
export const RING_CLOSED_MARK = '✓';

/**
 * Each ring status in words, for the ring's accessible name. `nothingToTend` is §39's closed
 * state, and its own sub-line says the rest; `notComputed` is never a zero.
 */
export const RING_STATUS_WORDS: Readonly<Record<RingStatus, string>> = {
  notComputed: 'not computed',
  notApplicable: 'does not apply',
  open: 'open',
  closed: 'closed',
  nothingToTend: 'closed',
};

/** Each rhythm cell's state in words, for its accessible name. */
export const RHYTHM_DAY_WORDS: Readonly<Record<RhythmDay, string>> = {
  active: 'active',
  inactive: 'not active',
  pending: 'today, not yet active',
  future: 'still to come',
  waived: 'waived',
  notComputed: 'not computed',
};

/** The week's cells, Monday first (§39.2: `weekStart` is the Monday). */
export const DAY_LETTERS: readonly string[] = ['M', 'T', 'W', 'T', 'F', 'S', 'S'];

/** The same seven days in full, for each cell's accessible name. */
export const DAY_NAMES: readonly string[] = [
  'Monday',
  'Tuesday',
  'Wednesday',
  'Thursday',
  'Friday',
  'Saturday',
  'Sunday',
];
