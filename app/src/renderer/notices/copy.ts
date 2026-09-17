/**
 * The four notice bodies §11 sources for §8.0's single slot — priorities 1, 3, 4 and 5.
 *
 * This module says *what* a notice says. §8.0 owns where it goes, how big it is, which one of
 * the six wins, and how it is dismissed; none of that is restated here, which is what stops six
 * obligations becoming six surfaces again. `Notice`, `selectNotice` and the dismissal key stay
 * with the slot; this produces only the payload one carries.
 */
import type {
  DegradedReason,
  Problems,
  SyncNotice,
  TargetVerification,
} from '../../generated/protocol';
import { GIT_FLOOR } from '../../shared/gitFloor';

export interface NoticeCopy {
  readonly title: string;
  readonly body: string;
  readonly note: string | null;
  readonly primary: string | null;
  readonly secondary: string | null;
}

/**
 * Rendered from `shared/gitFloor.ts`, which mirrors `GIT_FLOOR` in `core/src/git/version.rs`
 * and is held equal to it by a test that reads the Rust source (R24). Writing `'2.22'` here
 * instead would be a third copy of the floor with nothing holding it to the other two, on the
 * one surface whose whole job is to tell the user which version to install.
 */
export const GIT_FLOOR_DISPLAY = GIT_FLOOR.join('.');

export function degradedNotice(
  reason: DegradedReason,
  gitVersion: string | null,
): NoticeCopy | null {
  switch (reason) {
    case 'git_missing':
      return {
        title: 'CODOTHECA CANNOT FIND GIT',
        body:
          'Every fact on this shelf is read from git, so nothing can be indexed until git ' +
          `${GIT_FLOOR_DISPLAY} or newer is on this machine.`,
        note: null,
        primary: 'RETRY',
        secondary: 'OPEN THE LOG',
      };
    case 'git_too_old':
      return {
        title: 'THIS GIT IS TOO OLD',
        body:
          gitVersion === null
            ? `Codotheca needs git ${GIT_FLOOR_DISPLAY} or newer. The version on this machine ` +
              'has not been read yet.'
            : `Codotheca needs git ${GIT_FLOOR_DISPLAY} or newer. This machine has git ` +
              `${gitVersion}.`,
        note: null,
        primary: 'RETRY',
        secondary: 'OPEN THE LOG',
      };
    case 'index_read_only':
      return {
        title: 'NOTHING IS BEING SAVED',
        body:
          'The index cannot be written, so notes, sessions and settings made now will be gone ' +
          'at the next launch. Everything on screen is still readable.',
        note: null,
        primary: 'RETRY',
        secondary: 'OPEN THE LOG',
      };
    // Per-project, and already surfaced twice: §11.1's summary groups and the error block. A
    // shelf-wide banner for a condition holding on four projects out of two hundred is the
    // notice stacking §8.0 exists to prevent.
    case 'store_offline':
    case 'budget_exceeded':
      return null;
    default: {
      const unhandled: never = reason;
      return unhandled;
    }
  }
}

/**
 * The facts the shell hands over for §11.2's five spawn causes — quarantined binary, `noexec`
 * mount, lost `+x`, wrong architecture, glibc below the baseline.
 *
 * The sentences are **not written here**, and the shape is structural on purpose: only the main
 * process can stat the binary to tell those causes apart, so it owns the wording and this side
 * reshapes it for the slot. A second set of sentences would drift from the one that has the
 * filesystem evidence behind it.
 */
export interface SpawnFailureFacts {
  readonly title: string;
  readonly body: string;
  readonly logPath: string;
  readonly primary: string;
  readonly secondary: string;
}

export function spawnFailureNotice(notice: SpawnFailureFacts): NoticeCopy {
  return {
    title: notice.title,
    body: notice.body,
    note: `LOG · ${notice.logPath}`,
    primary: notice.primary,
    secondary: notice.secondary,
  };
}

/**
 * Whether §11.1's report is worth offering a way into — the one rule, for the two surfaces that
 * offer it.
 *
 * §8.0's banner asked `problemsNotice`; §8.3a's empty-state link asked **`scan.status`**, and the
 * panel it opens is gated on the report having been read. So a report that had not been read left
 * a drawn button that silently did nothing, which is §11.3a's dead control arriving through a
 * second reading of one run.
 */
export function hasReportableProblems(problems: Problems | null): boolean {
  return problems !== null && problemsNotice(problems) !== null;
}

export function problemsNotice(problems: Problems): NoticeCopy | null {
  const count = problems.header.problemCount;
  // `null` is a scan still running; `0` is a finished scan with nothing to report. Neither
  // earns a banner, and neither does ambiguous lineage — nothing broke there (§11.1).
  if (problems.runId === null || count === null || count === 0) return null;
  return {
    title:
      count === 1
        ? 'THE LAST SCAN LEFT ONE PROBLEM'
        : `THE LAST SCAN LEFT ${String(count)} PROBLEMS`,
    body:
      'Some folders could not be read, opened or trusted. Each one is listed with the path it ' +
      'was found at.',
    note: null,
    primary: 'SEE THE SUMMARY',
    secondary: null,
  };
}

export function staleTargetsNotice(rows: readonly TargetVerification[]): NoticeCopy | null {
  // `unverified` has not been checked. Drawing it as broken is unknown rendered as a failure.
  const gone = rows.filter(
    (r) => r.verifyState === 'missing' || r.verifyState === 'not_executable',
  );
  const first = gone[0];
  if (first === undefined) return null;
  if (gone.length === 1) {
    return {
      title: 'A LAUNCH TARGET NO LONGER RESOLVES',
      body:
        `${first.execDisplay} is not where it was recorded. Re-detect it, or pick it again in ` +
        'settings.',
      note: null,
      primary: 'RE-DETECT',
      secondary: null,
    };
  }
  return {
    title: `${String(gone.length)} LAUNCH TARGETS NO LONGER RESOLVE`,
    body:
      'The recorded paths are not where they were. Re-detect them, or pick them again in ' +
      'settings.',
    note: null,
    primary: 'RE-DETECT',
    secondary: null,
  };
}

/** Measured (§11.3). v2's "~30 MB" was false by 17×; this string is the correction. */
export const RESIDENCY_MEASUREMENTS =
  '307 MB EMPTY · 522 MB WITH A FULL SHELF · 232 MB WITH THE WINDOW DESTROYED';

/**
 * §11.3 asks this once, after value has been demonstrated, and never again. The secondary is
 * therefore not `NOT NOW`: §1.4 rules on the same shape for the identity card — a label that
 * promises a later ask is a lie told in two words. The switch stays reachable in settings, and
 * the body says so, which is the true version of the same reassurance.
 */
export function residencyNotice(): NoticeCopy {
  return {
    title: 'START CODOTHECA WITH THE SYSTEM?',
    body:
      'The window stays alive for thirty minutes after you last use it, then closes itself and ' +
      'keeps only a tray icon. Off by default, and changeable in settings at any time.',
    note: RESIDENCY_MEASUREMENTS,
    primary: 'START WITH THE SYSTEM',
    secondary: 'LEAVE IT OFF',
  };
}

/**
 * §21.10's four sentences, one per `SyncNotice` variant.
 *
 * **Every outcome other than a success leaves cached values rendering normally with their stale
 * marker**, sets each affected remote field to *unknown* — never `failed`, never zero — raises
 * exactly one non-modal banner, and downgrades no score. There is no second `UNKNOWN` vocabulary
 * here and no glyph: `—` is §8.4.1's and belongs to the surface that renders a field, not to a
 * banner about the whole lane.
 *
 * **The switch is exhaustive by type.** A fifth variant added to the generated enum fails
 * `tsc` here rather than falling through to a sentence written for something else, which is the
 * failure a `default` arm would hide.
 */
export function remoteSyncNotice(kind: SyncNotice): NoticeCopy {
  switch (kind) {
    case 'throttled':
      return {
        title: 'THE FORGE IS RATE LIMITING THIS APP',
        body:
          'Remote details will fill in by themselves once the limit resets. Everything already ' +
          'read is still on screen, with the time it was read.',
        note: null,
        primary: null,
        secondary: null,
      };
    case 'unauthorized':
      return {
        title: 'THIS CONNECTION NEEDS SIGNING IN AGAIN',
        body:
          'The forge no longer accepts this token, so remote details stopped updating. Nothing ' +
          'local has changed and nothing has been removed.',
        note: null,
        primary: 'OPEN ACCOUNTS',
        secondary: null,
      };
    case 'forbidden':
      return {
        title: 'THE FORGE REFUSED THIS CONNECTION',
        body:
          'A missing permission, an organisation that has not authorised this app, or access ' +
          'that was withdrawn. Waiting will not change it; the accounts screen will say which.',
        note: null,
        primary: 'OPEN ACCOUNTS',
        secondary: null,
      };
    case 'offline':
      return {
        title: 'THE FORGE COULD NOT BE REACHED',
        // **It promises no retry.** §21.4 defers a task at three transient failures, and a
        // deferred row is left only through a revival cause, none of which has a production
        // caller. "This retries on its own" was true of the first two failures and false for ever
        // after the third — *never claim currency you do not have*, pointed forwards: copy
        // asserting a future behaviour instead of a past observation.
        body:
          'Remote details are whatever was last read, with the time beside them. Nothing local ' +
          'has changed, and nothing here needs doing.',
        note: null,
        primary: null,
        secondary: null,
      };
    default: {
      const unhandled: never = kind;
      return unhandled;
    }
  }
}
