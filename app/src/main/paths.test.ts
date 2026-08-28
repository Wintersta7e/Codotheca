import * as path from 'node:path';
import { describe, expect, it } from 'vitest';
import { resolveCoreBinary, resolveDataDir } from './paths';

describe('paths', () => {
  it('lets the shell decide the data directory, once', () => {
    expect(resolveDataDir({ userDataPath: '/u/data', env: {} })).toBe('/u/data');
  });

  it('lets an explicit override win, so tests never touch the real directory', () => {
    const env = { CODOTHECA_DATA_DIR: '/tmp/fixture' };
    expect(resolveDataDir({ userDataPath: '/u/data', env })).toBe('/tmp/fixture');
  });

  it('does not treat an empty override as an override', () => {
    expect(resolveDataDir({ userDataPath: '/u/data', env: { CODOTHECA_DATA_DIR: '' } })).toBe(
      '/u/data',
    );
  });

  it('puts the packaged binary under resources and the dev one under the target dir', () => {
    expect(
      resolveCoreBinary({
        isPackaged: true,
        resourcesPath: '/app/res',
        appRoot: '/src',
        platform: 'linux',
      }),
    ).toBe(path.join('/app/res', 'core', 'codotheca-core'));
    expect(
      resolveCoreBinary({
        isPackaged: false,
        resourcesPath: '/app/res',
        appRoot: '/src',
        platform: 'win32',
      }),
    ).toBe(path.join('/src', 'core', 'target', 'release', 'codotheca-core.exe'));
  });
});
