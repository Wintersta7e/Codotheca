/**
 * Acceptance: §7.4, §7.6 — the shell-observable halves of criteria 56 and 62.
 *
 * Test names come from `acceptance/criteria.json`. Two clauses of AC-56-effects are NOT covered
 * here and are covered nowhere yet — see 10b's Acceptance coverage table.
 */
import * as path from 'node:path';
import { describe, expect, it, vi } from 'vitest';

import type {
  CommandArgs,
  CommandResult,
  ProjectId,
  SceneHash,
} from '../../src/generated/protocol';
import {
  createArtProtocolHandler,
  parseArtAddress,
  renditionFilePath,
} from '../../src/main/art/artProtocol';

const HASH = 'abcdef0123456789'.repeat(4);
const NEXT = '0123456789abcdef'.repeat(4);
const DATA_DIR = path.join('/data', 'codotheca');

describe('art', () => {
  it('AC-56 reroll holds the old bitmap until the new hash has decoded and swaps with no transition', async () => {
    // One projectId per call, and an absolute offset: the argument is a single id and a single
    // integer, so no batch reroll of a selection, section, collection or shelf is expressible.
    // `ProjectId` and `SceneHash` are branded nominal types (plan 02's codegen), so a fixture
    // has to say which id it is holding. That is the point of the brand, not a cast to work
    // around one: a bare number cannot reach a command that wants a project.
    const args: CommandArgs['art.rerender'] = { projectId: 7 as ProjectId, offset: 1 };
    expect(Object.keys(args).sort()).toEqual(['offset', 'projectId']);
    expect(typeof args.offset).toBe('number');

    // The reply carries the new hash and the rejection flag, so a stale rail resyncs rather than
    // teleporting the walk (§7.4).
    const reply: CommandResult['art.rerender'] = {
      projectId: 7 as ProjectId,
      offset: 1,
      rejected: false,
      sceneHash: NEXT as SceneHash,
      artState: 'ready',
    };
    expect(reply.rejected).toBe(false);

    // The address the renderer swaps to changes only because the hash changed: the scheme is
    // content-addressed, so the old bitmap stays valid at its own address until it is dropped.
    const before = parseArtAddress(`codotheca://art/${HASH}/card`);
    const after = parseArtAddress(`codotheca://art/${reply.sceneHash ?? ''}/card`);
    expect(before?.hash).not.toBe(after?.hash);
    expect(renditionFilePath(DATA_DIR, { hash: HASH, rendition: 'card' })).not.toBe(
      renditionFilePath(DATA_DIR, { hash: NEXT, rendition: 'card' }),
    );

    // Both addresses resolve while both files exist, which is what makes "hold the decoded
    // bitmap until the new hash has decoded" possible at all from the shell's side.
    const readRendition = vi.fn(() => Promise.resolve(Buffer.from([1])));
    const handler = createArtProtocolHandler({
      dataDir: DATA_DIR,
      readRendition,
      log: { write: vi.fn() },
    });
    expect((await handler({ url: `codotheca://art/${HASH}/card` })).status).toBe(200);
    expect((await handler({ url: `codotheca://art/${NEXT}/card` })).status).toBe(200);

    // NOT COVERED HERE, and covered nowhere yet:
    //   - holding the decoded bitmap and swapping with no transition — plan 12b's card component
    //   - the back step and the readout absent at offset 0 — plan 14's project-page rail
    //   - "fires no notification" — the shell has no notification module to assert against
    // The core-side clauses (no xp_events row, identity unchanged, absolute offset, the walk
    // back) are in core/tests/acceptance_art.rs::ac_56_reroll_offset_is_absolute.
  });

  it('AC-62 the art scheme takes two segments and the card rendition is the stated size', async () => {
    // Explicitly typed, so `mock.calls` carries the file argument: an inferred zero-arg mock
    // gives an empty tuple and the assertion below would have nothing to read.
    const readRendition = vi.fn<(file: string) => Promise<Buffer>>(() =>
      Promise.resolve(Buffer.from([1, 2, 3])),
    );
    const handler = createArtProtocolHandler({
      dataDir: DATA_DIR,
      readRendition,
      log: { write: vi.fn() },
    });

    // A fetch of codotheca://art/<hash> fails: one filename cannot hold two renditions.
    expect((await handler({ url: `codotheca://art/${HASH}` })).status).toBe(400);
    expect(readRendition).not.toHaveBeenCalled();

    // Both segments resolve, and to different files.
    expect((await handler({ url: `codotheca://art/${HASH}/card` })).status).toBe(200);
    expect((await handler({ url: `codotheca://art/${HASH}/hero` })).status).toBe(200);
    expect(readRendition.mock.calls.map((c) => c[0])).toEqual([
      path.join(DATA_DIR, 'art', 'ab', `${HASH}.card.webp`),
      path.join(DATA_DIR, 'art', 'ab', `${HASH}.hero.webp`),
    ]);

    // The card rendition's 600x900, and "art_state and fail_count track the card only", are
    // core-side facts asserted in core/tests/acceptance_storage.rs::ac_22_art_cache_under_50mb
    // and core/src/art/job.rs. The shell never decodes a rendition; it serves bytes.
  });
});
