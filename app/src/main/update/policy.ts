/**
 * Which artifact is executing.
 *
 * There is no auto-update: distribution is a releases page the user downloads from by hand, so
 * §14's rule that an installed system package must never rewrite itself holds by construction.
 * What survives is the question the diagnostics bundle asks — *which* of the five artifacts is
 * running — and that cannot be answered at build time. One `electron-builder --linux` run packs
 * one directory and emits an AppImage, a `.deb` and an `.rpm` from it; one `--win` run emits an
 * NSIS installer and a portable `.exe`. A build-time value is identical in all of them.
 *
 * Two environment variables are the only honest signals. The AppImage runtime sets `APPIMAGE`
 * and nothing else does; electron-builder's portable target sets `PORTABLE_EXECUTABLE_FILE` for
 * the running process and nothing else does. They are the same mechanism on the two platforms.
 *
 * This module is pure: no `fs`, no `electron`.
 */
export type ArtifactKind =
  'nsis' | 'portable' | 'appimage' | 'system-package' | 'unpackaged' | 'unsupported';

export interface ArtifactInputs {
  /** Electron's `app.isPackaged`. */
  readonly isPackaged: boolean;
  readonly platform: NodeJS.Platform;
  readonly env: NodeJS.ProcessEnv;
}

/** An unset variable and one set to the empty string mean the same thing: not that artifact. */
function names(value: string | undefined): boolean {
  return value !== undefined && value.length > 0;
}

export function detectArtifactKind(inputs: ArtifactInputs): ArtifactKind {
  if (!inputs.isPackaged) return 'unpackaged';
  if (inputs.platform === 'win32') {
    return names(inputs.env['PORTABLE_EXECUTABLE_FILE']) ? 'portable' : 'nsis';
  }
  if (inputs.platform !== 'linux') return 'unsupported';
  return names(inputs.env['APPIMAGE']) ? 'appimage' : 'system-package';
}
