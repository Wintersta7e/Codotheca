import { describe, expect, it } from 'vitest';

describe('the node test environment', () => {
  it('has no DOM, so a node-side test cannot accidentally rely on one', () => {
    expect('document' in globalThis).toBe(false);
  });

  it('runs on the supported Node major', () => {
    const major = Number.parseInt(process.versions.node.split('.')[0] ?? '0', 10);
    expect(major).toBeGreaterThanOrEqual(20);
  });
});
