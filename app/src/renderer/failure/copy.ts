/**
 * §11.2a's four full-screen windows, in strings.
 *
 * The fifth failure — core spawn — is **not here**: it is §8.0's notice slot at priority 1, its
 * sentences belong to the main process, which is the only side that can stat the binary to tell
 * the five causes apart, and `notices/copy.ts` adapts them rather than restating the box.
 *
 * Every number these windows print is the point of the window. "Please update" without both
 * schema versions is unactionable; "an error occurred" without the schema it was restored to
 * and when is the defect §11.2a exists to correct. Where a number is genuinely unknown — a null
 * gap start, a ledger the report could not count — the block **says so in words and prints no
 * figure**, which is "never render unknown as zero" on the screen where it costs the most.
 */
import type { SidecarReport, StartupFailure } from '../../shared/startupFailure';

export interface StillShuttingDown {
  readonly kind: 'still_shutting_down';
  readonly startedAtMs: number;
}

export type FailureFact = StartupFailure | StillShuttingDown;
export type FailureKind = FailureFact['kind'];

export interface FailureCopy {
  readonly eyebrow: string;
  readonly headline: string;
  readonly body: readonly string[];
  readonly primary: string;
  readonly secondary: string | null;
}

export const FORCE_AVAILABLE_AFTER_MS = 10_000;

/**
 * A forensic timestamp, scoped to these four windows. The product's time idiom is `formatAge`
 * and `formatClock` — an age and a wall clock. A restore or a quarantine is read once, possibly
 * days later, and needs its date, which neither of those carries.
 */
export function failureTimestamp(epochSecs: number): string {
  return new Date(epochSecs * 1000).toLocaleString('en-US', {
    dateStyle: 'medium',
    timeStyle: 'short',
  });
}

/**
 * §48.7.2: whether the window's primary is REBUILD. A sidecar a newer build wrote refuses the
 * rebuild before anything moves, so that window offers QUIT alone.
 */
export function offersRebuild(fact: FailureFact): boolean {
  return fact.kind === 'corrupt_index' && fact.sidecar.state !== 'newer';
}

/**
 * §48.7.2: what REBUILD will do, by the sidecar beside the index. With none to read, absent or
 * unreadable alike, the rebuild restores nothing and first run starts over.
 */
// ‹COPY› interim — the user's words replace this before v1.0.0
const REBUILD_WILL: Readonly<Record<SidecarReport['state'], string>> = {
  present: '‹COPY: says what REBUILD will do›',
  absent: '‹COPY: says REBUILD restores nothing and first run starts over›',
  unreadable: '‹COPY: says REBUILD restores nothing and first run starts over›',
  newer: '‹COPY: says a newer build wrote the sidecar, so this build will not rebuild›',
};

export function failureCopy(fact: FailureFact): FailureCopy {
  switch (fact.kind) {
    case 'schema_from_future':
      return {
        eyebrow: 'INDEX VERSION',
        headline: 'THIS LIBRARY WAS WRITTEN BY A NEWER CODOTHECA',
        body: [
          `The index on disk is at schema ${String(fact.onDisk)}. This build speaks schema ` +
            `${String(fact.supported)}.`,
          'Opening it would rewrite rows this build does not understand, so it will not open ' +
            'it. Install the newer version and your library comes back untouched.',
        ],
        primary: 'QUIT',
        secondary: null,
      };
    case 'migration_failed':
      return {
        eyebrow: 'UPGRADE',
        headline: 'AN UPGRADE STOPPED AND THE BACKUP WAS PUT BACK',
        body: [
          `The step to schema ${String(fact.version)} did not finish. Your index was restored ` +
            `to schema ${String(fact.restoredTo)} at ${failureTimestamp(fact.restoredAt)}.`,
          'Nothing was written past the restore point. The log below has the step that failed.',
        ],
        primary: 'QUIT',
        secondary: null,
      };
    case 'corrupt_index': {
      const rebuild = offersRebuild(fact);
      return {
        eyebrow: 'INDEX',
        headline: 'THE INDEX WOULD NOT OPEN',
        // §48.7.2: nothing has been moved before REBUILD, so no sentence may say it was.
        body: [
          // ‹COPY› interim — the user's words replace this before v1.0.0
          ...(fact.rebuildFailed === null
            ? []
            : [`‹COPY: names the failure› ${fact.rebuildFailed}`]),
          REBUILD_WILL[fact.sidecar.state],
          'What comes back and what does not is below, in three parts.',
        ],
        primary: rebuild ? 'REBUILD' : 'QUIT',
        secondary: rebuild ? 'QUIT' : null,
      };
    }
    case 'still_shutting_down':
      return {
        eyebrow: 'ANOTHER INSTANCE',
        headline: 'CODOTHECA IS STILL SHUTTING DOWN',
        body: [
          'A previous window is closing and still holds the index. It normally takes a moment.',
          'Forcing it ends that shutdown mid-write. Wait unless it has plainly stopped.',
        ],
        primary: 'WAIT',
        secondary: 'FORCE',
      };
    default: {
      const unhandled: never = fact;
      return unhandled;
    }
  }
}

export interface LedgerBlock {
  readonly label: string;
  readonly lines: readonly string[];
}

// ‹COPY› interim — the user's words replace this before v1.0.0
function interimNoun(key: string): readonly [string, string] {
  return [`‹COPY: ${key}›`, `‹COPY: ${key}›`];
}

/**
 * Every count the sidecar carries, keyed as the core keys it, with the singular and plural the
 * window prints: one per `BASE_COUNT_KEYS` entry and one per section in
 * `core/src/index/sidecar.rs`, which `app/test/sidecarNouns.test.ts` reads so a key added there
 * cannot go unnamed here. The first seven are §11.2a's enumeration, in its order.
 */
export const SIDECAR_COUNT_NOUNS: Readonly<Record<string, readonly [string, string]>> = {
  projects: ['project', 'projects'],
  notes: ['note', 'notes'],
  sessions: ['session', 'sessions'],
  collections: ['collection', 'collections'],
  roots: ['scan root', 'scan roots'],
  launch_targets: ['launch target', 'launch targets'],
  xp_events: ['XP event', 'XP events'],
  session_segments: interimNoun('session_segments'),
  collection_members: interimNoun('collection_members'),
  identities: interimNoun('identities'),
  merges: interimNoun('merges'),
  settings: interimNoun('settings'),
  view_state: interimNoun('view_state'),
  pending: interimNoun('pending'),
  no_scan_projects: interimNoun('no_scan_projects'),
};

/** §48.8.3: merges ride the sidecar and are never replayed, so a rebuild restores none. */
const NOT_RESTORED: ReadonlySet<string> = new Set(['merges']);

/**
 * `null` is *the report could not count this*, and `0` is *counted, and there were none*. They
 * are different sentences here, and neither is a zeroed noun: `0 projects` in this ledger reads
 * as a measurement of what was lost. A count the sidecar does not carry is no line at all.
 */
function ledgerLines(
  counts: Readonly<Record<string, number>> | null,
  subject: string,
): readonly string[] {
  if (counts === null) {
    return [`How much ${subject} is not known: the index did not open far enough to count it.`];
  }
  const lines = Object.entries(SIDECAR_COUNT_NOUNS)
    .filter(([key]) => !NOT_RESTORED.has(key))
    .flatMap(([key, [one, many]]) => {
      const n = counts[key] ?? 0;
      return n > 0 ? [`${String(n)} ${n === 1 ? one : many}`] : [];
    });
  return lines.length > 0 ? lines : ['Nothing.'];
}

/**
 * The three blocks. The third is the one §1.12 implies and never states: the sidecar is written
 * at shutdown and hourly, so everything decided since the last write is gone.
 *
 * `StartupFailure` carries no counts for that block, only whether they could have been known.
 * The block therefore names what a gap contains and never invents a figure for it.
 */
export function corruptLedger(
  fact: Extract<StartupFailure, { kind: 'corrupt_index' }>,
): readonly LedgerBlock[] {
  // The gap opens where the sidecar was last written; before a rebuild it is never counted.
  const writtenAt = fact.sidecar.writtenAt;
  const when =
    writtenAt === null
      ? 'The start of the gap is not known.'
      : `The gap starts at ${failureTimestamp(writtenAt)}.`;
  return [
    {
      // §48.7.2: unknown until the scan runs, so named and never counted.
      label: 'RE-DERIVED FROM DISK',
      lines: ledgerLines(null, 'can be read back from your folders'),
    },
    {
      // §48.7.2: what a rebuild will restore. Only a sidecar this build can read restores any.
      label: 'RESTORED FROM THE SIDECAR',
      lines:
        fact.sidecar.state === 'present'
          ? ledgerLines(fact.sidecar.counts, 'the sidecar still holds')
          : ['Nothing.'],
    },
    {
      label: 'LOST IN THE GAP',
      lines: [
        when,
        'Everything decided after that point is gone, and it cannot be counted.',
        'Notes, flags, collections, sessions, XP events, scan roots, consent, identity ' +
          'confirmations, custom launch targets, settings, view state, art rerolls and merge ' +
          'decisions all live there.',
      ],
    },
  ];
}

/** §11.2a: real elapsed seconds in the mono note, never a spinner. */
export function shuttingDownNote(startedAtMs: number, nowMs: number): string {
  const seconds = Math.max(0, Math.floor((nowMs - startedAtMs) / 1000));
  return `still shutting down · ${String(seconds)}s`;
}

export function forceIsOffered(startedAtMs: number, nowMs: number): boolean {
  return nowMs - startedAtMs >= FORCE_AVAILABLE_AFTER_MS;
}

/** §11.2a, §11.4: the path the user is being asked to open. Decision-carrying, so `--text-3`. */
export function logPathNote(logPath: string): string {
  return `LOG · ${logPath}`;
}
