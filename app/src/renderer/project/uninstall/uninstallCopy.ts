/**
 * §24.8's rendered strings, owned here so the control is arrangement and the sentences are
 * testable without a DOM — the shape `locationCopy.ts` already uses.
 *
 * **The word is `UNINSTALL`.** Phase 2 ships no `DELETE`, no `REMOVE`, no `RECLAIM SPACE` and
 * **no override of any kind**.
 */
import type {
  TrashRefusalKind,
  UninstallBlocker,
  UninstallDisposition,
} from '../../../generated/protocol';

export const UNINSTALL_LABEL = 'UNINSTALL';

/**
 * The affordance that **opens** the check, which is a different control from the one that
 * removes. The pre-flight reads every remote over the network (§47's verifying read), so
 * `protocol.json` says outright it is not callable on hover — nothing may run it but a press. The
 * ellipsis is `INSTALL…`'s, and says the same thing: this opens something, it does not do
 * something.
 */
export const UNINSTALL_OPEN_LABEL = 'UNINSTALL…';

/** Said beside the affordance, so the press is not a leap. */
export const UNINSTALL_OPEN_NOTE = 'Check whether this copy is safe to remove.';

/** §24.8's confirmation, exactly. */
export const UNINSTALL_CONFIRMATION = 'Uninstall — the tile stays, re-clone any time.';

/** Said before the click, so the copy knows what will happen to it (§24.7F). */
export const TRASH_AVAILABLE_NOTE = 'This copy goes to the recycle bin.';

/**
 * §46.7: why the recycle bin cannot take this copy, said **before** the click — and there is no
 * click, because nothing removes a copy outright instead. An **exhaustive switch over the
 * generated enum**, so a new reason fails to compile rather than rendering nothing.
 */
export function trashRefusalSentence(kind: TrashRefusalKind): string {
  switch (kind) {
    case 'unsupported':
      return 'There is no recycle bin for this location, so I will not uninstall this copy.';
    case 'network_drive':
      return 'This copy is on a network drive, which has no recycle bin, so I will not uninstall it.';
    case 'oversized_folder':
      return 'This copy is larger than the space left in the recycle bin, so I will not uninstall it.';
    case 'disabled_on_volume':
      return 'The recycle bin is turned off for this drive, so I will not uninstall this copy.';
    case 'capacity_unknown':
      return "I could not read this drive's recycle-bin settings, so I will not uninstall this copy.";
  }
}

/** While the verdict is in flight. It never renders enabled-then-disabled. */
export const CHECKING_NOTE = 'Checking what this copy holds…';

/** §8.4.1's glyph for a value that was not established. There is no second UNKNOWN vocabulary. */
export const UNKNOWN_GLYPH = '—';

/**
 * One sentence per blocker, in an **exhaustive switch over the generated enum**: a new variant
 * fails to compile rather than rendering nothing.
 *
 * Every sentence says what was found, not what the user should do — the control's job is to name
 * the reason, and the remedy is the user's. §45.10: each states what was read and never claims
 * *backed up*; `remote_unreachable` never says *unpushed*, and `no_remote` never says
 * *unreachable*. The words are the user's.
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
      return 'There are untracked files or hooks here that are not build output.';
    case 'ignored_precious':
      return 'There are ignored files here that do not look rebuildable.';
    case 'submodule_unsafe':
      return 'A repository inside this copy holds work of its own.';
    case 'linked_worktree':
      return 'This copy and another worktree are linked.';
    case 'shallow_clone':
      return 'This is a shallow clone, so I cannot tell whether every commit is upstream.';
    case 'remote_unreachable':
      return "A remote did not answer, so I can't confirm this copy's work is on it. Everything else still works.";
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
    case 'no_remote':
      return 'This copy has no remote on another machine, so nothing I read shows its commits anywhere else.';
    case 'unpushed_tag':
      return 'This copy has an annotated tag no remote has.';
    case 'interrupted_operation':
      return "An operation on this copy's history was left unfinished.";
    case 'borrowed_by_another_repository':
      return 'Another repository relies on files inside this copy.';
    case 'refs_unreadable':
      return 'I could not read every branch and tag here, so I cannot tell what is unique to this copy.';
    case 'hidden_from_status':
      return 'Some tracked files here are set to hide their changes, so I cannot tell whether they changed.';
    case 'lfs_unverified':
      return 'This copy holds large-file content, and I cannot confirm any server has it.';
    case 'nesting_too_deep':
      return 'Repositories are nested here more deeply than I check, so I cannot tell what they hold.';
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
