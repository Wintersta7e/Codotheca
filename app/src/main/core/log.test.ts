import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';
import { PassThrough } from 'node:stream';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { type LogLevel, drainStderr, openRollingLog } from './log';

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

// R12/R24: the log's level vocabulary and the schema's are one value stated in two places, and
// they had already drifted — `trace` existed on the wire and not here. A missing entry does not
// throw: the filter compares `ORDER[lvl] > ORDER[level]`, so it becomes `undefined > n`, which
// is false, and the filter FAILS OPEN — every trace line written at every level, burying the
// panic the log exists to preserve. So the discriminating assertion is that a level BELOW the
// configured one is suppressed; merely writing at each level passes either way.
describe('the level vocabulary', () => {
  it('suppresses a level more verbose than the configured one', () => {
    const dir = tmp('verbosity');
    const log = openRollingLog({ dir, maxBytes: 1024 * 1024, keep: 2, level: 'info' });
    log.write('trace', 'shell', 'trace line');
    log.write('info', 'shell', 'info line');
    log.close();
    const text = fs.readFileSync(log.path, 'utf8');
    expect(text).toContain('info line');
    expect(text, 'trace is below info and must not reach the file').not.toContain('trace line');
  });

  it('is the schema’s, every variant of it', () => {
    const schema = JSON.parse(
      fs.readFileSync(
        fileURLToPath(new URL('../../../../protocol/schema/protocol.json', import.meta.url)),
        'utf8',
      ),
    ) as { types: Record<string, { variants?: string[] }> };
    const variants = schema.types['LogLevel']?.variants ?? [];
    expect(variants.length).toBeGreaterThan(0);

    const dir = tmp('levels');
    for (const level of variants) {
      const log = openRollingLog({ dir, maxBytes: 1024 * 1024, keep: 2, level: level as LogLevel });
      log.write(level as LogLevel, 'shell', `at ${level}`);
      log.close();
      expect(fs.readFileSync(log.path, 'utf8'), `${level} wrote nothing`).toContain(`at ${level}`);
    }
  });
});
