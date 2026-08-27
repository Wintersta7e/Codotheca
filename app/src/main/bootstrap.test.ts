import { describe, expect, it, vi } from 'vitest';
import { BOOT_FILE_GENERATION, type BootFile } from '../shared/bootFile';
import { type BootstrapDeps, bootstrap, clearPaintFailure } from './bootstrap';

function stored(overrides: Partial<BootFile> = {}): BootFile {
  return {
    generation: BOOT_FILE_GENERATION,
    effectsTier: 'full',
    paintFailCount: 0,
    shelfProjection: null,
    ...overrides,
  };
}

function deps(overrides: Partial<BootstrapDeps> = {}): BootstrapDeps {
  return {
    argv: [],
    env: {},
    userDataDir: '/data',
    registerSchemesAsPrivileged: vi.fn(),
    disableHardwareAcceleration: vi.fn(),
    readBoot: () => stored(),
    writeBoot: () => undefined,
    ...overrides,
  };
}

describe('bootstrap', () => {
  it('registers the art scheme first of all', () => {
    // Electron throws "registerSchemesAsPrivileged should be called before app is ready" if
    // this call slips behind anything that awaits readiness — on a user's machine, not here.
    const order: string[] = [];
    bootstrap(
      deps({
        registerSchemesAsPrivileged: () => order.push('registerSchemesAsPrivileged'),
        readBoot: () => {
          order.push('readBoot');
          return stored();
        },
        writeBoot: () => order.push('writeBoot'),
      }),
    );
    expect(order).toEqual(['registerSchemesAsPrivileged', 'readBoot', 'writeBoot']);
  });

  it('hands Electron exactly one custom scheme', () => {
    const register = vi.fn();
    bootstrap(deps({ registerSchemesAsPrivileged: register }));
    expect(register).toHaveBeenCalledTimes(1);
    const [schemes] = register.mock.calls[0] as [{ scheme: string }[]];
    expect(schemes.map((s) => s.scheme)).toEqual(['codotheca']);
  });

  it('disables hardware acceleration when the resolved tier is off', () => {
    // The GPU is what may be broken, so the recovery path must not use it.
    const disable = vi.fn();
    const result = bootstrap(
      deps({ argv: ['app', '--effects-tier=off'], disableHardwareAcceleration: disable }),
    );
    expect(disable).toHaveBeenCalledTimes(1);
    expect(result).toMatchObject({ tier: 'off', source: 'argv' });
  });

  it('leaves hardware acceleration alone at every other tier', () => {
    const disable = vi.fn();
    bootstrap(
      deps({ argv: ['app', '--effects-tier=reduced'], disableHardwareAcceleration: disable }),
    );
    expect(disable).not.toHaveBeenCalled();
  });

  it('increments the paint-failure counter before the window is created', () => {
    const writeBoot = vi.fn();
    bootstrap(deps({ readBoot: () => stored({ paintFailCount: 1 }), writeBoot }));
    expect(writeBoot).toHaveBeenCalledWith('/data', expect.objectContaining({ paintFailCount: 2 }));
  });

  it('does not write an override back to the file', () => {
    // §11.2a: "Neither is written back."
    const writeBoot = vi.fn();
    bootstrap(deps({ argv: ['app', '--effects-tier=off'], writeBoot }));
    expect(writeBoot).toHaveBeenCalledWith(
      '/data',
      expect.objectContaining({ effectsTier: 'full' }),
    );
  });
});

describe('clearPaintFailure', () => {
  it('zeroes the counter and preserves everything else', () => {
    const writeBoot = vi.fn();
    clearPaintFailure({
      userDataDir: '/data',
      readBoot: () => stored({ paintFailCount: 3, effectsTier: 'reduced' }),
      writeBoot,
    });
    expect(writeBoot).toHaveBeenCalledWith('/data', {
      generation: BOOT_FILE_GENERATION,
      effectsTier: 'reduced',
      paintFailCount: 0,
      shelfProjection: null,
    });
  });
});
