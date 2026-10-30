import * as path from 'node:path';

/**
 * Every variable the app finds a user's files through, pointed into `home`.
 *
 * `HOME` alone is not enough: the core suggests first-run roots from editor history under
 * `APPDATA` and `XDG_CONFIG_HOME`, and DIG scans every suggested root. On a workstation a launch
 * that kept the developer's `APPDATA` scanned the developer's own repositories, and every card
 * count in the suite was off by them. A hosted runner has no editor history, so only a local run
 * could see it.
 */
export function homeEnv(home: string): Record<string, string> {
  return {
    HOME: home,
    USERPROFILE: home,
    APPDATA: path.join(home, 'AppData', 'Roaming'),
    XDG_CONFIG_HOME: path.join(home, '.config'),
  };
}

/**
 * The environment a fixture's own `git` runs in: no global or system config, so a developer's
 * hooks and settings cannot decide whether the fixture repository builds.
 */
export const FIXTURE_GIT_ENV: Readonly<Record<string, string>> = {
  GIT_CONFIG_GLOBAL: process.platform === 'win32' ? 'NUL' : '/dev/null',
  GIT_CONFIG_NOSYSTEM: '1',
};
