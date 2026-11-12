/**
 * §54.4 caller 2: opening a project page asks the core to re-read its current primary copy.
 *
 * The core decides everything — which copy, whether it is current, what the chain reads — and
 * the page only sends. Before this, no user action queued a fresh read at all, so a fix made in
 * an editor since the last walk could never reach §34's restoration as `foreground`.
 *
 * Sent once per mount and once per id: the view stays mounted when the id changes, so the
 * effect's `projectId` key is §54.4's "and when its id changes". It reads no event, so a
 * `projects.get` reload driven by `upserted`/`condition_changed`/`flags_changed` never sends
 * again — a re-read whose own settle re-sent it would never stop.
 *
 * §56 (GA26): this mount is one of the existing triggers that resume reads after an exclusion is
 * removed. Nothing here gates on "rescanned since removal".
 */
import { useEffect } from 'react';

import type { ProjectId } from '../../../generated/protocol';
import { useProjectPageDeps } from '../deps';

/** What the page reads from the re-check. Pending and the result line arrive with §54.5. */
export interface RecheckHandle {
  readonly pending: false;
}

const HANDLE: RecheckHandle = { pending: false };

export function useRecheck(projectId: ProjectId): RecheckHandle {
  const deps = useProjectPageDeps();

  useEffect(() => {
    // A refusal changes nothing the page shows: the stored reading stays exactly as honest as it
    // was, and the next open asks again.
    deps.request('projects.recheck', { id: projectId }).catch(() => undefined);
  }, [deps, projectId]);

  return HANDLE;
}
