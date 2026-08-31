import { FOCUS_HEARTBEAT_MS, focusArgs, type FocusReport } from '../../shared/sessionFocus';

export interface FocusReporterDeps {
  /** Issues `session.focus`. Injected so this module needs neither the bridge nor a clock. */
  readonly report: (args: FocusReport) => Promise<unknown>;
  readonly setTimer: (fn: () => void, ms: number) => number;
  readonly clearTimer: (handle: number) => void;
}

export interface FocusReporter {
  /** The project whose page or live tile is focused, or `null` for every other view. */
  readonly focus: (projectId: number | null) => void;
  readonly stop: () => void;
  readonly claimed: () => number | null;
}

/**
 * §9's renderer half, with two jobs that pull in opposite directions.
 *
 * **Coalescing**: a project page re-rendering ten times a second must not send ten reports, so
 * an unchanged claim sends nothing. **Liveness**: a claim that is never repeated cannot be
 * told apart from one made by a renderer that has since died, so an unchanged claim must be
 * repeated on a timer. The resolution is that the heartbeat runs *only while a project is
 * claimed*: releasing sends one `null` and stops the timer, because there is nothing left to
 * prove.
 *
 * Which project counts is §9's, not this file's: app focus extends a segment only if the
 * focused view is that project's — its page, or its live tile. Callers pass `null` for the
 * shelf, the palette, settings and every other view, which is what closes the farming hole.
 *
 * R3: the reporter reads no clock. The core stamps every report with its own `monotonic_ms`.
 */
export function createFocusReporter(deps: FocusReporterDeps): FocusReporter {
  let claimed: number | null = null;
  let timer: number | null = null;

  const stopTimer = (): void => {
    if (timer !== null) {
      deps.clearTimer(timer);
      timer = null;
    }
  };

  const send = (projectId: number | null): void => {
    // A transient core restart must not silence the reporter for the rest of the session.
    void deps.report(focusArgs(projectId)).catch(() => undefined);
  };

  const beat = (): void => {
    if (claimed === null) return;
    send(claimed);
    timer = deps.setTimer(beat, FOCUS_HEARTBEAT_MS);
  };

  return {
    focus(projectId) {
      if (projectId === claimed) return;
      claimed = projectId;
      stopTimer();
      send(projectId);
      // A claim is a liveness assertion and is repeated; a release asserts nothing.
      if (projectId !== null) timer = deps.setTimer(beat, FOCUS_HEARTBEAT_MS);
    },
    stop() {
      stopTimer();
      if (claimed === null) return;
      claimed = null;
      send(null);
    },
    claimed: () => claimed,
  };
}
