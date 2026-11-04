import * as path from 'node:path';

import { _electron as electron, type ElectronApplication } from '@playwright/test';

type LaunchOptions = NonNullable<Parameters<typeof electron.launch>[0]>;

/**
 * Electron's arguments that preload `offscreenWindow.cjs` into the main process: every window
 * opens off every display, with no taskbar button, and is never activated. Playwright transpiles
 * specs to CommonJS, so `__dirname` is correct here.
 */
export const OFFSCREEN_ARGS: readonly string[] = [
  '-r',
  path.join(__dirname, 'offscreenWindow.cjs'),
];

/** The one way an e2e spec starts Electron through Playwright: always off-screen. */
export function launchApp(options: LaunchOptions): Promise<ElectronApplication> {
  return electron.launch({ ...options, args: [...OFFSCREEN_ARGS, ...(options.args ?? [])] });
}
