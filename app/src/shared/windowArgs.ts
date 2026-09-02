/**
 * Facts the shell knows before the window exists and the renderer needs on its first frame.
 *
 * Same carrier and same reason as `effectsTier.ts`: §11.2 paints before the core joins, so a
 * value the first frame needs cannot come from a round trip. The value is **percent-encoded**,
 * which the tier flags do not need to be — a tier is one lowercase word, and a log path is a
 * user profile path that routinely contains spaces. Chromium re-parses a command line for the
 * sandboxed renderer, where an unencoded space is the difference between one argument and two.
 */

/** §11.2a: every failure window names the log. Display only — the renderer cannot open it. */
export const LOG_PATH_FLAG = '--log-path=';

export function logPathArgument(path: string): string {
  return `${LOG_PATH_FLAG}${encodeURIComponent(path)}`;
}

/**
 * The last decodable `--log-path=` in `argv`, or the empty string.
 *
 * Empty is "the shell passed none", which is the honest reading in a window created without it
 * — under test, and in any renderer the flag never reached. The failure windows draw no note
 * for an empty path rather than a note about nothing.
 */
export function logPathFromArgv(argv: readonly string[]): string {
  let found = '';
  for (const argument of argv) {
    if (!argument.startsWith(LOG_PATH_FLAG)) continue;
    try {
      found = decodeURIComponent(argument.slice(LOG_PATH_FLAG.length));
    } catch {
      found = '';
    }
  }
  return found;
}
