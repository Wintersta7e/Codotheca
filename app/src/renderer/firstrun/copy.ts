/**
 * Every string the four first-run screens and the two shelf cards render.
 *
 * They live together because §10.1's paragraph is the one claim the product invites the user to
 * check against their own disk, and v1 shipped a paragraph its own code contradicted. One file
 * makes the claim diffable in a single read.
 */

import { LOCKFILE_CAP_TEXT, LOCKFILE_DEPTH_TEXT } from '../../shared/lockfileNames';

/**
 * §10.1 verbatim. Any change here is a change to what the code is allowed to read.
 *
 * **[p3] The lock files and their depth are named, and the paragraph moves in the same change
 * that lands the read.** A11.3's *"rides the existing grant"* is true only **after** this
 * sentence moves: the shipped text promised a named set *at the repository root*, and a lock file
 * is neither a manifest nor root-only — §32.6's set sits at depth ≤ 3, so the grant as *written*
 * did not cover it and a section citing A11.3 without moving the copy would ship a read the
 * consent screen denies.
 *
 * **The per-file cap differs and the copy must not claim one number for both.** J6's 256 KB is
 * unchanged for J6's own named-file reads; the lock file read carries its own 16 MB, because
 * 256 KB is refuted by a measurement on this very repository — its own `package-lock.json` is
 * 287,417 bytes.
 *
 * **The *"does not read the text of your source files"* sentence is unchanged, because it is
 * still true**: a lock file is machine-generated dependency bookkeeping, not source text, and it
 * is read by name rather than by walking a tree.
 */
export const CONSENT_PARAGRAPH =
  "Codotheca reads your repositories' git metadata, the names and timestamps of files in your " +
  'working trees, and a small named set of files — README, LICENSE, and package manifests at the ' +
  'repository root, up to 256 KB each. It also reads your lock files — package-lock.json, ' +
  'yarn.lock, pnpm-lock.yaml, Cargo.lock, poetry.lock and uv.lock — up to three directories deep ' +
  'and up to 16 MB each, to check your dependencies against published advisories. It does not ' +
  'read the text of your source files unless you turn that on in settings, and it is off until ' +
  'you do. Nothing is uploaded. There is no account.';

export const ROOTS_EYEBROW = 'CODOTHECA';
export const ROOTS_HEADLINE = "LET'S SEE WHAT YOU HAVE WRITTEN";
export const ROOTS_STANDFIRST =
  'These are the places your git config and your editors already point at. Nothing is read ' +
  'until you say so, and nothing leaves this machine.';

/**
 * §10.1a. This sentence does more trust work than any privacy paragraph, because it says
 * *I read two small named files* rather than *I will read your disk*.
 */
export const ROOTS_LIST_CAPTION = "from your editor's recent projects and .gitconfig";

export const EXCLUSION_HEADING = 'NEVER LOOKED AT';
export const CONSENT_HEADING = 'CONSENT';
export const WHAT_EXACTLY_LABEL = 'WHAT EXACTLY';

export interface ConsentRow {
  /** A statement row trades the tick for a dot and takes no hover and no pointer (§10.1b). */
  readonly kind: 'control' | 'statement';
  readonly body: string;
  readonly note: string;
}

/**
 * §10.1b's three rows, one of them a control. The design's own rule decides the split: every
 * switch must do something observable or be presented as a statement rather than a control.
 */
export const CONSENT_ROWS: readonly ConsentRow[] = [
  {
    // [p3] §32.6: the lock file read rides this row's grant, so the row names it. Four root files
    // alone would under-describe a read that goes three directories deep.
    kind: 'control',
    body:
      'Index what is already public inside a repo — names, dates, branches, remotes — and four ' +
      `named files at its root, 256 KB each, and its lock files ${LOCKFILE_DEPTH_TEXT}, ` +
      `${LOCKFILE_CAP_TEXT}.`,
    note: 'READ FROM .git, FOUR ROOT FILES AND LOCK FILES · NEEDED NOW',
  },
  {
    // §29.8: it stays a **statement** here. The ask is in context, so a checkbox on this screen
    // would still store a preference nothing on this screen reads.
    kind: 'statement',
    body: 'Read the text of committed source files, to count TODOs and find debt items.',
    note: 'OFF UNTIL YOU TURN IT ON · SETTINGS · SCANNING',
  },
  {
    kind: 'statement',
    body: 'A ledger of commit-days, releases and revivals is written from the first scan.',
    note: 'RECORDED FROM DAY ONE · NOTHING RENDERS IT YET',
  },
];

export const DIG_LABEL = 'DIG';
export const DIG_NOTE = 'The scan keeps running after you leave this screen.';
export const DIG_INERT_NOTE = 'NOTHING TO INDEX WITHOUT THIS.';
export const ADD_A_FOLDER_LABEL = 'ADD A FOLDER';

/** §10.1b's refusal reasons, in the row's mono register. */
export const REFUSAL_REASONS = {
  filesystem_root: 'A DRIVE ROOT IS NOT A PROJECT FOLDER',
  home_without_narrowing: 'PICK A FOLDER INSIDE YOUR HOME, NOT ALL OF IT',
  already_a_root: 'ALREADY BEING LOOKED IN',
  too_many_directories: 'THAT IS A VERY LARGE FOLDER · ALMOST ALWAYS A MIS-PICK',
} as const;

export const CONFIRM_LARGE_LABEL = 'LOOK THERE ANYWAY';
export const CONFIRM_LARGE_BODY = (dirs: number): string =>
  `About ${dirs.toLocaleString()} folders sit under that one. A root that large is nearly ` +
  'always a mis-pick — it takes about five seconds to walk, so the time is not the problem.';

// §10.3a's escape control, present in the first rendered frame. **R12:** `SKIP_AHEAD_LABEL` is
// declared once, by plan 12b in `app/src/renderer/a11y/names.ts` — it is the visible label *and*
// the accessible name, so it cannot live in two files. This file does not redeclare it and does
// not re-export it: the screens import it from `../a11y/names`.
export const FOUND_LABEL = 'FOUND';
export const ROOTS_SUFFIX = 'ROOTS';

/** §10.3a: mandatory at every coverage, never suppressed. */
export const SO_FAR_QUALIFIER = 'SO FAR';

export const REVEAL_EYEBROW = (projects: number): string => `THE DIG · ${projects} PROJECTS`;
export const GO_ON_LABEL = 'GO ON';
export const EVIDENCE_FOOTER = 'EVERY FIGURE OPENS ITS EVIDENCE';
export const SHOW_WORKING_LABEL = 'SHOW WORKING';
export const CLOSE_LABEL = 'CLOSE';
export const EVIDENCE_HEADING = 'EVIDENCE';

/**
 * §10.4a. Span is the one figure monotone under increasing coverage — more history can only
 * push the first commit earlier — so `AT LEAST` is exactly honest and needs no chip.
 */
export function revealHeadline(spanYears: number | null, complete: boolean): string {
  if (spanYears === null) return 'HISTORY IS STILL ARRIVING';
  const unit = spanYears === 1 ? 'YEAR' : 'YEARS';
  return complete
    ? `YOU HAVE BEEN AT THIS FOR ${spanYears} ${unit}`
    : `YOU HAVE BEEN AT THIS FOR AT LEAST ${spanYears} ${unit}`;
}

export const COVERAGE_PARTIAL = (covered: number): string =>
  `across the ${covered} projects indexed so far`;
export const COVERAGE_HISTORY_GROWS = 'history still indexing — this can only grow';
export const COVERAGE_HISTORY_MOVES = 'history still indexing — this can move, not just grow';
export const UNCOMPUTED_NOTE = 'NOT COMPUTED YET · HISTORY HAS NOT RETURNED';

/** The unknown glyph. Distinct from `0`, which is a claim. */
export const UNKNOWN_GLYPH = '—';
/** A count that exists but has not been computed — an unscanned or offline root (§4.6). */
export const UNCOMPUTED_GLYPH = '?';

export const SHOW_ME_LABEL = 'SHOW ME';
export const NOT_NOW_LABEL = 'NOT NOW';

/** §10.4a's fetch qualifier, under the turn's line. */
export const FETCH_QUALIFIER =
  'AHEAD IS MEASURED AGAINST YOUR LAST FETCH · CODOTHECA NEVER FETCHES';
export const OBSERVED_QUALIFIER = (clock: string): string =>
  `AS OBSERVED AT ${clock} · WORKTREE STATE IS NEVER CACHED`;

/** §1.4's card. */
export const IDENTITY_TITLE = 'WHAT COUNTS AS YOURS';
export const IDENTITY_BODY_1 =
  'Every figure you have just seen was computed from these addresses. Untick anything that is ' +
  'not you.';
export const IDENTITY_BODY_2 =
  'Nothing is deleted. Anything that stops being yours moves below the grid and stops counting ' +
  'toward your figures.';
export const IDENTITY_CONFIRM_LABEL = 'CONFIRM';
export const IDENTITY_LEAVE_LABEL = 'LEAVE IT AS IT IS';
export const IDENTITY_FOOTNOTE = 'THIS IS ASKED ONCE · SETTINGS · IDENTITY';
export const IDENTITY_NO_COMMITS = 'NO COMMITS IN THIS LIBRARY';

/** §11.3a's ask, with the measured numbers. */
export const RESIDENCY_TITLE = 'KEEP IT READY';
export const RESIDENCY_BODY =
  'Codotheca can start with the system and keep its window alive for thirty minutes after you ' +
  'last use it, so the shortcut shows the shelf in 9 ms instead of 134.';
export const RESIDENCY_NOTE =
  '307 MB EMPTY · 522 MB WITH A FULL SHELF · 232 MB WITH THE WINDOW DESTROYED';
export const RESIDENCY_YES = 'START WITH THE SYSTEM';
export const RESIDENCY_NO = 'LEAVE IT OFF';
