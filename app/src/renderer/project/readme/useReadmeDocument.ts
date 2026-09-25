/**
 * §25.5's sequence, and the order **is** the guarantee.
 *
 * 1. Mount with `ReadmeState` as today. A README that is not `present` renders §8.5.3's existing
 *    strings and **imports nothing**.
 * 2. Ask the core for the document. On any refusal, fall back to the stored paragraph — which is
 *    a real fact — and draw no frame. Never invent a state.
 * 3. `await import('./frame')`, render, serialise, set `srcdoc`. **This first paint issues zero
 *    requests of any kind**, because every image left the document as a placeholder.
 * 4. Only then ask for the assets, apply what came back, re-serialise and reassign `srcdoc`.
 * 5. `truncated` is stated below the frame, so the panel says the document was cut rather than
 *    implying it ended.
 *
 * **One dynamic import, not four.** `./frame` re-exports the pipeline, so the whole markup stack —
 * parser, sanitiser, highlighter, typesetter — arrives as one chunk off the first-paint path.
 */
import { useCallback, useEffect, useRef, useState } from 'react';

import type { LocationId, ProjectId, ReadmeState } from '../../../generated/protocol';
import { useProjectPageDeps } from '../deps';

export interface ReadmeDocumentState {
  /** `stored` is §8.5.3's paragraph; `frame` is the rendered document. */
  readonly phase: 'stored' | 'frame';
  readonly srcdoc: string | null;
  /**
   * Which document this is, counting from 1.
   *
   * **Chromium does not re-navigate a sandboxed `srcdoc` frame when the attribute is replaced** —
   * measured in the built app: the attribute held the substituted document and the frame kept
   * rendering the first one, with `will-frame-navigate` firing zero times for the reassignment.
   * The panel therefore keys the element on this number, so a new document is a **new element**
   * that mounts with its own `srcdoc` and commits it. Nothing about the sandbox is relaxed.
   */
  readonly revision: number;
  readonly anchorCount: number;
  readonly truncated: boolean;
  /** How many references the core refused for **consent**, which is not a failure. */
  readonly blockedRemote: number;
  /** Which of `README_NAMES` was read, so the header names the file rather than a literal. */
  readonly path: string | null;
}

export interface ReadmeDocument extends ReadmeDocumentState {
  /** §25.5's one control. Grants remote images for this repository and re-runs step 4. */
  readonly grantRemote: () => void;
}

const STORED: ReadmeDocumentState = {
  phase: 'stored',
  srcdoc: null,
  revision: 0,
  anchorCount: 0,
  truncated: false,
  blockedRemote: 0,
  path: null,
};

/**
 * The renderer's split, which is a **hint**: the core re-classifies every reference, so a value
 * that lands in the wrong list is handled correctly anyway.
 */
export function splitRefs(refs: readonly string[]): { local: string[]; remote: string[] } {
  const local: string[] = [];
  const remote: string[] = [];
  for (const reference of refs) {
    if (reference.includes('://') || reference.startsWith('data:')) remote.push(reference);
    else local.push(reference);
  }
  return { local, remote };
}

export interface UseReadmeDocumentArgs {
  readonly projectId: ProjectId;
  readonly locationId: LocationId | null;
  readonly readme: ReadmeState;
}

export function useReadmeDocument({
  projectId,
  locationId,
  readme,
}: UseReadmeDocumentArgs): ReadmeDocument {
  const deps = useProjectPageDeps();
  const [state, setState] = useState<ReadmeDocumentState>(STORED);
  // Bumped by `projects/readme_remote_changed`, which is what re-runs the asset step after a
  // consent change — the wire, not the click, so the renderer and the row cannot disagree.
  const [consentGeneration, setConsentGeneration] = useState(0);
  const projectRef = useRef(projectId);
  projectRef.current = projectId;

  useEffect(() => {
    return deps.subscribe((event) => {
      if (event.topic !== 'projects' || event.event !== 'readme_remote_changed') return;
      const payload = event.data as { id?: number } | undefined;
      if (payload?.id !== projectRef.current) return;
      setConsentGeneration((generation) => generation + 1);
    });
  }, [deps]);

  useEffect(() => {
    if (readme.state !== 'present' || locationId === null) {
      setState(STORED);
      return undefined;
    }
    let cancelled = false;
    // Read through a call: the cleanup sets `cancelled` during an await, and a plain read would
    // stay narrowed to whatever the first test found.
    const isCancelled = (): boolean => cancelled;

    const run = async (): Promise<void> => {
      const source = await deps.request('projects.readme', { projectId, locationId });
      if (isCancelled()) return;
      if (source.state !== 'present' || source.text === null || source.text === '') return;

      // The one dynamic import. Everything the pipeline needs arrives with it.
      const pipeline = await import('./frame');
      if (isCancelled()) return;

      const rendered = pipeline.renderMarkup(source.text);
      const tokens = pipeline.readFrameTokens(document.documentElement);
      setState((previous) => ({
        phase: 'frame',
        srcdoc: pipeline.buildSrcdoc(rendered.fragment, tokens),
        revision: previous.revision + 1,
        anchorCount: rendered.anchorCount,
        truncated: source.truncated,
        blockedRemote: 0,
        path: source.path,
      }));
      if (rendered.imageRefs.length === 0) return;

      const { local, remote } = splitRefs(rendered.imageRefs);
      const assets = await deps.request('projects.readmeAssets', {
        projectId,
        locationId,
        local,
        remote,
      });
      if (isCancelled()) return;
      pipeline.applyAssets(rendered.fragment, assets);
      const blockedRemote = assets.filter((asset) => asset.state === 'blocked').length;
      setState((previous) => ({
        ...previous,
        srcdoc: pipeline.buildSrcdoc(rendered.fragment, tokens),
        revision: previous.revision + 1,
        blockedRemote,
      }));
    };

    // A refusal is not a state: the panel keeps §8.5.3's stored paragraph, which is a fact the
    // index already holds, rather than rendering an empty frame or inventing an error.
    void run().catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [deps, projectId, locationId, readme.state, consentGeneration]);

  const grantRemote = useCallback(() => {
    void deps
      .request('projects.setReadmeRemote', { projectId, allow: true })
      .catch(() => undefined);
  }, [deps, projectId]);

  return { ...state, grantRemote };
}
