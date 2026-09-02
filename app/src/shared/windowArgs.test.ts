import { describe, expect, it } from 'vitest';

import { LOG_PATH_FLAG, logPathArgument, logPathFromArgv } from './windowArgs';

describe('logPathFromArgv', () => {
  it('round-trips a path with spaces, which every Windows profile has', () => {
    const path = 'C:\\Users\\Some One\\AppData\\Roaming\\Codotheca\\logs\\codotheca.log';
    const argv = ['electron.exe', logPathArgument(path), '--effects-tier=full'];
    expect(logPathFromArgv(argv)).toBe(path);
  });

  it('encodes the value, so a space cannot split one argument into two', () => {
    // Chromium re-parses a command line for the sandboxed renderer. An unencoded space in the
    // value is the difference between one argument and two, and the second one is not a flag.
    expect(logPathArgument('/tmp/a b/x.log')).not.toContain(' ');
    expect(logPathArgument('/tmp/a b/x.log').startsWith(LOG_PATH_FLAG)).toBe(true);
  });

  it('is the empty string when the shell passed none', () => {
    expect(logPathFromArgv(['electron.exe'])).toBe('');
  });

  it('is the empty string when the value will not decode, never a mangled path', () => {
    expect(logPathFromArgv([`${LOG_PATH_FLAG}%E0%A4%A`])).toBe('');
  });
});
