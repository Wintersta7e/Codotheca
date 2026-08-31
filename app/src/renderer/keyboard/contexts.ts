/**
 * §11.7. **Each context owns its keys exclusively**: the shelf's grid handler must not run on
 * the project page. This is a pure function of `(context, event)` and returns `null` for a key
 * the context does not own, so a caller has nothing to fall through to and no shared handler
 * exists to check a variable.
 */
export type KeyContext = 'shelf' | 'projectPage' | 'palette' | 'settings';

export type KeyAction =
  | 'quickSwitch'
  | 'shelf.moveUp'
  | 'shelf.moveDown'
  | 'shelf.moveLeft'
  | 'shelf.moveRight'
  | 'shelf.play'
  | 'shelf.openPage'
  | 'shelf.peek'
  | 'shelf.closePeek'
  | 'shelf.togglePin'
  | 'shelf.saveCollection'
  | 'page.prevTab'
  | 'page.nextTab'
  | 'page.back'
  | 'palette.moveUp'
  | 'palette.moveDown'
  | 'palette.launch'
  | 'palette.openPage'
  | 'palette.close'
  | 'settings.close';

export interface KeyEventLike {
  readonly key: string;
  readonly code: string;
  readonly altKey: boolean;
  readonly ctrlKey: boolean;
  readonly shiftKey: boolean;
  readonly metaKey: boolean;
  readonly target: { readonly tagName?: string; readonly isContentEditable?: boolean } | null;
}

export interface KeyResolution {
  readonly action: KeyAction;
  readonly preventDefault: boolean;
}

/**
 * Prose and any label naming a chord away from its own surface spells it out (§11.7): `⌥` and
 * `␣` are Mac keycaps and phase 1 targets Windows and Linux.
 */
export const QUICK_SWITCH_CHORD = 'Alt+Space';

export const CONTEXT_ACTIONS: Readonly<Record<KeyContext, readonly KeyAction[]>> = {
  shelf: [
    'quickSwitch',
    'shelf.moveUp',
    'shelf.moveDown',
    'shelf.moveLeft',
    'shelf.moveRight',
    'shelf.play',
    'shelf.openPage',
    'shelf.peek',
    'shelf.closePeek',
    'shelf.togglePin',
    'shelf.saveCollection',
  ],
  projectPage: ['quickSwitch', 'page.prevTab', 'page.nextTab', 'page.back'],
  palette: [
    'quickSwitch',
    'palette.moveUp',
    'palette.moveDown',
    'palette.launch',
    'palette.openPage',
    'palette.close',
  ],
  settings: ['quickSwitch', 'settings.close'],
};

/** The prototype's own guard — `t === "INPUT" || t === "TEXTAREA"` — is the rule, not an incidental. */
export function isTextEntry(target: KeyEventLike['target']): boolean {
  if (target === null) return false;
  if (target.isContentEditable === true) return true;
  const tag = target.tagName;
  return tag === 'INPUT' || tag === 'TEXTAREA';
}

const hit = (action: KeyAction, preventDefault = false): KeyResolution => ({
  action,
  preventDefault,
});

function shelfKey(event: KeyEventLike): KeyResolution | null {
  if (isTextEntry(event.target)) return null; // [R45] per-context guard
  if (event.ctrlKey) {
    // `Ctrl+S` is preventDefaulted over Chromium's Save-Page accelerator (§8.8).
    return event.code === 'KeyS' ? hit('shelf.saveCollection', true) : null;
  }
  switch (event.code) {
    case 'ArrowUp':
      return hit('shelf.moveUp', true);
    case 'ArrowDown':
      return hit('shelf.moveDown', true);
    case 'ArrowLeft':
      return hit('shelf.moveLeft', true);
    case 'ArrowRight':
      return hit('shelf.moveRight', true);
    case 'Enter':
      return hit(event.shiftKey ? 'shelf.openPage' : 'shelf.play');
    case 'Space':
      return hit('shelf.peek', true);
    case 'Escape':
      return hit('shelf.closePeek');
    // A bare letter is available because phase 1 has no type-ahead on the grid — searching is
    // the query field (§8.3). If type-ahead is ever added, this binding moves rather than fights.
    case 'KeyP':
      return hit('shelf.togglePin');
    default:
      return null;
  }
}

function projectPageKey(event: KeyEventLike): KeyResolution | null {
  if (isTextEntry(event.target)) return null; // [R45] per-context guard
  if (event.ctrlKey) return null;
  switch (event.code) {
    case 'ArrowLeft':
      return hit('page.prevTab', true);
    case 'ArrowRight':
      return hit('page.nextTab', true);
    case 'Escape':
      return hit('page.back');
    default:
      return null;
  }
}

function paletteKey(event: KeyEventLike): KeyResolution | null {
  if (event.ctrlKey) return null;
  switch (event.code) {
    case 'ArrowUp':
      return hit('palette.moveUp', true);
    case 'ArrowDown':
      return hit('palette.moveDown', true);
    case 'Enter':
    // [R45] The numpad's Enter is a different `code`. The resolver R42 deleted had it;
    // collapsing to one table would otherwise have silently dropped numpad launch.
    case 'NumpadEnter':
      return hit(event.shiftKey ? 'palette.openPage' : 'palette.launch');
    case 'Escape':
      return hit('palette.close');
    default:
      return null;
  }
}

function settingsKey(event: KeyEventLike): KeyResolution | null {
  if (isTextEntry(event.target)) return null; // [R45] per-context guard
  if (event.ctrlKey) return null;
  return event.code === 'Escape' ? hit('settings.close') : null;
}

export function resolveKey(context: KeyContext, event: KeyEventLike): KeyResolution | null {
  // No phase-1 binding carries Meta; a Meta chord belongs to the OS.
  if (event.metaKey) return null;

  // The one binding that crosses contexts, including while the query field has focus. Detection
  // is `code === "Space" && altKey`, preventDefaulted so the keystroke never reaches the field.
  if (event.altKey) {
    return event.code === 'Space' && !event.ctrlKey ? hit('quickSwitch', true) : null;
  }

  // [R45] The text-entry guard is PER CONTEXT, not global. Typing in the shelf's query field
  // must not move the grid — that is the criterion it exists for — but the palette's query bar
  // is an autoFocus'd <input role="combobox">, so a global guard leaves the open palette with
  // no working keys at all: no arrows, no Enter, no Escape. `paletteKey` therefore runs while a
  // text entry has focus; the other three contexts apply the guard themselves.

  switch (context) {
    case 'shelf':
      return shelfKey(event);
    case 'projectPage':
      return projectPageKey(event);
    case 'palette':
      return paletteKey(event);
    case 'settings':
      return settingsKey(event);
  }
}
