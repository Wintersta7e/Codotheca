import type { Settings } from '../generated/protocol';

/**
 * Everything this process does with a stored setting, after the core has stored it.
 *
 * The drawer writes through `settings.set`, which reaches the core and nothing else — so a chord
 * would be stored and never registered, and a tier stored and never mirrored into `boot.json`.
 * `settings.set` answers with the whole `Settings`, so every listener acts on the value the core
 * kept rather than on the patch the renderer hoped for. One wrapper owns that, because two
 * copies of it had already drifted on what a missing answer means.
 *
 * An answer that is not a `Settings` stored nothing, so no listener hears it; it passes through
 * to the caller unchanged either way.
 */
export function onStoredSettings<N extends string, A, R>(
  request: (name: N, args: A) => Promise<R>,
  ...listeners: readonly ((settings: Settings) => unknown)[]
): (name: N, args: A) => Promise<R> {
  return async (name, args) => {
    const value = await request(name, args);
    const stored: unknown = value;
    if ((name as string) === 'settings.set' && stored !== null && typeof stored === 'object') {
      for (const listen of listeners) listen(stored as Settings);
    }
    return value;
  };
}
