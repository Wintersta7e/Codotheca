/**
 * §43's copy: every string the motivation layer renders, in one module, and no other file spells
 * one. The badge's own name and sentence are §40's (`../badgeAward/copy.ts`), read, never
 * restated here.
 *
 * Each export's comment keeps the constraint its words answer, whole. A word the constraint bans
 * stays out of every rendered string and accessible name (§39.10a). A formatter renders the figures
 * it is handed; an unknown figure is §7.7a's U+2014, never a zero.
 */
import type { MotivationReason, RhythmDay, RingStatus, XpTerm } from '../../generated/protocol';
import { quietNote } from '../../shared/motivationQuiet';

/** §7.7a's glyph for an unknown figure. */
export const UNKNOWN = '—';

const figure = (value: number | null): string => (value === null ? UNKNOWN : String(value));

/** A counted noun: singular at exactly one, plural otherwise (an unknown count reads plural). */
const counted = (value: number | null, one: string, many: string): string =>
  `${figure(value)} ${value === 1 ? one : many}`;

/** One phrase for one fact: the projects whose history is still being read. */
const stillIndexing = (pendingProjects: number): string =>
  `${counted(pendingProjects, 'PROJECT', 'PROJECTS')} STILL INDEXING`;

/**
 * §43.2, the level block's XP line while the level is `computed`. Constraint in full: one figure —
 * the XP remaining to the next level, `nextLevelAt − xp`; never a total, never a percentage.
 */
export const XP_LINE_COMPUTED = (remaining: number): string =>
  `${String(remaining)} XP TO NEXT LEVEL`;

/**
 * §43.2, the level block's XP line while the level is `partial`. Constraint in full: the XP figure
 * of `computed`, then a qualifier naming the `pendingProjects` still being read and saying the
 * level can only grow; never presents the number as final. "Can only grow" sits last, so it
 * qualifies the reading — the level, which the floor holds — and not the remaining XP.
 */
export const XP_LINE_PARTIAL = (remaining: number, pendingProjects: number): string =>
  `${XP_LINE_COMPUTED(remaining)} · ${stillIndexing(pendingProjects)} · CAN ONLY GROW`;

/**
 * §43.2, the level block's line while the floor holds the level. Constraint in full: says the
 * floor holds this level; names no XP figure.
 */
export const XP_LINE_FLOORED = 'HELD AT THIS LEVEL';

/**
 * §43.2 and §39.10a, a ring that does not apply. Constraint in full: one short word or mark
 * stating the ring does not apply; never a zero, never an empty ring.
 */
export const RING_NOT_APPLICABLE = 'N/A';

/**
 * §43.2 and §39.10a, the Tend ring's `nothingToTend` sub-line. Constraint in full: §39's closed
 * state, never read as "nothing done"; says nothing is left to tend *of what is read*; must not
 * claim the library is clean; never celebrates — whether it names its exclusions is the user's
 * call. The words name no exclusions.
 */
export const TEND_NOTHING_TO_TEND = 'NOTHING TO TEND IN WHAT IS READ';

/**
 * §39.10a, the Ship ring's `open` sub-line. Constraint in full: no push recorded this week on this
 * machine; never "you did not push"; no verb aimed at the user. It states what is recorded, not
 * what the user did.
 */
export const SHIP_OPEN_SUB = 'NO PUSH RECORDED THIS WEEK';

/**
 * §43.2 and §39.10a, day one's one sentence in place of the three rings' sub-lines. Constraint in
 * full: one sentence; says the rings start tomorrow; never implies a miss; implies no failure and
 * names no absence.
 */
export const DAY_ONE_SENTENCE = 'RINGS START TOMORROW.';

/**
 * §43.2 and §39.10a, the `RHYTHM` line. Constraint in full: active days of target this week, and
 * best rhythm in weeks; the word is "rhythm", never "streak", in every rendered string and
 * accessible name. An active-day count over a week not yet read, a target not yet set and a best rhythm not yet known
 * render as U+2014.
 */
export const RHYTHM_LINE = (
  activeDays: number | null,
  target: number | null,
  bestWeeks: number | null,
): string =>
  `${figure(activeDays)} OF ${figure(target)} DAYS · BEST ${counted(bestWeeks, 'WEEK', 'WEEKS')}`;

/**
 * The `RHYTHM` line's qualifier while `RhythmState.gitCoverage` is `partial`. Constraint in full:
 * qualifies the rhythm line while the history it counts is still being read.
 */
export const RHYTHM_PARTIAL = 'HISTORY STILL INDEXING';

/**
 * §43.2, after `LVL <n>` in the condensed top-bar block while a line waits. Constraint in full: at
 * most two words; says a line waits in the band; never a count.
 */
export const CONDENSED_LINE_WAITS = 'LINE WAITS';

/**
 * §43.4, the `arrival` line, drawn after its level. Constraint in full: the level, that it was
 * computed from history, the coverage qualifier while partial; whether it names where quiet mode
 * lives is the user's call — it names none. `line.ts` draws the level after these words.
 */
export const LINE_ARRIVAL = 'COMPUTED FROM YOUR HISTORY · LEVEL';

/**
 * §43.4, the `arrival` line's coverage qualifier while the level is `partial`, after the level.
 * Constraint in full: the coverage qualifier while partial — the projects still being read.
 */
export const LINE_COVERAGE_PARTIAL = (pendingProjects: number): string =>
  stillIndexing(pendingProjects);

/**
 * One ledger term named in the `rise` line's receipt, singular — one row (§43.4). §38.3's source
 * names; `focus` never reaches the line (its row carries no reason), but the table is total so a
 * new term fails the type check.
 */
export const RISE_TERM_WORDS: Readonly<Record<XpTerm, string>> = {
  commitDay: 'COMMIT-DAY',
  revival: 'PROJECT REVIVED',
  release: 'RELEASE TAGGED',
  language: 'NEW LANGUAGE',
  focus: 'FOCUS',
  debtDay: 'HEALTH ITEM FIXED',
  completionCheck: 'COMPLETION CHECK',
  amnestyVerdict: 'AMNESTY VERDICT',
  cleanArchive: 'ARCHIVED CLEANLY',
};

/**
 * §43.4, the `rise` line. Constraint in full: the level, then the reason — one transition named
 * from its receipt, several counted; no word after the number, no chips. One transition names
 * its term, its project and its local date, a part the receipt left null dropped.
 */
export const LINE_RISE = (level: number | null, reason: MotivationReason | null): string => {
  const head = `LEVEL ${figure(level)}`;
  if (reason === null) return head;
  if (reason.transitions > 1) return `${head} · FROM ${String(reason.transitions)} NEW ENTRIES`;
  const term = reason.term === null ? null : RISE_TERM_WORDS[reason.term];
  const receipt = [term, reason.projectName, reason.on].filter((part) => part !== null);
  return receipt.length === 0 ? head : `${head} · ${receipt.join(' · ')}`;
};

/**
 * §43.4, the `return` line, drawn after its level. Constraint in full: the level alone; no
 * reason, no "welcome back". `line.ts` draws the level after this word.
 */
export const LINE_RETURN = 'LEVEL';

/** §43.11, the drawer group's title. Constraint in full: a group of its own after `MOTION`. */
export const QUIET_GROUP_TITLE = 'MOTIVATION';

/**
 * §43.11, the quiet switch's label. Constraint in full: one real switch, checked meaning the
 * motivation layer is shown; quiet is the switch off.
 */
export const QUIET_SWITCH_LABEL = 'Show level, XP and rings';

/**
 * §43.11, the quiet switch's note. Constraint in full: the note names the list, read from the list
 * module, never restated — so it is the list, and nothing written here.
 */
export const QUIET_SWITCH_NOTE = quietNote();

/** §43.11's Focus target row. Constraint in full: Focus minutes. The range is the stepper's. */
export const FOCUS_TARGET_LABEL = 'Focus target, minutes a day';

/** §43.11's rhythm target row. Constraint in full: rhythm days. The range is the stepper's. */
export const RHYTHM_TARGET_LABEL = 'Rhythm target, days a week';

/**
 * §43.7, the reveal's `LEVEL` panel caption. Constraint in full: true of the ledger it describes —
 * never "you did not start at zero" over a ledger that holds nothing. It states the method, not a
 * result, so it holds over an empty ledger and one not yet computed.
 */
export const LEVEL_PANEL_CAPTION = 'Counted back from your history, not from today.';

/**
 * §43.13, the evidence's line when `floored`. Constraint in full: says the floor holds the number.
 */
export const FLOOR_HOLDS_LINE = `${XP_LINE_FLOORED} · THE SUM IS BELOW IT`;

/**
 * §43.10, `DIG`'s inert note once suggestions have answered and no root would be committed.
 * Constraint in full: one line; says a root must be ticked or added; the brief interval before
 * suggestions answer needs no note.
 */
export const DIG_NO_ROOT_NOTE = 'NOTHING TO INDEX UNTIL A FOLDER IS TICKED OR ADDED.';

/**
 * §43.12, the first-ask identity card's level clause. Constraint in full: one clause; on a set
 * never confirmed before, says the level is recomputed from the addresses kept; on the re-opened
 * card, says the level cannot fall; says badges stay; states no figure. No surface re-opens the
 * card with this clause yet, so only the first-ask form exists.
 */
export const IDENTITY_LEVEL_CLAUSE =
  'Your level is recomputed from the addresses you keep, and badges stay.';

/*
 * §43.2's labels and the words each ring and cell's accessible name carries. These are the
 * prototype's labels and §39's state names: no §43 constraint governs their words.
 */

/** §43.2's label under the level number — the only thing naming the figure (§10.4a). */
export const LEVEL_LABEL = 'LEVEL';

/** §43.7's eighth reveal panel, the prototype's label for the badge §40 backdates. */
export const BADGE_EARNED_LABEL = 'BADGE EARNED';

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

/** §43.2's condensed top-bar block: the prototype's label before the level. */
export const CONDENSED_LEVEL_LABEL = 'LVL';

/**
 * §43.2's short rhythm in the condensed block, the prototype's form: active days over the target,
 * then the best rhythm. §39.7 counts the best rhythm in weeks, so its unit is `w`. An active-day
 * count over a week not yet read, a target not yet set and a best rhythm not yet known render as
 * U+2014.
 */
export const RHYTHM_SHORT = (
  activeDays: number | null,
  target: number | null,
  bestWeeks: number | null,
): string => `${figure(activeDays)}/${figure(target)} · ${figure(bestWeeks)}w`;

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
