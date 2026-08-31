import { useCallback, useEffect, useReducer, useRef } from 'react';
import type { KeyboardEvent as ReactKeyboardEvent, ReactElement } from 'react';
import type { LocationId, ProjectId, ProjectRow } from '../../generated/protocol.js';
import type { CodothecaBridge } from '../../shared/bridge.js';
import type { EffectsTier } from '../../shared/effectsTier.js';
import type { KeyContext, KeyEventLike } from '../keyboard/contexts.js';
import { resolveKey } from '../keyboard/contexts.js';
import { QuickSwitch } from './QuickSwitch.js';
import { PALETTE_CLOSED, paletteIntent, paletteReducer } from './reducer.js';
import { paletteRowAction, selectPaletteRows } from './rows.js';

/**
 * R12/R42: `resolveKey` takes plan 12b's `KeyEventLike`, whose `target` is
 * `{ tagName?, isContentEditable? } | null` — a DOM `EventTarget` is not assignable to it, so the
 * narrowing happens once, here, rather than as a cast at the call site. The target is carried
 * rather than dropped: 12b's per-context text-entry guard (R45) reads it, and a host that always
 * reported `null` would be testing a shape the product never has.
 */
function keyEventLike(event: KeyboardEvent | ReactKeyboardEvent<HTMLElement>): KeyEventLike {
  const target: unknown = event.target;
  return {
    key: event.key,
    code: event.code,
    altKey: event.altKey,
    ctrlKey: event.ctrlKey,
    metaKey: event.metaKey,
    shiftKey: event.shiftKey,
    target:
      target instanceof HTMLElement
        ? { tagName: target.tagName, isContentEditable: target.isContentEditable }
        : null,
  };
}

export function subscribeShellOpenPalette(cb: () => void): () => void {
  // §2.4 makes the preload the only channel, and it is simply absent in a renderer the preload
  // never reached — under test, and in any window created without it. Absence is silence.
  const bridge = (globalThis as { codotheca?: Partial<CodothecaBridge> }).codotheca;
  bridge?.onOpenPalette?.(cb);
  return () => {
    /* the preload bridge registers for the window's lifetime; there is nothing to detach */
  };
}

export interface QuickSwitchHostProps {
  readonly rows: readonly ProjectRow[];
  readonly liveSessionProjectIds: ReadonlySet<ProjectId>;
  /** Already resolved by the renderer's tier resolver. */
  readonly effectsTier: EffectsTier;
  readonly jewelFor: (row: ProjectRow) => string;
  /** Epoch seconds. Injected so the sub-line's age is testable. */
  readonly now: () => number;
  readonly onLaunch: (projectId: ProjectId, locationId: LocationId) => void;
  readonly onOpenPage: (projectId: ProjectId) => void;
  readonly subscribeShellOpen?: (cb: () => void) => () => void;
  /**
   * R42: the context to resolve in while the palette is closed — the caller's own. `Alt+Space`
   * resolves to `quickSwitch` in every context, so this changes no answer the host acts on; it
   * exists because `resolveKey` is `(context, event)` and the context is the caller's to name.
   */
  readonly ambientContext?: KeyContext;
}

export function QuickSwitchHost(props: QuickSwitchHostProps): ReactElement | null {
  const [state, dispatch] = useReducer(paletteReducer, PALETTE_CLOSED);
  const restoreTo = useRef<HTMLElement | null>(null);
  const wasOpen = useRef(false);

  const query = state.open ? state.query : '';
  const cursor = state.open ? state.cursor : 0;
  const visible = selectPaletteRows(props.rows, query).rows;

  const close = useCallback(() => {
    dispatch({ type: 'close' });
  }, []);

  /**
   * §8.6: `Esc` closes and restores focus to whatever held it. The element is recorded **before**
   * the panel is asked to open, not in an effect afterwards: the query bar autofocuses during
   * commit, so by the time a passive effect runs `document.activeElement` is already the palette's
   * own input and the thing the user came from is lost.
   */
  const rememberFocus = useCallback(() => {
    restoreTo.current =
      document.activeElement instanceof HTMLElement ? document.activeElement : null;
  }, []);

  useEffect(() => {
    if (state.open) {
      wasOpen.current = true;
      return;
    }
    if (!wasOpen.current) return;
    wasOpen.current = false;
    const target = restoreTo.current;
    restoreTo.current = null;
    if (target !== null && target.isConnected) target.focus();
  }, [state.open]);

  const handle = useCallback(
    (event: KeyboardEvent | ReactKeyboardEvent<HTMLElement>, context: KeyContext): void => {
      // R42: one resolution, 12b's. The context is chosen by the caller below — `'palette'` when
      // the panel is up, the ambient one when it is not — which is what the deleted
      // `open: boolean` was standing in for.
      const resolved = resolveKey(context, keyEventLike(event));
      if (resolved === null) return;
      const action = paletteIntent(resolved.action);
      if (action === null) return;
      event.preventDefault();
      switch (action.kind) {
        case 'toggle':
          if (!state.open) rememberFocus();
          dispatch({ type: 'toggle' });
          return;
        case 'close':
          dispatch({ type: 'close' });
          return;
        case 'move':
          dispatch({ type: 'move', delta: action.delta, count: visible.length });
          return;
        case 'launch': {
          const row = visible[cursor];
          if (row === undefined) return;
          const target = paletteRowAction(row);
          if (target.kind !== 'launch') return;
          props.onLaunch(row.id, target.locationId);
          dispatch({ type: 'close' });
          return;
        }
        case 'openPage': {
          const row = visible[cursor];
          if (row === undefined) return;
          props.onOpenPage(row.id);
          dispatch({ type: 'close' });
        }
      }
    },
    [cursor, props, rememberFocus, state.open, visible],
  );

  // R42: the caller picks the context. Open → `'palette'`; closed → the ambient one, where the
  // only action this host maps is `quickSwitch`, and that one crosses every context.
  const ambient: KeyContext = props.ambientContext ?? 'shelf';

  // Capture, so a focused field never sees the chord (§8.6, criterion 32).
  useEffect(() => {
    const listener = (event: KeyboardEvent): void => {
      handle(event, state.open ? 'palette' : ambient);
    };
    window.addEventListener('keydown', listener, true);
    return () => {
      window.removeEventListener('keydown', listener, true);
    };
  }, [ambient, handle, state.open]);

  const subscribe = props.subscribeShellOpen ?? subscribeShellOpenPalette;
  useEffect(
    () =>
      subscribe(() => {
        rememberFocus();
        dispatch({ type: 'open' });
      }),
    [rememberFocus, subscribe],
  );

  if (!state.open) return null;

  return (
    <QuickSwitch
      rows={props.rows}
      query={state.query}
      cursor={state.cursor}
      liveSessionProjectIds={props.liveSessionProjectIds}
      nowSecs={props.now()}
      effectsTier={props.effectsTier}
      jewelFor={props.jewelFor}
      onQueryChange={(value) => {
        dispatch({ type: 'query', value });
      }}
      onPoint={(index) => {
        dispatch({ type: 'point', index });
      }}
      onLaunch={props.onLaunch}
      onOpenPage={props.onOpenPage}
      onClose={close}
      onKeyDown={(event) => {
        // The panel is up, so this one is unconditionally the palette context.
        handle(event, 'palette');
      }}
    />
  );
}
