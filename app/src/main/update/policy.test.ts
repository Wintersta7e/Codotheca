import { describe, expect, it } from 'vitest';
import { detectArtifactKind } from './policy';

const win = { isPackaged: true, platform: 'win32' as const, env: {} };
const linux = (env: NodeJS.ProcessEnv): Parameters<typeof detectArtifactKind>[0] => ({
  isPackaged: true,
  platform: 'linux',
  env,
});

describe('detectArtifactKind', () => {
  it('is unpackaged in a development tree', () => {
    expect(detectArtifactKind({ ...win, isPackaged: false })).toBe('unpackaged');
  });

  it('is nsis on packaged Windows', () => {
    expect(detectArtifactKind(win)).toBe('nsis');
  });

  it('is portable only when PORTABLE_EXECUTABLE_FILE names a file', () => {
    // electron-builder's portable target sets this for the running process, and nothing else
    // does. It is the exact Windows parallel of APPIMAGE, and for the same reason: one pack
    // produces both artifacts from one directory, so no build-time value can tell them apart.
    expect(
      detectArtifactKind({ ...win, env: { PORTABLE_EXECUTABLE_FILE: 'D:\\Codotheca.exe' } }),
    ).toBe('portable');
    expect(detectArtifactKind({ ...win, env: { PORTABLE_EXECUTABLE_FILE: '' } })).toBe('nsis');
  });

  it('is appimage only when APPIMAGE names a file', () => {
    expect(detectArtifactKind(linux({ APPIMAGE: '/opt/Codotheca.AppImage' }))).toBe('appimage');
  });

  it('is a system package when APPIMAGE is absent or empty', () => {
    expect(detectArtifactKind(linux({}))).toBe('system-package');
    expect(detectArtifactKind(linux({ APPIMAGE: '' }))).toBe('system-package');
  });

  it('is unsupported on a platform this build does not target', () => {
    expect(detectArtifactKind({ ...win, platform: 'darwin' })).toBe('unsupported');
  });

  it('never reports an artifact kind for an unpackaged tree, whatever the environment', () => {
    // The development tree is checked first on purpose: `npm run dev` on a machine that has
    // ever run an AppImage inherits APPIMAGE from the parent shell, and reporting that as an
    // installed AppImage would put a wrong artifact name in every diagnostics bundle.
    expect(
      detectArtifactKind({ isPackaged: false, platform: 'linux', env: { APPIMAGE: '/opt/x' } }),
    ).toBe('unpackaged');
  });
});
