import { describe, expect, it } from 'vitest';
import { installQuitGate, type QuitEvent, type QuitStep } from './quitGate';

function rig(steps: readonly QuitStep[]): {
  fireQuit: () => void;
  quits: () => number;
  prevented: () => number;
  lines: string[];
} {
  let listener: (event: QuitEvent) => void = () => undefined;
  let quits = 0;
  let prevented = 0;
  const lines: string[] = [];
  installQuitGate({
    app: {
      on: (_event, nextListener): void => {
        listener = nextListener;
      },
      quit: (): void => {
        quits += 1;
        listener({
          preventDefault: (): void => {
            prevented += 1;
          },
        });
      },
    },
    steps,
    log: {
      path: 'log-file',
      write: (_level, _source, line): void => {
        lines.push(line);
      },
      setLevel: (): void => undefined,
      close: (): void => undefined,
    },
  });
  return {
    fireQuit: (): void => {
      listener({
        preventDefault: (): void => {
          prevented += 1;
        },
      });
    },
    quits: (): number => quits,
    prevented: (): number => prevented,
    lines,
  };
}

async function flush(): Promise<void> {
  for (let index = 0; index < 8; index += 1) await Promise.resolve();
}

describe('installQuitGate', () => {
  it('runs the steps in order, then quits once', async () => {
    const order: string[] = [];
    const fixture = rig([
      {
        name: 'first',
        run: (): Promise<string> => {
          order.push('first');
          return Promise.resolve('ok');
        },
      },
      {
        name: 'second',
        run: (): Promise<string> => {
          order.push('second');
          return Promise.resolve('ok');
        },
      },
    ]);

    fixture.fireQuit();
    await flush();

    expect(order).toEqual(['first', 'second']);
    expect(fixture.quits()).toBe(1);
    expect(fixture.prevented()).toBe(1);
  });

  it('a second quit while the gate is running is blocked, and the steps run once', async () => {
    let runs = 0;
    let release: (() => void) | null = null;
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    const fixture = rig([
      {
        name: 'slow',
        run: async (): Promise<string> => {
          runs += 1;
          await gate;
          return 'ok';
        },
      },
    ]);

    fixture.fireQuit();
    await flush();
    fixture.fireQuit();
    await flush();

    expect(runs).toBe(1);
    expect(fixture.prevented()).toBe(2);
    if (release === null) throw new Error('the slow step did not start');
    (release as () => void)();
    await flush();
    expect(fixture.quits()).toBe(1);
  });

  it('a step that throws does not strand the app unquittable', async () => {
    let followingStepRan = false;
    const fixture = rig([
      {
        name: 'boom',
        run: (): Promise<string> => Promise.reject(new Error('nope')),
      },
      {
        name: 'after',
        run: (): Promise<string> => {
          followingStepRan = true;
          return Promise.resolve('ok');
        },
      },
    ]);

    fixture.fireQuit();
    await flush();

    expect(fixture.quits()).toBe(1);
    expect(fixture.lines.some((line) => line.includes('boom') && line.includes('nope'))).toBe(true);
    expect(followingStepRan).toBe(true);
  });
});
