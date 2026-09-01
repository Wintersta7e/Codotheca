// §11.3's hybrid. The window survives half an hour of disuse and is then destroyed; §0's tray
// keeps the app alive with zero timers, zero frames and zero scheduled callbacks in the
// renderer, because the renderer is gone.
//
// Measured, not estimated: 307 MB resident with a live window and no textures, 522 MB with a
// full shelf, 232 MB with the window destroyed; 9 ms hotkey-to-visible alive, 134 ms
// destroyed. The hybrid buys the 9 ms while presses cluster and the 232 MB when they stop,
// rather than paying ~290 MB around the clock for a 125 ms saving.
import type { CommandName, ScanStatus } from '../generated/protocol';

export const RESIDENT_WINDOW_TTL_MS = 1_800_000;
export const HIDDEN_RESCAN_POLL_MS = 1_800_000;
/** §10.6: a full walk runs on launch only if the last scan is older than a day. */
export const RESCAN_DUE_AFTER_MS = 86_400_000;

export interface ResidencyDeps {
  hideWindow(): void;
  destroyWindow(): void;
  showWindow(): void;
  isWindowAlive(): boolean;
  setTimer(fn: () => void, ms: number): () => void;
  now(): number;
  request(name: CommandName, args: unknown): Promise<unknown>;
}

/**
 * `endedAt` is epoch **seconds** (the wire's only time unit) and `nowMs` is a millisecond
 * clock, so the conversion is load-bearing: compared raw, every scan looks a moment old and
 * the hidden rescan never fires.
 *
 * A run that has never ended is due — that is "not scanned", not "scanned at time zero".
 */
export function rescanIsDue(status: ScanStatus, nowMs: number): boolean {
  if (status.running) return false;
  if (status.endedAt === null) return true;
  return nowMs - status.endedAt * 1000 > RESCAN_DUE_AFTER_MS;
}

export class Residency {
  private cancelDestroy: (() => void) | null = null;
  private cancelPoll: (() => void) | null = null;

  constructor(private readonly deps: ResidencyDeps) {}

  get timers(): number {
    return (this.cancelDestroy === null ? 0 : 1) + (this.cancelPoll === null ? 0 : 1);
  }

  /** Closing the window hides it (§0); it does not quit the app. */
  onCloseRequested(): void {
    this.deps.hideWindow();
    this.cancelDestroy?.();
    this.cancelDestroy = this.deps.setTimer(() => {
      this.cancelDestroy = null;
      this.deps.destroyWindow();
      this.startHiddenPoll();
    }, RESIDENT_WINDOW_TTL_MS);
  }

  onShown(): void {
    this.cancelDestroy?.();
    this.cancelDestroy = null;
    this.cancelPoll?.();
    this.cancelPoll = null;
  }

  onQuit(): void {
    this.cancelDestroy?.();
    this.cancelPoll?.();
    this.cancelDestroy = null;
    this.cancelPoll = null;
  }

  /** The one timer the shell keeps while tray-hidden. */
  private startHiddenPoll(): void {
    this.cancelPoll?.();
    this.cancelPoll = this.deps.setTimer(() => {
      void this.deps
        .request('scan.status', {})
        .then((value) => {
          const status = value as ScanStatus;
          if (rescanIsDue(status, this.deps.now())) {
            return this.deps.request('scan.start', { full: false });
          }
          return null;
        })
        .finally(() => {
          this.cancelPoll = null;
          if (!this.deps.isWindowAlive()) this.startHiddenPoll();
        });
    }, HIDDEN_RESCAN_POLL_MS);
  }
}

export interface TrayDeps {
  onActivate(): void;
  onQuit(): void;
}

/**
 * §8.6: tray activation shows the shelf and opens nothing — the palette opens only for the
 * resident show shortcut, so the tray path never sends on `IPC_OPEN_PALETTE`.
 *
 * §17: no destructive item. QUIT ends the process and touches nothing on disk.
 */
export function trayMenu(
  deps: TrayDeps,
): readonly { readonly label: string; readonly click: () => void }[] {
  return [
    { label: 'SHOW CODOTHECA', click: (): void => deps.onActivate() },
    { label: 'QUIT', click: (): void => deps.onQuit() },
  ];
}
