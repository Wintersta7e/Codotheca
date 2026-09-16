/**
 * The renderer's Content-Security-Policy.
 *
 * `NO ACCOUNT · NO TELEMETRY` is rendered in the footer (§8.7), so this policy naming a
 * remote origin would make the product lie. `font-src 'self'` is the clause §8.7 calls out by
 * name: the three families ship as WOFF2 inside the bundle, and a font request is a request —
 * it carries an IP address and a User-Agent to a third party on every cold start.
 */

/** Every source expression this policy is allowed to name. Anything else is a remote origin. */
const ALLOWED_SOURCES: ReadonlySet<string> = new Set([
  "'none'",
  "'self'",
  "'unsafe-inline'",
  'data:',
  'blob:',
  'codotheca:',
]);

const LOOPBACK_HOSTNAMES: ReadonlySet<string> = new Set(['localhost', '127.0.0.1', '[::1]']);

/**
 * `style-src` keeps `'unsafe-inline'` because the visual language is built from inline style
 * attributes and CSP treats those as inline styles. It admits no origin, so it cannot fetch.
 * `connect-src` names only the art scheme: §7.2 declares `supportFetchAPI`, and the shell↔core
 * conversation is IPC, not HTTP.
 */
export const CONTENT_SECURITY_POLICY = [
  "default-src 'none'",
  "script-src 'self'",
  "style-src 'self' 'unsafe-inline'",
  "font-src 'self'",
  "img-src 'self' codotheca: data:",
  'connect-src codotheca:',
  "worker-src 'self'",
  "media-src 'none'",
  "object-src 'none'",
  // [p2] §25.5's whole CSP delta, and it is one directive. `about:srcdoc`'s treatment under
  // `frame-src 'none'` is implementation-defined and has moved between Chromium versions, and a
  // README panel that silently renders nothing after an Electron upgrade is the worst failure
  // mode available. `'self'` is in ALLOWED_SOURCES, so `remoteOriginsIn` still returns [] and the
  // test that proves this policy names no remote origin keeps its exact meaning.
  "frame-src 'self'",
  "child-src 'none'",
  "manifest-src 'none'",
  "base-uri 'none'",
  "form-action 'none'",
].join('; ');

/**
 * Every source expression in `policy` that is not in the allowlist above.
 *
 * Empty is the only acceptable result for {@link CONTENT_SECURITY_POLICY}. It is *not* the
 * expected result for a development policy, which by construction names the loopback dev
 * server.
 */
export function remoteOriginsIn(policy: string): string[] {
  const offenders: string[] = [];
  for (const directive of policy.split(';')) {
    const tokens = directive
      .trim()
      .split(/\s+/u)
      .filter((token) => token.length > 0);
    for (const source of tokens.slice(1)) {
      if (!ALLOWED_SOURCES.has(source)) {
        offenders.push(source);
      }
    }
  }
  return offenders;
}

/**
 * The production policy widened by exactly one origin — the Vite dev server — so that module
 * loading, the HMR socket and React Refresh's inline preamble work while `electron-vite dev`
 * is running. It throws rather than widen to anything that is not loopback, so the dev path
 * cannot become a hole in the production guarantee by configuration.
 */
export function developmentContentSecurityPolicy(devServerOrigin: string): string {
  const url = new URL(devServerOrigin);
  if (!LOOPBACK_HOSTNAMES.has(url.hostname)) {
    throw new Error(`refusing a non-loopback development origin: ${devServerOrigin}`);
  }
  const origin = url.origin;
  const socket = `ws://${url.host}`;
  return CONTENT_SECURITY_POLICY.replace(
    "script-src 'self'",
    `script-src 'self' 'unsafe-inline' ${origin}`,
  )
    .replace("style-src 'self' 'unsafe-inline'", `style-src 'self' 'unsafe-inline' ${origin}`)
    .replace("font-src 'self'", `font-src 'self' ${origin}`)
    .replace("img-src 'self' codotheca: data:", `img-src 'self' codotheca: data: ${origin}`)
    .replace('connect-src codotheca:', `connect-src codotheca: ${origin} ${socket}`);
}
