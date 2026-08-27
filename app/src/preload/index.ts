/**
 * Preload. The only bridge between the sandboxed renderer and the shell.
 *
 * The renderer may never originate a filesystem path or an executable. Paths enter only from
 * a shell-owned native dialog; assets are resolved from opaque references. Spec §2.4.
 */
import { contextBridge } from 'electron';
import { PROTOCOL_VERSION } from '../generated/protocol';
import { CODOTHECA_BRIDGE_KEY, type CodothecaBridge } from '../shared/bridge';
import { effectsTierFromArgv } from '../shared/effectsTier';

// process.argv is available synchronously in a sandboxed preload, so the tier reaches the
// document with no round trip — which is the whole point of §11.2a.
const bridge: CodothecaBridge = {
  protocolVersion: PROTOCOL_VERSION,
  effectsTier: effectsTierFromArgv(process.argv) ?? 'auto',
};

contextBridge.exposeInMainWorld(CODOTHECA_BRIDGE_KEY, bridge);
