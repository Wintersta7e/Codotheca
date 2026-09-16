/**
 * Acceptance: §25.2 — the external opener, and the permission handler it does not touch.
 *
 * Test names come from `acceptance/criteria.json`. The unit assertions live beside the module
 * in `src/main/dialogs/externalLink.test.ts`; these two are what the register names.
 */
import { readdirSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it, vi } from 'vitest';

import { readScannedFile } from '../../../scripts/lib/read-scanned.mjs';
import type { CommandName } from '../../src/generated/protocol';
import { IPC_OPEN_REMOTE_LINK, type OpenRemoteLinkReply } from '../../src/shared/channels';
import { registerExternalLink } from '../../src/main/dialogs/externalLink';
import { denyPermissionRequest } from '../../src/main/security';

const appDir = fileURLToPath(new URL('../..', import.meta.url));

function rendererSources(): { path: string; code: string }[] {
  const out: { path: string; code: string }[] = [];
  const walk = (dir: string): void => {
    for (const entry of readdirSync(dir)) {
      const full = join(dir, entry);
      let stat;
      try {
        stat = statSync(full);
      } catch (error: unknown) {
        if (
          typeof error === 'object' &&
          error !== null &&
          'code' in error &&
          error.code === 'ENOENT'
        )
          continue;
        throw error;
      }
      if (stat.isDirectory()) walk(full);
      else if (/\.tsx?$/u.test(entry)) {
        const code = readScannedFile(full);
        // Skipped BEFORE it is counted: counting a file that was never read would make the
        // zero guard below stop meaning what it says.
        if (code !== null) out.push({ path: relative(appDir, full).replace(/\\/gu, '/'), code });
      }
    }
  };
  walk(join(appDir, 'src/renderer'));
  return out;
}

/**
 * Every permission name Electron can ask for, plus the one §25.2 is about. The list is
 * transcribed because the point is that `openExternal` is **among** them and is refused like
 * every other: a loop over one name would prove nothing about the handler.
 */
const PERMISSION_NAMES = [
  'clipboard-read',
  'clipboard-sanitized-write',
  'display-capture',
  'fullscreen',
  'geolocation',
  'idle-detection',
  'media',
  'mediaKeySystem',
  'midi',
  'midiSysex',
  'notifications',
  'pointerLock',
  'keyboardLock',
  'openExternal',
  'speaker-selection',
  'storage-access',
  'top-level-storage-access',
  'window-management',
  'unknown',
];

describe('remote surfaces: the opener', () => {
  it('AC-P2-25-12 no URL crosses IPC and the shell is the only thing that opens one', async () => {
    const requests: { name: string; args: unknown }[] = [];
    const opened: string[] = [];
    let handler: ((payload: unknown) => Promise<OpenRemoteLinkReply>) | null = null;

    registerExternalLink({
      handle: (channel, fn) => {
        expect(channel).toBe(IPC_OPEN_REMOTE_LINK);
        handler = fn;
      },
      request: (name: CommandName, args: unknown) => {
        requests.push({ name, args });
        // The host is not on the allowlist, so the core answers NULL. This is the criterion's
        // second clause: a non-allowlisted host produces no URL and therefore no link.
        return Promise.resolve(name === 'accounts.list' ? [] : null);
      },
      confirm: () => Promise.resolve(true),
      openExternal: (url: string) => {
        opened.push(url);
        return Promise.resolve();
      },
    });
    if (handler === null) throw new Error('registerExternalLink registered no handler');
    const invoke = handler as (payload: unknown) => Promise<OpenRemoteLinkReply>;

    // The renderer's message carries `{ projectId, kind }` only. A `url` field on the payload
    // reaches nothing: the args that leave this process are built from the two read fields.
    const reply = await invoke({
      projectId: 7,
      kind: 'issues',
      url: 'https://evil.example.invalid/pwn',
    });
    expect(reply).toEqual({ kind: 'not_linkable' });
    expect(opened).toEqual([]);
    expect(requests.filter((r) => r.name === 'remote.webUrl')).toEqual([
      { name: 'remote.webUrl', args: { projectId: 7, kind: 'issues' } },
    ]);

    // …and `shell.openExternal` appears in no renderer module. The walk prints its count and
    // fails at zero, because a gate whose passing run scanned nothing is a failing gate.
    const files = rendererSources();
    process.stderr.write(
      `opener: renderer scan read ${String(files.length)} renderer source file(s)\n`,
    );
    expect(files.length, 'the renderer walk read no file, so it proved nothing').toBeGreaterThan(
      20,
    );
    expect(files.filter((f) => /openExternal/u.test(f.code)).map((f) => f.path)).toEqual([]);
  });

  it('AC-P2-25-13 denyPermissionRequest refuses every permission, openExternal included', () => {
    process.stderr.write(
      `opener: permission gate covered ${String(PERMISSION_NAMES.length)} permission name(s)\n`,
    );
    expect(
      PERMISSION_NAMES.length,
      'the permission gate covered no name, so it proved nothing',
    ).toBeGreaterThan(0);
    expect(PERMISSION_NAMES).toContain('openExternal');

    for (const permission of PERMISSION_NAMES) {
      const callback = vi.fn();
      denyPermissionRequest(null, permission, callback);
      expect(callback, permission).toHaveBeenCalledWith(false);
    }
  });
});
