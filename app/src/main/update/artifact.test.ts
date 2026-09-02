import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import { formatArtifactStamp, readArtifactStamp, readArtifactVersion } from './artifact';

const appPath = '/opt/app.asar';

const reader =
  (contents: Record<string, string>) =>
  (path: string): string => {
    const found = contents[path];
    if (found === undefined) throw new Error(`ENOENT: ${path}`);
    return found;
  };

const manifest = (body: string): ((path: string) => string) =>
  reader({ [join(appPath, 'package.json')]: body });

describe('readArtifactVersion', () => {
  it('reads the version out of the packaged manifest', () => {
    const readTextFile = manifest(JSON.stringify({ version: '1.2.3' }));
    expect(readArtifactVersion({ appPath, readTextFile })).toBe('1.2.3');
  });

  it('is null when the manifest is missing, unreadable or not JSON', () => {
    expect(readArtifactVersion({ appPath, readTextFile: reader({}) })).toBeNull();
    expect(readArtifactVersion({ appPath, readTextFile: manifest('not json') })).toBeNull();
  });

  it('is null when the manifest is JSON but not an object', () => {
    expect(readArtifactVersion({ appPath, readTextFile: manifest('"1.2.3"') })).toBeNull();
  });

  it('is null for an absent or empty version rather than a plausible default', () => {
    expect(readArtifactVersion({ appPath, readTextFile: manifest('{}') })).toBeNull();
    expect(
      readArtifactVersion({ appPath, readTextFile: manifest(JSON.stringify({ version: '' })) }),
    ).toBeNull();
    expect(
      readArtifactVersion({ appPath, readTextFile: manifest(JSON.stringify({ version: 3 })) }),
    ).toBeNull();
  });
});

describe('readArtifactStamp', () => {
  it('pairs the packed version with the artifact observed at run time', () => {
    expect(
      readArtifactStamp({
        appPath,
        readTextFile: manifest(JSON.stringify({ version: '1.2.3' })),
        artifact: { isPackaged: true, platform: 'linux', env: { APPIMAGE: '/opt/x.AppImage' } },
      }),
    ).toEqual({ version: '1.2.3', kind: 'appimage' });
  });
});

describe('formatArtifactStamp', () => {
  it('names an unknown version rather than printing one', () => {
    const line = formatArtifactStamp({ version: null, kind: 'unpackaged' });
    expect(line).toContain('unpackaged');
    expect(line).not.toContain('null');
    expect(line).not.toMatch(/\d/u);
  });

  it('carries both halves when both are known', () => {
    expect(formatArtifactStamp({ version: '1.2.3', kind: 'portable' })).toContain('1.2.3');
    expect(formatArtifactStamp({ version: '1.2.3', kind: 'portable' })).toContain('portable');
  });
});
