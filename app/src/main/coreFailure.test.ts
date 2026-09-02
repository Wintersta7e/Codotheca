import { describe, expect, it } from 'vitest';
import type { SpawnFailureFacts } from '../renderer/notices/copy';
import type { CoreStatus } from './core/supervisor';
import { classifySpawnFailure, coreFailureNotice, SPAWN_FAILURE_SENTENCE } from './coreFailure';

const EXECUTABLE = 0o100_755;
const NOT_EXECUTABLE = 0o100_644;

describe('classifySpawnFailure', () => {
  it('names a quarantined binary on Windows', () => {
    expect(
      classifySpawnFailure({
        detail: 'Error: spawn EPERM',
        platform: 'win32',
        mode: EXECUTABLE,
      }),
    ).toBe('quarantined');
    expect(
      classifySpawnFailure({
        detail:
          'Operation did not complete successfully because the file contains a virus or potentially unwanted software.',
        platform: 'win32',
        mode: EXECUTABLE,
      }),
    ).toBe('quarantined');
  });

  it('separates a noexec mount from a lost execute bit by the file mode', () => {
    expect(
      classifySpawnFailure({ detail: 'spawn EACCES', platform: 'linux', mode: EXECUTABLE }),
    ).toBe('noexec');
    expect(
      classifySpawnFailure({ detail: 'spawn EACCES', platform: 'linux', mode: NOT_EXECUTABLE }),
    ).toBe('not-executable');
  });

  it('names a wrong architecture', () => {
    expect(
      classifySpawnFailure({ detail: 'spawn ENOEXEC', platform: 'linux', mode: EXECUTABLE }),
    ).toBe('wrong-architecture');
    expect(
      classifySpawnFailure({
        detail: 'cannot execute binary file: Exec format error',
        platform: 'linux',
        mode: EXECUTABLE,
      }),
    ).toBe('wrong-architecture');
  });

  it('names a C library older than the baseline from the stderr tail', () => {
    expect(
      classifySpawnFailure({
        detail:
          "two crashes within 60000 ms\ncodotheca-core: /lib/libc.so.6: version 'GLIBC_2.38' not found",
        platform: 'linux',
        mode: EXECUTABLE,
      }),
    ).toBe('glibc-too-old');
  });

  it('names a binary the installation did not leave behind', () => {
    expect(classifySpawnFailure({ detail: 'spawn ENOENT', platform: 'linux', mode: null })).toBe(
      'missing',
    );
  });

  it('falls back to unknown rather than guessing', () => {
    expect(
      classifySpawnFailure({ detail: 'something else', platform: 'linux', mode: EXECUTABLE }),
    ).toBe('unknown');
  });
});

describe('coreFailureNotice', () => {
  const failed = (
    reason: 'spawn' | 'protocol_version' | 'crash_loop',
    detail: string,
  ): CoreStatus => ({
    kind: 'failed',
    reason,
    detail,
    logPath: 'application.log',
  });

  it('is null unless the core actually failed', () => {
    expect(coreFailureNotice({ kind: 'starting' }, { platform: 'linux', mode: null })).toBeNull();
    expect(
      coreFailureNotice(
        { kind: 'restarting', epoch: 2, delayMs: 2_000 },
        { platform: 'linux', mode: null },
      ),
    ).toBeNull();
  });

  it('carries the cause sentence, log path, and no dismissal', () => {
    const notice = coreFailureNotice(failed('spawn', 'spawn EACCES'), {
      platform: 'linux',
      mode: NOT_EXECUTABLE,
    });
    const facts: SpawnFailureFacts | null = notice;
    expect(facts?.body).toBe(SPAWN_FAILURE_SENTENCE['not-executable']);
    expect(notice?.logPath).toBe('application.log');
    expect(notice?.primary).toBe('RETRY');
    expect(notice?.secondary).toBe('OPEN THE LOG');
    expect(notice?.dismissible).toBe(false);
  });

  it('shows the shell-composed protocol mismatch unchanged', () => {
    const detail = 'core speaks protocol 3, this build speaks 2';
    expect(
      coreFailureNotice(failed('protocol_version', detail), { platform: 'linux', mode: null })
        ?.body,
    ).toBe(detail);
  });

  it('classifies a crash loop from its stderr tail rather than reporting the loop', () => {
    const notice = coreFailureNotice(
      failed(
        'crash_loop',
        "two crashes within 60000 ms\n/lib/libc.so.6: version 'GLIBC_2.38' not found",
      ),
      { platform: 'linux', mode: EXECUTABLE },
    );
    expect(notice?.body).toBe(SPAWN_FAILURE_SENTENCE['glibc-too-old']);
  });

  it('never names a destructive action', () => {
    const all = [
      ...Object.values(SPAWN_FAILURE_SENTENCE),
      coreFailureNotice(failed('spawn', 'spawn EACCES'), {
        platform: 'linux',
        mode: NOT_EXECUTABLE,
      })?.title ?? '',
    ].join(' ');
    expect(all).not.toMatch(/FORGET|delete|uninstall|reinstall/iu);
  });
});
