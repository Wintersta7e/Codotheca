/**
 * §24.3's Install offer, as a surface holds it.
 *
 * **The destination is a `RootId` and never a path.** §24.3a stores the choice as
 * `Settings.installRootId`, chosen once from the roots that already exist; `ADD A FOLDER…` routes
 * to `pickRoot`, which is the shell's dialog and the only path-origination channel the product
 * has. Nothing here composes a destination string — `install.preview` returns the composed
 * display form, because `path_display` is lossy and the collision verdict is core knowledge.
 *
 * **The preview may be asked for eagerly, and the pre-flight of §24.7 may not.** `install.preview`
 * is read-only and unprivileged: it spawns no process and writes nothing, *"which is what lets the
 * page ask before the user has committed to anything"*. That is the whole difference between this
 * hook and `useUninstall`.
 *
 * **A refusal is a reply.** Neither the collision verdict on the preview nor the one `install.start`
 * answers with is an error, and neither is composed here: a refused start re-reads the preview, so
 * what is rendered is always the core's own current answer.
 */
import { useCallback, useEffect, useRef, useState } from 'react';

import type { InstallPreview, ProjectId, Root, RootId } from '../../generated/protocol';
import { useProjectPageDeps } from '../project/deps';
import { toSettingsPatch } from '../settings/Drawer';

export interface InstallOffer {
  /**
   * `undefined` is *this surface is not offering it*, `null` is *not computed yet* — which
   * `InstallControl` renders as nothing rather than as an enabled button over an unknown
   * destination — and a preview is the core's answer.
   */
  readonly preview: InstallPreview | null | undefined;
  /**
   * Non-null exactly when no root has been chosen yet: the roots there are to choose from.
   * §24.3a makes the choice a stored setting, so this is empty on every later visit.
   */
  readonly roots: readonly Root[] | null;
  readonly chosen: RootId | null;
  readonly choose: (rootId: RootId) => void;
  /** Invokes the shell's folder dialog and nothing else. */
  readonly addFolder: () => void;
  readonly start: () => void;
}

/**
 * `projectId` is `null` on a surface that is not offering Install — a project with a working copy
 * has nothing to clone — and nothing is asked for at all in that case.
 */
export function useInstallOffer(projectId: ProjectId | null): InstallOffer {
  const deps = useProjectPageDeps();
  const [preview, setPreview] = useState<InstallPreview | null | undefined>(undefined);
  const [roots, setRoots] = useState<readonly Root[] | null>(null);
  const [chosen, setChosen] = useState<RootId | null>(null);
  // Bumped by anything that can change the stored root, which re-runs the read below rather than
  // duplicating it: one place asks for the setting and the preview, in that order.
  const [nonce, setNonce] = useState(0);
  const inFlight = useRef(false);

  useEffect(() => {
    if (projectId === null) {
      setPreview(undefined);
      setRoots(null);
      setChosen(null);
      return undefined;
    }
    let live = true;
    // Read through a call: the cleanup clears `live` during an await, and a plain read would stay
    // narrowed to whatever the first test found.
    const stillLive = (): boolean => live;
    setPreview(null);
    void (async () => {
      try {
        const settings = await deps.request('settings.get', {});
        if (!stillLive()) return;
        const rootId = settings.installRootId;
        setChosen(rootId);
        if (rootId === null) {
          // No destination is stored, so there is nothing to preview yet. §24.3a's chooser is
          // what the surface draws instead, and `install.preview` is not callable without a root.
          const list = await deps.request('roots.list', {});
          if (stillLive()) setRoots(list);
          return;
        }
        setRoots(null);
        const answer = await deps.request('install.preview', { projectId, rootId });
        if (stillLive()) setPreview(answer);
      } catch {
        // §11.4's failure window owns the copy. An offer this surface cannot compute is not an
        // offer, and a refusal it invented would be a claim about the remote it cannot make.
        if (stillLive()) setPreview(null);
      }
    })();
    return () => {
      live = false;
    };
  }, [deps, projectId, nonce]);

  const choose = useCallback(
    (rootId: RootId) => {
      void deps
        // `toSettingsPatch` is the one builder of this shape: the generated patch is total, so an
        // omitted key is not "leave it", and a second builder here is the drifting-copy defect.
        .request('settings.set', { patch: toSettingsPatch({ installRootId: rootId }) })
        .then(() => {
          setNonce((n) => n + 1);
        })
        .catch(() => undefined);
    },
    [deps],
  );

  const addFolder = useCallback(() => {
    void deps
      .pickRoot(false)
      .then((reply) => {
        // A cancelled dialog changes nothing; a failure is the shell's to report. Only a root
        // that was actually added is worth re-reading for.
        if (reply.kind === 'added') setNonce((n) => n + 1);
      })
      .catch(() => undefined);
  }, [deps]);

  const start = useCallback(() => {
    // §2.4: the start is non-idempotent, so the guard is a ref and holds within the tick as well
    // as across the re-render a state flag would have to wait for.
    if (projectId === null || chosen === null || inFlight.current) return;
    inFlight.current = true;
    void deps
      .installStart(projectId, chosen)
      .then((reply) => {
        inFlight.current = false;
        // A refusal rides inside the reply. Re-read rather than render it from here: the preview
        // is the core's own statement of why, and there is then one place that says it.
        if (reply.kind === 'failed' || (reply.start as { runId: number | null }).runId === null) {
          setNonce((n) => n + 1);
        }
      })
      .catch(() => {
        inFlight.current = false;
        setNonce((n) => n + 1);
      });
  }, [chosen, deps, projectId]);

  return { preview, roots, chosen, choose, addFolder, start };
}
