/**
 * Preload. The only bridge between the sandboxed renderer and the shell.
 *
 * The renderer may never originate a filesystem path or an executable. Paths enter only from
 * a shell-owned native dialog; assets are resolved from opaque references. Spec §2.4.
 */
import { contextBridge } from 'electron';
import { PROTOCOL_VERSION } from '../generated/protocol';

contextBridge.exposeInMainWorld('codotheca', {
  protocolVersion: PROTOCOL_VERSION,
});
