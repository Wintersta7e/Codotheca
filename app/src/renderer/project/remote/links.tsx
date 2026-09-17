/**
 * §25.2's links row.
 *
 * **Buttons, not anchors.** There is no URL in this document to put in an `href`: the renderer
 * sends `{ projectId, kind }` and the shell builds, re-checks, confirms and opens the address. An
 * anchor with no `href` is the dead control §11.3a forbids, and an anchor *with* one would mean
 * the renderer held a URL derived from repository content.
 *
 * **The row is absent entirely when `linkable` is false**, and the key beside it renders as text
 * with no link affordance — an https form guessed from an arbitrary remote host is a URL the
 * product invented.
 */
import type { ReactElement } from 'react';

import type { ProjectId, RemoteFacts, RemoteLinkKind } from '../../../generated/protocol';

const LINKS: readonly { readonly kind: RemoteLinkKind; readonly label: string }[] = [
  { kind: 'repository', label: 'REPOSITORY' },
  { kind: 'issues', label: 'ISSUES' },
  { kind: 'pulls', label: 'PULL REQUESTS' },
  { kind: 'actions', label: 'ACTIONS' },
  { kind: 'releases', label: 'RELEASES' },
];

export interface RemoteLinksProps {
  readonly projectId: ProjectId;
  readonly facts: RemoteFacts;
  readonly onOpenLink: (projectId: ProjectId, kind: RemoteLinkKind) => void;
}

export function RemoteLinks({
  projectId,
  facts,
  onOpenLink,
}: RemoteLinksProps): ReactElement | null {
  if (!facts.linkable) return null;
  return (
    <div className="cp-remote-links" data-testid="cp-remote-links">
      {LINKS.map((link) => (
        <button
          key={link.kind}
          type="button"
          className="cp-remote-link"
          onClick={() => {
            onOpenLink(projectId, link.kind);
          }}
        >
          {link.label}
        </button>
      ))}
    </div>
  );
}
