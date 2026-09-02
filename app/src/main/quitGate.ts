/**
 * Electron's `before-quit` is synchronous; the work that has to happen before the process ends
 * is not. This gate prevents the first quit, runs an ordered list of asynchronous steps once,
 * and quits again.
 *
 * Re-entrancy is the whole difficulty: a user can press quit twice, and the gate's final quit
 * enters this listener again. While the gate is running every further quit is blocked; once it
 * is done every quit passes through.
 */
import type { RollingLog } from './core/log';

export interface QuitEvent {
  preventDefault(): void;
}

export interface QuitGateApp {
  on(event: 'before-quit', listener: (event: QuitEvent) => void): void;
  quit(): void;
}

export interface QuitStep {
  readonly name: string;
  /** Resolves with a short outcome word, which is what the log records. */
  readonly run: () => Promise<string>;
}

export interface QuitGateDeps {
  readonly app: QuitGateApp;
  readonly steps: readonly QuitStep[];
  readonly log: RollingLog;
}

export function installQuitGate(deps: QuitGateDeps): void {
  let state: 'idle' | 'running' | 'done' = 'idle';

  deps.app.on('before-quit', (event) => {
    if (state === 'done') return;
    event.preventDefault();
    if (state === 'running') return;
    state = 'running';
    void run();
  });

  async function run(): Promise<void> {
    for (const step of deps.steps) {
      try {
        const outcome = await step.run();
        deps.log.write('info', 'shell', `quit: ${step.name} ${outcome}`);
      } catch (error: unknown) {
        deps.log.write('error', 'shell', `quit: ${step.name} failed: ${String(error)}`);
      }
    }
    state = 'done';
    deps.app.quit();
  }
}
