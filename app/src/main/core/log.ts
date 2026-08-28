/**
 * The rolling log. Everything the core writes to stderr is merged in here, tagged.
 *
 * This is where a panic, a missing shared library and every backtrace arrive. Without it the
 * crash-recovery path has no diagnosis, which is the failure §2.1 records against v1.
 */
import * as fs from 'node:fs';
import * as path from 'node:path';

export type LogLevel = 'error' | 'warn' | 'info' | 'debug';
export type LogSource = 'shell' | 'core';

const ORDER: Record<LogLevel, number> = { error: 0, warn: 1, info: 2, debug: 3 };

export interface RollingLog {
  /** The path the failure surfaces show the user. */
  readonly path: string;
  write(level: LogLevel, source: LogSource, line: string): void;
  setLevel(level: LogLevel): void;
  close(): void;
}

export interface RollingLogOptions {
  dir: string;
  maxBytes: number;
  keep: number;
  level: LogLevel;
}

export function openRollingLog(opts: RollingLogOptions): RollingLog {
  fs.mkdirSync(opts.dir, { recursive: true });
  const file = path.join(opts.dir, 'codotheca.log');
  let level = opts.level;
  let fd = fs.openSync(file, 'a');
  let written = fs.fstatSync(fd).size;

  function roll(): void {
    fs.closeSync(fd);
    for (let i = opts.keep - 1; i >= 1; i -= 1) {
      const from = `${file}.${String(i)}`;
      const to = `${file}.${String(i + 1)}`;
      if (fs.existsSync(from)) fs.renameSync(from, to);
    }
    fs.renameSync(file, `${file}.1`);
    fd = fs.openSync(file, 'a');
    written = 0;
  }

  return {
    path: file,
    setLevel(next: LogLevel): void {
      level = next;
    },
    write(lvl: LogLevel, source: LogSource, line: string): void {
      if (ORDER[lvl] > ORDER[level]) return;
      const record = `${new Date().toISOString()} ${lvl} ${source} ${line.replace(/\n+$/u, '')}\n`;
      const bytes = Buffer.from(record, 'utf8');
      if (written + bytes.length > opts.maxBytes) roll();
      fs.writeSync(fd, bytes);
      written += bytes.length;
    },
    close(): void {
      fs.closeSync(fd);
    },
  };
}

/**
 * Splits a stderr byte stream into lines and hands each to the log. A panic arrives in
 * however many chunks the pipe felt like, so a partial line is carried to the next chunk
 * rather than logged as two.
 */
export function drainStderr(stream: NodeJS.ReadableStream, log: RollingLog): void {
  let carry = '';
  stream.setEncoding('utf8');
  stream.on('data', (chunk: string | Buffer) => {
    carry += typeof chunk === 'string' ? chunk : chunk.toString('utf8');
    const lines = carry.split('\n');
    carry = lines.pop() ?? '';
    for (const line of lines) if (line.length > 0) log.write('info', 'core', line);
  });
  stream.on('end', () => {
    if (carry.length > 0) log.write('info', 'core', carry);
    carry = '';
  });
}
