import { useCallback, useEffect, useRef, useState, type ReactElement } from 'react';
import type { Collection, CollectionRefusal } from '../../generated/protocol.js';
import type { QueryEngine } from './engine.js';
import {
  REFUSAL_CONTROL_TEXT,
  SAVE_CONTROL_TEXT,
  proposedName,
  refuseSave,
  saveControlVisible,
} from './refusals.js';
import { nextSortIndex, useCollections } from './store.js';
import './collections.css';

const NO_COLLECTIONS: readonly Collection[] = [];

export const SAVE_CONTROL_NAME = 'Save this query as a collection';

export type SaveFlowState =
  | { readonly phase: 'idle' }
  | {
      readonly phase: 'naming';
      readonly name: string;
      readonly queryText: string;
      readonly refusal: CollectionRefusal | null;
    };

export interface SaveQueryFlow {
  readonly state: SaveFlowState;
  readonly visible: boolean;
  readonly begin: () => void;
  readonly setName: (value: string) => void;
  readonly commit: () => void;
  readonly cancel: () => void;
}

export interface SaveQueryDeps {
  readonly queryText: string;
  readonly engine: QueryEngine;
  readonly onRestoreQuery: (text: string) => void;
}

export function useSaveQueryFlow(deps: SaveQueryDeps): SaveQueryFlow {
  const store = useCollections();
  const { create } = store;
  const { engine, onRestoreQuery, queryText } = deps;
  const [state, setState] = useState<SaveFlowState>({ phase: 'idle' });
  const mounted = useRef(true);
  const existing = store.state.status === 'ready' ? store.state.collections : NO_COLLECTIONS;
  const visible = store.state.status === 'ready' && saveControlVisible(queryText, existing, engine);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  const begin = useCallback((): void => {
    if (!visible) return;
    setState({
      phase: 'naming',
      name: proposedName(queryText, engine),
      queryText,
      refusal: null,
    });
  }, [engine, queryText, visible]);

  const setName = useCallback((value: string): void => {
    setState((current) =>
      current.phase === 'naming' ? { ...current, name: value, refusal: null } : current,
    );
  }, []);

  const cancel = useCallback((): void => {
    if (state.phase !== 'naming') return;
    onRestoreQuery(state.queryText);
    setState({ phase: 'idle' });
  }, [onRestoreQuery, state]);

  const commit = useCallback((): void => {
    if (state.phase !== 'naming') return;
    const snapshot = state;
    const refusal = refuseSave(
      { name: snapshot.name, queryText: snapshot.queryText, existing },
      engine,
    );
    if (refusal !== null) {
      setState({ ...snapshot, refusal });
      return;
    }

    void create({
      name: snapshot.name.trim(),
      queryText: snapshot.queryText,
      queryGrammarVersion: engine.grammarVersion,
      sortIndex: nextSortIndex(existing),
    })
      .then((coreRefusal) => {
        if (!mounted.current) return;
        if (coreRefusal !== null) {
          setState((current) =>
            current.phase === 'naming' &&
            current.name === snapshot.name &&
            current.queryText === snapshot.queryText
              ? { ...current, refusal: coreRefusal }
              : current,
          );
          return;
        }
        onRestoreQuery(snapshot.queryText);
        setState({ phase: 'idle' });
      })
      // §8.8 states five refusals and none of them means "the core could not be reached", so
      // nothing is worded here. The field is left exactly as the user left it — the typed name
      // survives and `Enter` can be pressed again — and the rejection is absorbed rather than
      // surfacing as an unhandled promise in the renderer. §2.4 forbids showing the core's
      // diagnostic message raw, and §8.8 owes this path a sentence it does not yet have.
      .catch(() => undefined);
  }, [create, engine, existing, onRestoreQuery, state]);

  return { state, visible, begin, setName, commit, cancel };
}

export interface SaveQueryButtonProps {
  readonly flow: SaveQueryFlow;
}

export function SaveQueryButton(props: SaveQueryButtonProps): ReactElement | null {
  const { flow } = props;
  if (!flow.visible) return null;
  const refusal = flow.state.phase === 'naming' ? flow.state.refusal : null;

  return (
    <button
      type="button"
      id="cdt-save-control"
      className="cdt-save-control"
      data-refused={refusal === null ? 'false' : 'true'}
      aria-label={SAVE_CONTROL_NAME}
      aria-live="polite"
      onClick={flow.state.phase === 'naming' ? flow.commit : flow.begin}
    >
      {refusal === null ? SAVE_CONTROL_TEXT : REFUSAL_CONTROL_TEXT[refusal]}
    </button>
  );
}

export interface CollectionNameInputProps {
  readonly flow: SaveQueryFlow;
}

export function CollectionNameInput(props: CollectionNameInputProps): ReactElement | null {
  const { flow } = props;
  const input = useRef<HTMLInputElement>(null);
  const naming = flow.state.phase === 'naming';

  useEffect(() => {
    if (naming) input.current?.select();
  }, [naming]);

  if (flow.state.phase !== 'naming') return null;
  const refusal = flow.state.refusal;

  return (
    <input
      ref={input}
      type="text"
      className="cdt-name-input"
      aria-label="Collection name"
      aria-invalid={refusal === null ? 'false' : 'true'}
      aria-describedby={refusal === null ? undefined : 'cdt-save-control'}
      value={flow.state.name}
      onChange={(event) => {
        flow.setName(event.target.value);
      }}
      onKeyDown={(event) => {
        if (event.key === 'Enter') {
          event.preventDefault();
          flow.commit();
        } else if (event.key === 'Escape') {
          event.preventDefault();
          event.stopPropagation();
          flow.cancel();
        }
      }}
    />
  );
}
