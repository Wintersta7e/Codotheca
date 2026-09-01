import { type Mock, describe, expect, it, vi } from 'vitest';
import { mirrorOnJoin, mirrorShelf, noteForcedOff } from '../src/main/bootMirror';
import type { BootFile } from '../src/shared/bootFile';
import type { Settings } from '../src/generated/protocol';

function deps(stored: BootFile): {
  deps: { dataDir: string; readBoot: () => BootFile; writeBoot: Mock };
  writeBoot: Mock;
} {
  const writeBoot = vi.fn();
  return { deps: { dataDir: '/data', readBoot: () => stored, writeBoot }, writeBoot };
}

const settings: Settings = {
  effectsTier: 'reduced',
  reducedMotionOverride: false,
  autostart: false,
  residentShortcut: null,
  roastEnabled: true,
  logLevel: 'info',
};

const stored: BootFile = {
  generation: 2,
  effectsTier: 'full',
  paintFailCount: 0,
  paintFailForcedAt: null,
  shelfProjection: null,
};

describe('the boot mirror', () => {
  it('rewrites the file when the core disagrees with it', () => {
    const { deps: d, writeBoot } = deps(stored);
    const next = mirrorOnJoin(d, settings);
    expect(next.effectsTier).toBe('reduced');
    expect(writeBoot).toHaveBeenCalledTimes(1);
  });

  it('writes nothing when the file already agrees', () => {
    const { deps: d, writeBoot } = deps({ ...stored, effectsTier: 'reduced' });
    mirrorOnJoin(d, settings);
    expect(writeBoot).not.toHaveBeenCalled();
  });

  it('mirrors the shelf projection without disturbing the tier', () => {
    const { deps: d, writeBoot } = deps(stored);
    mirrorShelf(d, { rows: [] });
    const written = writeBoot.mock.calls[0]?.[1] as BootFile;
    expect(written.shelfProjection).toEqual({ rows: [] });
    expect(written.effectsTier).toBe('full');
  });

  it('records which launch forced the tier off so settings can name it', () => {
    const { deps: d, writeBoot } = deps({ ...stored, paintFailCount: 2 });
    noteForcedOff(d, 1_700_000_000_000);
    const written = writeBoot.mock.calls[0]?.[1] as BootFile;
    expect(written.effectsTier).toBe('off');
    expect(written.paintFailForcedAt).toBe(1_700_000_000_000);
  });

  it('the database stays authoritative — the join never writes the file back to the core', () => {
    // §11.2a: `boot.json` is a mirror. If the mirror won, a tier forced off by a broken GPU
    // would overwrite the setting the user chose, and settings would show a value nothing set.
    const { deps: d } = deps({ ...stored, effectsTier: 'off' });
    expect(mirrorOnJoin(d, settings).effectsTier).toBe('reduced');
  });

  it('a mirrored shelf keeps the forcing record the drawer reads', () => {
    const { deps: d, writeBoot } = deps({ ...stored, paintFailForcedAt: 42 });
    mirrorShelf(d, { rows: [1] });
    expect((writeBoot.mock.calls[0]?.[1] as BootFile).paintFailForcedAt).toBe(42);
  });
});
