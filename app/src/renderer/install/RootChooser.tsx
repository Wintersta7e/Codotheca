/**
 * §24.3a's root chooser.
 *
 * **The renderer originates no path.** It lists the roots that already exist and sends a
 * `RootId`; the composed display form comes back from `install.preview`. A folder that is not yet
 * a root is reached through `ADD A FOLDER…`, which routes to the **existing** `IPC_PICK_ROOT`
 * dialog → `roots.add` — the only path-origination channel the product has.
 */
import type { ReactElement } from 'react';

import type { Root, RootId } from '../../generated/protocol.js';

export const ADD_A_FOLDER_LABEL = 'ADD A FOLDER…';

export interface RootChooserProps {
  readonly roots: readonly Root[];
  readonly selected: RootId | null;
  readonly onSelect: (root: RootId) => void;
  /** Invokes `IPC_PICK_ROOT` and nothing else. */
  readonly onAddFolder: () => void;
}

export function RootChooser({
  roots,
  selected,
  onSelect,
  onAddFolder,
}: RootChooserProps): ReactElement {
  return (
    <div className="cdt-root-chooser" role="group" aria-label="Where to install">
      {roots.map((root) => (
        <button
          key={root.id}
          type="button"
          className="cdt-root-chooser__root"
          aria-pressed={root.id === selected}
          // The id, never the path. `pathDisplay` is §1.10's lossy form and is shown, not sent.
          onClick={() => {
            onSelect(root.id);
          }}
        >
          {root.pathDisplay}
        </button>
      ))}
      <button type="button" className="cdt-root-chooser__add" onClick={onAddFolder}>
        {ADD_A_FOLDER_LABEL}
      </button>
    </div>
  );
}
