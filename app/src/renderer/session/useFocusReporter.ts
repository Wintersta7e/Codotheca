/**
 * §39.3a's production caller: one focus reporter per window, and the input that feeds it.
 *
 * The app root builds the reporter and routes every view change to `focus`; a project's page and
 * its live tile feed `input` from their own roots. Nothing here originates a path or an
 * executable — `session.focus` carries one project id or `null`.
 */
import { createContext, useContext, useEffect, useMemo, type RefObject } from 'react';

import type { ProjectId } from '../../generated/protocol';
import type { AppDeps } from '../app/deps';
import { createFocusReporter, type FocusReporter } from './focusReporter';

/** The input that counts as someone at a project's view; focus or hover alone never does. */
const EVIDENCE_EVENTS = ['keydown', 'pointerdown', 'wheel'] as const;

export const FocusReporterContext = createContext<FocusReporter | null>(null);

/** Builds the window's one reporter, stopped on unmount, and tells it when a session starts. */
export function useFocusReporter(deps: AppDeps): FocusReporter {
  const reporter = useMemo(
    () =>
      createFocusReporter({
        // The reporter speaks in plain numbers; every id it holds came from a `ProjectId`.
        report: (args) =>
          deps.request('session.focus', { projectId: args.projectId as ProjectId | null }),
        setTimer: (fn, ms) => window.setTimeout(fn, ms),
        clearTimer: (handle) => {
          window.clearTimeout(handle);
        },
      }),
    [deps],
  );

  useEffect(
    () => () => {
      reporter.stop();
    },
    [reporter],
  );

  useEffect(
    () =>
      deps.subscribe((event) => {
        if (event.topic !== 'session' || event.event !== 'started') return;
        const session: unknown = (event.data as { session?: unknown } | null)?.session;
        if (typeof session !== 'object' || session === null || !('projectId' in session)) return;
        if (typeof session.projectId === 'number') reporter.sessionStarted(session.projectId);
      }),
    [deps, reporter],
  );

  return reporter;
}

/**
 * Feeds the window's reporter from `ref`'s element while `projectId` is set. Capture phase, so a
 * control that stops propagation still counts as input in the view it sits in. Input counts only
 * when it is trusted and the document has focus — a script's synthetic event never claims.
 */
export function useFocusEvidence(
  ref: RefObject<HTMLElement | null>,
  projectId: number | null,
): void {
  const reporter = useContext(FocusReporterContext);
  useEffect(() => {
    const element = ref.current;
    if (reporter === null || projectId === null || element === null) return;
    const onInput = (event: Event): void => {
      reporter.input(projectId, event.isTrusted && document.hasFocus());
    };
    for (const type of EVIDENCE_EVENTS) {
      element.addEventListener(type, onInput, { capture: true, passive: true });
    }
    return () => {
      for (const type of EVIDENCE_EVENTS) {
        element.removeEventListener(type, onInput, { capture: true });
      }
    };
  }, [ref, reporter, projectId]);
}
