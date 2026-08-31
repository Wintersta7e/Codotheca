/**
 * The Electron half. **R10**: `ResidentShortcut` is this plan's, because §8.6 specifies the
 * resident show shortcut and §11.5's surfaces defer to §8.6; the rival class plan 17 declared is
 * deleted and its settings drawer rebinds through this one. The pure half is re-exported here so
 * no caller has two import paths to choose between.
 */
export {
  SYSTEM_MENU_CONSEQUENCE,
  chordProposalWarning,
  isSystemWindowMenuChord,
  recordChord,
  residentShortcutStatusText,
  type ChordDescriptor,
} from '../shared/chord.js';

export interface ShortcutHost {
  readonly register: (chord: string, cb: () => void) => boolean;
  readonly unregister: (chord: string) => void;
  readonly isRegistered: (chord: string) => boolean;
}

export type BindOutcome =
  | { readonly kind: 'unbound' }
  | { readonly kind: 'bound'; readonly chord: string }
  | { readonly kind: 'refused'; readonly chord: string };

export class ResidentShortcut {
  #outcome: BindOutcome = { kind: 'unbound' };

  constructor(
    private readonly host: ShortcutHost,
    private readonly onPress: () => void,
  ) {}

  get outcome(): BindOutcome {
    return this.#outcome;
  }

  bind(chord: string | null): BindOutcome {
    const held = this.#outcome;
    if (held.kind === 'bound') this.host.unregister(held.chord);
    if (chord === null || chord === '') {
      this.#outcome = { kind: 'unbound' };
      return this.#outcome;
    }
    const ok = this.host.register(chord, this.onPress);
    this.#outcome = ok ? { kind: 'bound', chord } : { kind: 'refused', chord };
    return this.#outcome;
  }

  dispose(): void {
    this.bind(null);
  }
}

/**
 * §8.8: `Ctrl+S` saves a collection, and the shell drops the default accelerator — Chromium's own
 * `Ctrl+S` opens a Save-Page dialog, which is a filesystem dialog the renderer did not ask for and
 * §2.4's trust rule does not admit. This app draws its own chrome and needs no menu.
 */
export function disableDefaultApplicationMenu(setApplicationMenu: (menu: null) => void): void {
  setApplicationMenu(null);
}
