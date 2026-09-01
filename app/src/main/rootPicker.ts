/**
 * The shell half of adding a scan root (§2.4, §10.1b).
 *
 * §2.4 makes `roots.add` privileged and forbids the renderer from originating a filesystem
 * path, so a folder can only reach the core through a native dialog this process owns. This is
 * that dialog, and R11 makes it the only `ipcMain.handle` on `IPC_PICK_ROOT` in the product:
 * Electron throws at a second registration, so a duplicate is a startup crash rather than a
 * duplicated string.
 */
import type { RootAdd } from '../generated/protocol';
import { type BridgeError, IPC_PICK_ROOT, type PickRootReply } from '../shared/channels';

/**
 * §2.5: bytes that must cross are tagged, never sent as a display string.
 *
 * `dialogs/relocate.ts` has `pathToBytes` doing the same job for the other dialog that owns a
 * path. One of the two should become the shell's single byte-tagging helper; neither plan
 * declares an owner for it, so this one is left where its plan puts it and the overlap is
 * reported rather than resolved across a plan boundary.
 */
export function encodePathBytes(path: string): { readonly b64: string } {
  return { b64: Buffer.from(path, 'utf8').toString('base64') };
}

export interface RootPickerDeps {
  readonly showOpenDialog: () => Promise<{ canceled: boolean; filePaths: string[] }>;
  readonly addRoot: (args: {
    pathBytes: { b64: string };
    confirmLarge: boolean;
  }) => Promise<RootAdd>;
  readonly handle: (channel: string, fn: (payload: unknown) => Promise<PickRootReply>) => void;
}

function isRequest(value: unknown): value is { confirmLarge: boolean } {
  return (
    typeof value === 'object' &&
    value !== null &&
    typeof (value as { confirmLarge?: unknown }).confirmLarge === 'boolean'
  );
}

/**
 * §2.2: `outcome` is `'unknown'` or absent, and there is no `'failed'` member. A payload
 * rejected before the dialog opened definitely did not take effect, so it carries `null`.
 */
const PROTOCOL_ERROR: BridgeError = {
  code: 'PROTOCOL',
  message: 'pick-root: confirmLarge must be a boolean',
  outcome: null,
  retryable: false,
};

export function registerRootPicker(deps: RootPickerDeps): void {
  deps.handle(IPC_PICK_ROOT, async (payload): Promise<PickRootReply> => {
    if (!isRequest(payload)) {
      return { kind: 'failed', error: PROTOCOL_ERROR };
    }
    const chosen = await deps.showOpenDialog();
    const first = chosen.filePaths[0];
    if (chosen.canceled || first === undefined) {
      return { kind: 'cancelled' };
    }
    try {
      const add = await deps.addRoot({
        pathBytes: encodePathBytes(first),
        confirmLarge: payload.confirmLarge,
      });
      // A refusal is a reply, not a failure: §10.1a requires an explanation and never a
      // silent no, and the screen draws refusals as rows.
      return { kind: 'added', add };
    } catch (error) {
      const code = (error as { code?: unknown }).code;
      return {
        kind: 'failed',
        error: {
          code: typeof code === 'string' ? (code as BridgeError['code']) : 'CORE_RESTARTED',
          message: error instanceof Error ? error.message : 'the core did not answer',
          // The request left this process. §2.2 forbids auto-replaying it.
          outcome: 'unknown',
          retryable: false,
        },
      };
    }
  });
}
