import { FOCUS_HEARTBEAT_MS, focusArgs, type FocusReport } from '../../shared/sessionFocus';

export interface FocusReporterDeps {
  /** Issues `session.focus`. Injected so this module needs neither the bridge nor a clock. */
  readonly report: (args: FocusReport) => Promise<unknown>;
  readonly setTimer: (fn: () => void, ms: number) => number;
  readonly clearTimer: (handle: number) => void;
}

export interface FocusReporter {
  /** The view changed: the project whose page or live tile it is, or `null` for any other view. */
  readonly focus: (projectId: number | null) => void;
  /** Keyboard or pointer input inside P's page or live tile; untrusted input is ignored. */
  readonly input: (projectId: number, trusted: boolean) => void;
  /** P's session started, so the input that launched it is not evidence of anyone at P. */
  readonly sessionStarted: (projectId: number) => void;
  readonly stop: () => void;
  readonly claimed: () => number | null;
}

/**
 * §9's renderer half, gated on input (§39.3a): a project is claimed because someone *touched*
 * its view, never because the view is on screen — a page left open with nobody at it would
 * otherwise hold a segment open for as long as the window stayed up.
 *
 * **Claiming**: the first trusted input in P's view sends P at once. **Liveness**: a claim that
 * is never repeated cannot be told apart from one made by a renderer that has since died, so it
 * is repeated at each heartbeat — but only when input arrived in the interval just ended.
 * Without it the heartbeat stops and the core lets the claim lapse after `FOCUS_STALE_SECS`;
 * the next input claims again at once. **Releasing** is immediate and ungated: a view that is
 * not the claimed project's sends one `null`, because the core believes a lapsing claim until
 * it goes stale.
 *
 * R3: the reporter reads no clock. The core stamps every report with its own `monotonic_ms`.
 */
export function createFocusReporter(deps: FocusReporterDeps): FocusReporter {
  let claimed: number | null = null;
  let evidence = false;
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
    timer = null;
    if (claimed === null || !evidence) return;
    evidence = false;
    send(claimed);
    timer = deps.setTimer(beat, FOCUS_HEARTBEAT_MS);
  };

  const release = (): void => {
    stopTimer();
    evidence = false;
    if (claimed === null) return;
    claimed = null;
    send(null);
  };

  return {
    focus(projectId) {
      if (claimed !== null && claimed !== projectId) release();
    },
    input(projectId, trusted) {
      if (!trusted) return;
      if (claimed === projectId && timer !== null) {
        evidence = true;
        return;
      }
      stopTimer();
      claimed = projectId;
      evidence = false;
      send(projectId);
      timer = deps.setTimer(beat, FOCUS_HEARTBEAT_MS);
    },
    sessionStarted(projectId) {
      if (claimed === projectId) evidence = false;
    },
    stop: release,
    claimed: () => claimed,
  };
}
