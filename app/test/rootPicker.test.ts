import { test, expect } from 'vitest';
import { IPC_PICK_ROOT } from '../src/shared/channels';
import type { PickRootReply } from '../src/shared/channels';
import { encodePathBytes, registerRootPicker } from '../src/main/rootPicker';
import type { RootAdd, RootId } from '../src/generated/protocol';
import { required } from '../src/shared/required';

function harness(
  dialog: { canceled: boolean; filePaths: string[] },
  addRoot: (args: { pathBytes: { b64: string }; confirmLarge: boolean }) => Promise<RootAdd>,
): (payload: unknown) => Promise<PickRootReply> {
  let handler: ((payload: unknown) => Promise<PickRootReply>) | null = null;
  let registrations = 0;
  registerRootPicker({
    showOpenDialog: () => Promise.resolve(dialog),
    addRoot,
    handle: (channel, fn) => {
      expect(channel).toBe(IPC_PICK_ROOT);
      registrations += 1;
      handler = fn;
    },
  });
  // R11: Electron throws at the second `ipcMain.handle` on one channel, so a second registration
  // anywhere in the product is a startup crash. This registration is the only one.
  expect(registrations).toBe(1);
  if (handler === null) throw new Error('registerRootPicker did not register a handler');
  return handler;
}

const added: RootAdd = {
  root: {
    id: 1 as RootId,
    pathDisplay: '/somewhere/dev',
    kind: 'linux',
    distro: '',
    enabled: true,
    descendIntoRepos: false,
    provenance: 'dialog',
    state: 'watched',
    addedAt: 1,
    projectCount: null,
  },
  refusedBecause: null,
  estimatedDirs: null,
};

test('a path is encoded as tagged bytes, never as a string on the wire', () => {
  // §2.5: where bytes genuinely must cross they are tagged {"b64": "..."}.
  expect(encodePathBytes('/somewhere/dev')).toEqual({
    b64: Buffer.from('/somewhere/dev', 'utf8').toString('base64'),
  });
});

test('cancelling the dialog calls nothing and says so', async () => {
  let calls = 0;
  const handler = harness({ canceled: true, filePaths: [] }, () => {
    calls += 1;
    return Promise.resolve(added);
  });
  await expect(handler({ confirmLarge: false })).resolves.toEqual({ kind: 'cancelled' });
  expect(calls).toBe(0);
});

test('a chosen folder reaches roots.add exactly once, with the confirmation flag', async () => {
  const seen: { pathBytes: { b64: string }; confirmLarge: boolean }[] = [];
  const handler = harness({ canceled: false, filePaths: ['/somewhere/dev'] }, (args) => {
    seen.push(args);
    return Promise.resolve(added);
  });
  const reply = await handler({ confirmLarge: true });
  expect(reply).toEqual({ kind: 'added', add: added });
  expect(seen).toHaveLength(1);
  const [call] = seen;
  expect(required(call, 'roots.add call').confirmLarge).toBe(true);
  expect(required(call, 'roots.add call').pathBytes).toEqual(encodePathBytes('/somewhere/dev'));
});

// §10.1a: a refusal comes back with an explanation, never a silent no. The refusal is a normal
// reply, not an error, so the screen can draw it.
test('a refusal is a reply and not a failure', async () => {
  const refused: RootAdd = {
    root: null,
    refusedBecause: 'too_many_directories',
    estimatedDirs: 500000,
  };
  const handler = harness({ canceled: false, filePaths: ['/somewhere/big'] }, () =>
    Promise.resolve(refused),
  );
  await expect(handler({ confirmLarge: false })).resolves.toEqual({
    kind: 'added',
    add: refused,
  });
});

test('a malformed payload is refused rather than defaulted', async () => {
  const handler = harness({ canceled: false, filePaths: ['/somewhere/dev'] }, () =>
    Promise.resolve(added),
  );
  const reply = await handler({ confirmLarge: 'yes' });
  expect(reply.kind).toBe('failed');
  if (reply.kind !== 'failed') throw new Error('unreachable');
  // §2.2: there is no `failed` wire value. A payload rejected before anything ran definitely
  // did not take effect, and `null` is how that is said.
  expect(reply.error.outcome).toBe(null);
});

test('a core failure comes back as a bridge error and never as a thrown dialog', async () => {
  const handler = harness({ canceled: false, filePaths: ['/somewhere/dev'] }, () =>
    Promise.reject(new Error('core is gone')),
  );
  const reply = await handler({ confirmLarge: false });
  expect(reply.kind).toBe('failed');
  if (reply.kind !== 'failed') throw new Error('unreachable');
  expect(reply.error.code).toBe('CORE_RESTARTED');
  expect(reply.error.outcome).toBe('unknown');
});
