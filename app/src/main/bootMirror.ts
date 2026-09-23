// §11.2a: the database stays authoritative and `boot.json` is a mirror. On join the core's
// value wins and the file is rewritten if it differs; nothing reads the file after the join.
//
// The direction matters. If the mirror won, a tier forced off by a broken GPU would overwrite
// the setting the user chose, and the drawer would show a value nothing set.
import type { BootFile } from '../shared/bootFile';
import type { Settings } from '../generated/protocol';

export interface BootMirrorDeps {
  dataDir: string;
  readBoot(dataDir: string): BootFile;
  writeBoot(dataDir: string, file: BootFile): void;
}

/**
 * Returns the file as it now stands, written only if the core disagreed with it. The tier and the
 * reduced-motion override travel together: the override clamps the tier, so a first frame that
 * had one without the other would run motion the user turned down.
 */
export function mirrorOnJoin(deps: BootMirrorDeps, settings: Settings): BootFile {
  const stored = deps.readBoot(deps.dataDir);
  if (
    stored.effectsTier === settings.effectsTier &&
    stored.reducedMotionOverride === settings.reducedMotionOverride
  ) {
    return stored;
  }
  const next: BootFile = {
    ...stored,
    effectsTier: settings.effectsTier,
    reducedMotionOverride: settings.reducedMotionOverride,
  };
  deps.writeBoot(deps.dataDir, next);
  return next;
}

/**
 * The one mirror the join and the write path share. It never throws: the core has already stored
 * the value by the time the file is touched, and a failed write rejected here would read as a
 * failed setting.
 *
 * It remembers the pair it last left in the file, so an answer that moves neither — a check
 * switch, the roast, the shortcut — costs no synchronous read of it. Nothing else in this process
 * writes either field after launch, so the memory cannot go stale.
 */
export function tierMirror(
  deps: BootMirrorDeps,
  onError: (error: unknown) => void,
): (settings: Settings) => void {
  let mirrored: Pick<BootFile, 'effectsTier' | 'reducedMotionOverride'> | null = null;
  return (settings) => {
    if (
      mirrored?.effectsTier === settings.effectsTier &&
      mirrored.reducedMotionOverride === settings.reducedMotionOverride
    ) {
      return;
    }
    try {
      const file = mirrorOnJoin(deps, settings);
      mirrored = {
        effectsTier: file.effectsTier,
        reducedMotionOverride: file.reducedMotionOverride,
      };
    } catch (error: unknown) {
      onError(error);
    }
  };
}

/**
 * §11.2's UI lane paints from this, because §2.3's snapshot is a backpressure frame inside a
 * live subscription and at first paint there is no subscription.
 */
export function mirrorShelf(deps: BootMirrorDeps, projection: unknown): void {
  const stored = deps.readBoot(deps.dataDir);
  deps.writeBoot(deps.dataDir, { ...stored, shelfProjection: projection });
}

/**
 * §11.2a: at two paint failures the tier is forced off, and settings states which launch
 * forced it and offers to clear it.
 */
export function noteForcedOff(deps: BootMirrorDeps, atMs: number): void {
  const stored = deps.readBoot(deps.dataDir);
  deps.writeBoot(deps.dataDir, { ...stored, effectsTier: 'off', paintFailForcedAt: atMs });
}
