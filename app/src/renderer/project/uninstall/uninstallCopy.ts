/**
 * §24.8's rendered strings, owned here so the control is arrangement and the sentences are
 * testable without a DOM — the shape `locationCopy.ts` already uses.
 *
 * **The word is `UNINSTALL`.** Phase 2 ships no `DELETE`, no `REMOVE`, no `RECLAIM SPACE` and
 * **no override of any kind**.
 */
import type { UninstallBlocker, UninstallDisposition } from '../../../generated/protocol';

export const UNINSTALL_LABEL = 'UNINSTALL';

/** §24.8's confirmation, exactly. */
export const UNINSTALL_CONFIRMATION = 'Uninstall — the tile stays, re-clone any time.';

/** Said before the click, so the copy knows what will happen to it (§24.7F). */
export const TRASH_AVAILABLE_NOTE = 'This copy goes to the recycle bin.';
export const TRASH_UNAVAILABLE_NOTE =
  'The recycle bin is not available here, so this copy is removed outright.';

/** While the verdict is in flight. It never renders enabled-then-disabled. */
export const CHECKING_NOTE = 'Checking what this copy holds…';

/** §8.4.1's glyph for a value that was not established. There is no second UNKNOWN vocabulary. */
export const UNKNOWN_GLYPH = '—';

/**
 * One sentence per blocker, in an **exhaustive switch over the generated enum**: a fifteenth
 * variant fails to compile rather than rendering nothing.
 *
 * Every sentence says what was found, not what the user should do — the control's job is to name
 * the reason, and the remedy is the user's.
 */
export function blockerSentence(blocker: UninstallBlocker): string {
  switch (blocker) {
    case 'unpushed_commits':
      return 'This copy has commits no remote has.';
    case 'uncommitted_changes':
      return 'There are changes here that are not committed.';
    case 'stash_present':
      return 'There is stashed work here.';
    case 'untracked_precious':
      return 'There are untracked files here that are not build output.';
    case 'ignored_precious':
      return 'There are ignored files here that do not look rebuildable.';
    case 'submodule_unsafe':
      return 'A submodule holds work of its own.';
    case 'linked_worktree':
      return 'Another worktree points into this copy.';
    case 'shallow_clone':
      return 'This is a shallow clone, so I cannot tell whether every commit is upstream.';
    case 'remote_unreachable':
      // §24.7C's existing copy, verbatim.
      return "Can't reach GitHub, so I can't confirm this is backed up. Everything else still works.";
    case 'remote_is_local_mirror':
      return 'The remote is on this machine, so it is not a second copy.';
    case 'stash_unreadable':
      return 'I could not read the stash log, so I cannot tell whether anything is stashed.';
    case 'live_session':
      return 'A session is open on this copy.';
    case 'refused_path':
      return 'This path is not one I will remove.';
    case 'never_observed':
      return 'I have never successfully read this copy.';
  }
}

/**
 * The three states, and they **are** three.
 *
 * `unknown` renders distinctly from `blocked` (AC-P2-24-14) — not the same disabled state with
 * different words. `unknown` is *I could not check*; `blocked` is *I checked and the answer is
 * no*, and collapsing them is the type-level violation §24.8 exists to prevent.
 */
export function dispositionHeading(disposition: UninstallDisposition): string {
  switch (disposition) {
    case 'safe':
      return UNINSTALL_CONFIRMATION;
    case 'blocked':
      return 'This copy holds work that exists nowhere else.';
    case 'unknown':
      return `I could not check everything ${UNKNOWN_GLYPH} so I will not remove this.`;
  }
}
