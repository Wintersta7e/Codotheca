/**
 * Preload. The only bridge between the sandboxed renderer and the shell.
 *
 * The renderer may never originate a filesystem path or an executable. Paths enter only from
 * a shell-owned native dialog; assets are resolved from opaque references. Spec §2.4.
 */
import { contextBridge, ipcRenderer } from 'electron';
import { PROTOCOL_VERSION } from '../generated/protocol';
import { CODOTHECA_BRIDGE_KEY, type CodothecaBridge } from '../shared/bridge';
import { IPC_CORE_STATUS, IPC_EVENTS, IPC_REQUEST } from '../shared/channels';
import { effectsTierFromArgv } from '../shared/effectsTier';

// process.argv is available synchronously in a sandboxed preload, so the tier reaches the
// document with no round trip — which is the whole point of §11.2a.
const bridge: CodothecaBridge = {
  protocolVersion: PROTOCOL_VERSION,
  effectsTier: effectsTierFromArgv(process.argv) ?? 'auto',
  request: (name: string, args: unknown): Promise<unknown> =>
    ipcRenderer.invoke(IPC_REQUEST, { name, args }),
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
};

contextBridge.exposeInMainWorld(CODOTHECA_BRIDGE_KEY, bridge);
