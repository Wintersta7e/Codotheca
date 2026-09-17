/**
 * §24.3d's Install affordance, in the slot Play occupies on a cloned project.
 *
 * **Offered in exactly two places** — the blueprint card's primary action slot and the project
 * page's left rail — and nowhere else. The same discipline as *roasting only inside an opened
 * project card*; `app/test/installSites.test.ts` walks the renderer and fails on a third.
 *
 * On `private_needs_upgrade` the control **states the consequence and offers §20.3's upgrade**. It
 * never auto-upgrades and never silently retries anonymously: a retry without credentials against
 * a private remote returns 404, which would render the repository as *gone* when it is merely
 * unreadable.
 *
 * **Name collision, deliberately not collapsed.** §8.5.3's cut `INSTALL` block — a synthesised
 * build command — shares four letters with this control and nothing else. It is not resurrected.
 */
import type { ReactElement } from 'react';

import type { InstallPreview, InstallRefusal } from '../../generated/protocol.js';

export const INSTALL_LABEL = 'INSTALL…';
export const UPGRADE_LABEL = 'UPGRADE ACCESS';

/**
 * §24.3d's refusals as sentences. **No refusal renders the repository as gone or missing**: every
 * one of these is a reason the install cannot start, not a claim about whether the remote exists.
 */
export const REFUSAL_COPY: Readonly<Record<InstallRefusal, string>> = {
  destination_exists: 'Something else is already at that path.',
  already_installed: 'This is already installed there.',
  unsafe_name: 'The repository name cannot be a folder on this system.',
  root_unavailable: 'That folder is not available right now.',
  no_clone_url: 'This project has no HTTPS address to clone from.',
  no_install_root_chosen: 'Choose where to install first.',
  private_needs_upgrade: 'This repository is private. Your access does not cover it yet.',
};

export interface InstallControlProps {
  readonly preview: InstallPreview | null;
  readonly onInstall: () => void;
  readonly onOpenUpgrade: () => void;
}

export function InstallControl({
  preview,
  onInstall,
  onOpenUpgrade,
}: InstallControlProps): ReactElement | null {
  // No preview yet is *not computed* — not a refusal, and not an enabled button over an unknown
  // destination.
  if (preview === null) return null;

  if (preview.refusedBecause !== null) {
    const refusal = preview.refusedBecause;
    return (
      <div className="cdt-install-control" data-refused={refusal} role="group">
        <p className="cdt-install-control__reason">{REFUSAL_COPY[refusal]}</p>
        {refusal === 'private_needs_upgrade' ? (
          <button type="button" onClick={onOpenUpgrade}>
            {UPGRADE_LABEL}
          </button>
        ) : null}
      </div>
    );
  }

  return (
    <div className="cdt-install-control" role="group">
      {preview.destination === null ? null : (
        // The composed display form, built by the core. The renderer never assembles one.
        <p className="cdt-install-control__destination">{preview.destination.display}</p>
      )}
      <button type="button" onClick={onInstall}>
        {INSTALL_LABEL}
      </button>
    </div>
  );
}
