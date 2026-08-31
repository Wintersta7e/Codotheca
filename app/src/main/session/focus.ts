import { focusArgs, type FocusReport } from '../../shared/sessionFocus';

export interface WindowFocusDeps {
  readonly release: (args: FocusReport) => Promise<unknown>;
  readonly onBlur: (cb: () => void) => void;
  readonly onHide: (cb: () => void) => void;
  readonly onDestroyed: (cb: () => void) => void;
  readonly onError: (detail: string) => void;
}

/**
 * The shell's entire half of §9's focus protocol: it releases a claim and never makes one.
 *
 * L5: `release` takes a `FocusReport` this module builds itself with `focusArgs(null)`, so
 * there is no code path from a `BrowserWindow` event to a project id, and there must not be.
 * The residency hybrid destroys the window after 30 minutes, and a shell that could assert
 * focus would be asserting it from a process with no renderer in it.
 *
 * Three events release, not one. **Blur** is the ordinary case. **Hide** is the resident
 * window going to the tray, which does not always blur first. **Destroy** is the residency
 * hybrid, and it is the important one: the renderer's heartbeat dies with the window, so
 * without this the core would believe the last claim for a further `FOCUS_STALE_SECS`.
 *
 * There is no `onFocus`. The shell does not know which view will be on screen when the window
 * comes back, and the renderer re-reports on its own.
 */
export function registerFocusRelease(deps: WindowFocusDeps): void {
  const release = (): void => {
    void releaseFocus(deps);
  };
  deps.onBlur(release);
  deps.onHide(release);
  deps.onDestroyed(release);
}

export async function releaseFocus(
  deps: Pick<WindowFocusDeps, 'release' | 'onError'>,
): Promise<void> {
  try {
    await deps.release(focusArgs(null));
  } catch (error: unknown) {
    // The core may already be gone at destroy time. An unhandled rejection inside a window
    // event handler turns a clean shutdown into a crash report.
    deps.onError(error instanceof Error ? error.message : String(error));
  }
}
