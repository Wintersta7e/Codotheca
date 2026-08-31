import { describe, expect, it } from 'vitest';
import { registerFocusRelease, releaseFocus } from './focus';

interface Harness {
  sent: unknown[];
  errors: string[];
  fire: (name: string) => void;
}

function harness(): Harness {
  const sent: unknown[] = [];
  const errors: string[] = [];
  const cbs: Record<string, () => void> = {};
  registerFocusRelease({
    release: (args: unknown) => {
      sent.push(args);
      return Promise.resolve();
    },
    onBlur: (cb: () => void) => {
      cbs['blur'] = cb;
    },
    onHide: (cb: () => void) => {
      cbs['hide'] = cb;
    },
    onDestroyed: (cb: () => void) => {
      cbs['destroyed'] = cb;
    },
    onError: (detail: string) => {
      errors.push(detail);
    },
  });
  return { sent, errors, fire: (name: string) => cbs[name]?.() };
}

describe('the shell half of the focus protocol', () => {
  it.each(['blur', 'hide', 'destroyed'])('%s releases the claim', (event) => {
    const h = harness();
    h.fire(event);
    expect(h.sent).toEqual([{ projectId: null }]);
  });

  it('destroy releases even after a blur already did', () => {
    // The residency hybrid: the renderer's heartbeat dies with the window, so the last claim
    // would otherwise stand for the whole staleness window.
    const h = harness();
    h.fire('blur');
    h.fire('destroyed');
    expect(h.sent).toEqual([{ projectId: null }, { projectId: null }]);
  });

  it('never sends a project id, from any event', () => {
    // L5. The shell knows the window went away; it does not know what was on it.
    const h = harness();
    for (const event of ['blur', 'hide', 'destroyed']) h.fire(event);
    expect(h.sent).toHaveLength(3);
    for (const args of h.sent) expect(args).toEqual({ projectId: null });
  });

  it('registers all three events, so none of the assertions above is vacuous', () => {
    const h = harness();
    for (const event of ['blur', 'hide', 'destroyed']) h.fire(event);
    expect(h.sent).toHaveLength(3);
    // There is no onFocus, and there must not be: the shell does not know which view will be
    // on screen when the window comes back.
    h.fire('focus');
    expect(h.sent).toHaveLength(3);
  });

  it('a rejected release is routed to onError, never thrown into an Electron handler', async () => {
    // At destroy time the core may already be gone; an unhandled rejection in a window event
    // handler is how a clean shutdown becomes a crash report.
    const errors: string[] = [];
    await releaseFocus({
      release: () => Promise.reject(new Error('core is gone')),
      onError: (detail) => errors.push(detail),
    });
    expect(errors).toHaveLength(1);
    expect(errors[0]).toContain('core is gone');
  });
});
