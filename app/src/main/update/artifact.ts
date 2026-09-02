/**
 * §14's build-time artifact flag, read back at run time — the half of it that survives having
 * no update channel.
 *
 * §11.4's bundle has to name the build it came from, and neither half of that name can be a
 * compile-time constant. The version is written into the packaged manifest during pack, which
 * is *after* the bundler has run, so importing the manifest would inline a value from the
 * source tree and the stamp would never be seen; it is read from `app.getAppPath()` instead.
 * The artifact kind is a runtime observation for the reason `policy.ts` gives.
 *
 * Every failure path returns null rather than a plausible-looking default. A bundle that says
 * `0.0.0` when it does not know the version is worse than one that says it does not know:
 * unknown is never rendered as a value.
 */
import { join } from 'node:path';
import { type ArtifactInputs, type ArtifactKind, detectArtifactKind } from './policy';

export const ARTIFACT_MANIFEST_NAME = 'package.json';

export interface ArtifactManifestInputs {
  /** Electron's `app.getAppPath()` — the asar root in a packaged app. */
  readonly appPath: string;
  /** `fs.readFileSync(path, 'utf8')`, injected so this is testable without a filesystem. */
  readonly readTextFile: (path: string) => string;
}

export function readArtifactVersion(inputs: ArtifactManifestInputs): string | null {
  let parsed: unknown;
  try {
    parsed = JSON.parse(inputs.readTextFile(join(inputs.appPath, ARTIFACT_MANIFEST_NAME)));
  } catch {
    return null;
  }
  if (typeof parsed !== 'object' || parsed === null) return null;
  const version = (parsed as Record<string, unknown>)['version'];
  return typeof version === 'string' && version.length > 0 ? version : null;
}

export interface ArtifactStamp {
  readonly version: string | null;
  readonly kind: ArtifactKind;
}

export function readArtifactStamp(
  inputs: ArtifactManifestInputs & { readonly artifact: ArtifactInputs },
): ArtifactStamp {
  return {
    version: readArtifactVersion(inputs),
    kind: detectArtifactKind(inputs.artifact),
  };
}

/** One line for the rolling log, which is what a diagnostics bundle collects. */
export function formatArtifactStamp(stamp: ArtifactStamp): string {
  return `${stamp.version ?? 'version not recorded'} · ${stamp.kind}`;
}
