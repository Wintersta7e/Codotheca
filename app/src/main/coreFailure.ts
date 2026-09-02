/**
 * The five ways an installation stops the core starting, and the sentence for each.
 *
 * §11.2's core-spawn-failure surface, placed by §11.2a: not a full-screen window but §8.0's
 * notice slot at priority 1, which has no dismissal and stands until the condition clears. Plan
 * 17 draws the box; this decides which cause it is and what it says.
 *
 * All five are packaging or installation faults, which is why they live here. Two of them need
 * more than the error string:
 *   - Linux returns EACCES both for a `noexec` mount and for a lost execute bit. Only the file
 *     mode separates them, so the mode is an input.
 *   - A too-old C library is not a spawn failure at all. The process starts, the dynamic loader
 *     writes to stderr, and the process exits — twice — so it arrives as `crash_loop` with the
 *     loader's line in the stderr tail the supervisor keeps.
 *
 * §2.4: the core's own `message` is diagnostic and never shown raw. Every sentence below is
 * shell-owned prose.
 */
import type { CoreStatus } from './core/supervisor';

export type SpawnFailureCause =
  | 'quarantined'
  | 'noexec'
  | 'not-executable'
  | 'wrong-architecture'
  | 'glibc-too-old'
  | 'missing'
  | 'unknown';

export interface SpawnFailureInput {
  /** The supervisor's failure detail, which includes the stderr tail when there was one. */
  readonly detail: string;
  readonly platform: NodeJS.Platform;
  /** `fs.statSync(coreBinaryPath).mode`, or null when the file could not be stat'd. */
  readonly mode: number | null;
}

const EXECUTE_BITS = 0o111;

export function classifySpawnFailure(input: SpawnFailureInput): SpawnFailureCause {
  const { detail, mode, platform } = input;
  if (/ENOENT|no such file or directory/iu.test(detail)) return 'missing';
  if (/GLIBC_[0-9.]+.{0,20}not found|GLIBC_[0-9.]+' not found/u.test(detail)) {
    return 'glibc-too-old';
  }
  if (/ENOEXEC|Exec format error|cannot execute binary file/iu.test(detail)) {
    return 'wrong-architecture';
  }
  if (
    platform === 'win32' &&
    /EPERM|UNKNOWN|contains a virus|Operation did not complete/iu.test(detail)
  ) {
    return 'quarantined';
  }
  if (/EACCES|Permission denied/iu.test(detail)) {
    return mode !== null && (mode & EXECUTE_BITS) !== 0 ? 'noexec' : 'not-executable';
  }
  return 'unknown';
}

export const SPAWN_FAILURE_SENTENCE: Readonly<Record<SpawnFailureCause, string>> = {
  quarantined:
    "A security tool is holding Codotheca's background process and will not let it start. Allow it, then try again.",
  noexec:
    'Codotheca is installed on a volume that does not permit running programs. Installing it on a volume without that restriction will fix this.',
  'not-executable':
    "Codotheca's background process lost its permission to run during installation.",
  'wrong-architecture':
    "This copy of Codotheca's background process was built for a different processor than this machine has.",
  'glibc-too-old': "This system's C library is older than this build of Codotheca needs.",
  missing: "Codotheca's background process is not where the installation left it.",
  unknown: "Codotheca's background process would not start.",
};

export const CORE_FAILURE_TITLE = "CODOTHECA'S BACKGROUND PROCESS DID NOT START";

export interface CoreFailureNotice {
  readonly title: string;
  readonly body: string;
  readonly logPath: string;
  readonly primary: 'RETRY';
  readonly secondary: 'OPEN THE LOG';
  /** Priority 1 is the one notice with no dismissal. */
  readonly dismissible: false;
}

export function coreFailureNotice(
  status: CoreStatus,
  probe: { readonly platform: NodeJS.Platform; readonly mode: number | null },
): CoreFailureNotice | null {
  if (status.kind !== 'failed') return null;
  return {
    title: CORE_FAILURE_TITLE,
    body:
      status.reason === 'protocol_version'
        ? status.detail
        : SPAWN_FAILURE_SENTENCE[
            classifySpawnFailure({
              detail: status.detail,
              platform: probe.platform,
              mode: probe.mode,
            })
          ],
    logPath: status.logPath,
    primary: 'RETRY',
    secondary: 'OPEN THE LOG',
    dismissible: false,
  };
}
