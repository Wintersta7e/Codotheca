/**
 * Preload. The only bridge between the sandboxed renderer and the shell.
 *
 * The renderer may never originate a filesystem path or an executable. Paths enter only from
 * a shell-owned native dialog; assets are resolved from opaque references. Spec §2.4.
 */
import { contextBridge, ipcRenderer } from 'electron';
import { PROTOCOL_VERSION } from '../generated/protocol';
import { CODOTHECA_BRIDGE_KEY, type CodothecaBridge } from '../shared/bridge';
import {
  IPC_CLEAR_PAINT_FAILURE,
  IPC_CORE_STATUS,
  IPC_EVENTS,
  IPC_INDEX_LOCATION,
  IPC_OPEN_PALETTE,
  IPC_PICK_EXECUTABLE,
  IPC_PICK_ROOT,
  IPC_RELOCATE,
  IPC_REQUEST,
  IPC_REVEAL,
  IPC_SHORTCUT_STATE,
  type CommitSuggestionReply,
  IPC_COMMIT_SUGGESTION,
  type PickRootReply,
  type RelocateReply,
  type RevealTarget,
  type ShortcutState,
} from '../shared/channels';
import {
  effectsTierFromArgv,
  effectsTierSourceFromArgv,
  paintFailForcedAtFromArgv,
} from '../shared/effectsTier';
import { logPathFromArgv } from '../shared/windowArgs';

// process.argv is available synchronously in a sandboxed preload, so the tier reaches the
// document with no round trip — which is the whole point of §11.2a.
const bridge: CodothecaBridge = {
  protocolVersion: PROTOCOL_VERSION,
  effectsTier: effectsTierFromArgv(process.argv) ?? 'auto',
  effectsTierSource: effectsTierSourceFromArgv(process.argv) ?? 'boot-file',
  paintFailForcedAt: paintFailForcedAtFromArgv(process.argv),
  logPath: logPathFromArgv(process.argv),
  request: (name: string, args: unknown): Promise<unknown> =>
    ipcRenderer.invoke(IPC_REQUEST, { name, args }),
  // An id and nothing else. The folder is chosen in the main process, where the dialog lives.
  relocate: (locationId: number): Promise<RelocateReply> =>
    ipcRenderer.invoke(IPC_RELOCATE, { locationId }) as Promise<RelocateReply>,
  // A flag and nothing else. The folder is chosen in the main process, where the dialog lives,
  // and its path never enters the sandbox — §2.4.
  pickRoot: (confirmLarge: boolean): Promise<PickRootReply> =>
    ipcRenderer.invoke(IPC_PICK_ROOT, { confirmLarge }) as Promise<PickRootReply>,
  // A display string and nothing else. The shell resolves it against what `roots.suggest`
  // returned and adds the folder the core itself proposed — the renderer never holds a path.
  commitSuggestion: (pathDisplay: string): Promise<CommitSuggestionReply> =>
    ipcRenderer.invoke(IPC_COMMIT_SUGGESTION, { pathDisplay }) as Promise<CommitSuggestionReply>,
  onCoreStatus: (cb: (status: unknown) => void): void => {
    ipcRenderer.on(IPC_CORE_STATUS, (_event, status: unknown) => {
      cb(status);
    });
  },
  onCoreEvents: (cb: (batch: unknown) => void): void => {
    ipcRenderer.on(IPC_EVENTS, (_event, batch: unknown) => {
      cb(batch);
    });
  },
  onOpenPalette: (cb: () => void): void => {
    ipcRenderer.on(IPC_OPEN_PALETTE, () => {
      cb();
    });
  },
  // R11: `pickRoot` is plan 16's — it declares `IPC_PICK_ROOT` and owns the only handler for
  // it, so its renderer wrapper belongs beside the constant rather than here.
  pickExecutable: (scope: unknown): Promise<unknown> =>
    ipcRenderer.invoke(IPC_PICK_EXECUTABLE, scope),
  reveal: (target: RevealTarget): Promise<unknown> => ipcRenderer.invoke(IPC_REVEAL, { target }),
  indexLocation: (): Promise<unknown> => ipcRenderer.invoke(IPC_INDEX_LOCATION, null),
  clearPaintFailure: (): Promise<unknown> => ipcRenderer.invoke(IPC_CLEAR_PAINT_FAILURE, null),
  onShortcutState: (cb: (state: ShortcutState) => void): void => {
    ipcRenderer.on(IPC_SHORTCUT_STATE, (_event, state: ShortcutState) => {
      cb(state);
    });
  },
};

contextBridge.exposeInMainWorld(CODOTHECA_BRIDGE_KEY, bridge);
