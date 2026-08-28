import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';
import { PassThrough } from 'node:stream';
import { describe, expect, it } from 'vitest';
import { drainStderr, openRollingLog } from './log';

function tmp(tag: string): string {
  return fs.mkdtempSync(path.join(os.tmpdir(), `codotheca-${tag}-`));
}

describe('rolling log', () => {
  it('rolls once it passes its byte budget and keeps the old file', () => {
    const dir = tmp('log');
    const log = openRollingLog({ dir, maxBytes: 400, keep: 2, level: 'debug' });
    for (let i = 0; i < 20; i += 1) log.write('info', 'shell', `line ${String(i)} padded out`);
    log.close();
    expect(fs.existsSync(path.join(dir, 'codotheca.log'))).toBe(true);
    expect(fs.existsSync(path.join(dir, 'codotheca.log.1'))).toBe(true);
    fs.rmSync(dir, { recursive: true, force: true });
  });

  it('drops a level below the threshold', () => {
    const dir = tmp('level');
    const log = openRollingLog({ dir, maxBytes: 1_000_000, keep: 2, level: 'warn' });
    log.write('debug', 'shell', 'noise');
    log.write('error', 'shell', 'signal');
    log.close();
    const body = fs.readFileSync(path.join(dir, 'codotheca.log'), 'utf8');
    expect(body).not.toContain('noise');
    expect(body).toContain('signal');
    fs.rmSync(dir, { recursive: true, force: true });
  });

  it('joins a panic split across two stderr chunks into one line, tagged core', async () => {
    const dir = tmp('stderr');
    const log = openRollingLog({ dir, maxBytes: 1_000_000, keep: 2, level: 'debug' });
    const stream = new PassThrough();
    drainStderr(stream, log);
    stream.write('thread ');
    stream.write("'main' panicked\n");
    await new Promise<void>((r) => stream.end(r));
    log.close();
    const body = fs.readFileSync(path.join(dir, 'codotheca.log'), 'utf8');
    expect(body).toContain("core thread 'main' panicked");
    fs.rmSync(dir, { recursive: true, force: true });
  });
});
