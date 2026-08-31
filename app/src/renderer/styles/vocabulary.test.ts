import { describe, expect, it } from 'vitest';
import vocabulary from './css-vocabulary.json';
// `?raw` rather than node:fs: the renderer project carries no Node types by design, and under
// jsdom `import.meta.url` is not a file URL. The dom project processes these stylesheets so the
// query returns the real text — Vitest otherwise stubs CSS to an empty module, `?raw` included.
import base from './base.css?raw';

describe('the css vocabulary', () => {
  it('permits only durations §11.6 or a named single-effect surface states', () => {
    // §11.6's beats, plus §7.8's hover table, §7.8a's pin fade and §11.6's flicker glow.
    expect(vocabulary.durationsMs).toContain(160); // the reduced clamp
    expect(vocabulary.durationsMs).toContain(260); // transform hover, lift
    expect(vocabulary.durationsMs).toContain(620); // LED run
    expect(vocabulary.durationsMs).toContain(700); // specular sweep
    expect(vocabulary.durationsMs).toContain(720); // scan line traverse
    expect(vocabulary.durationsMs).toContain(220); // bloom, §7.8
    expect(vocabulary.durationsMs).toContain(140); // pin fade, §7.8a
    expect(vocabulary.durationsMs).toContain(90); // flicker glow, §11.6
    // The design's fixed 130 ms flicker hold is cut; synthesis §2.3's band replaced it.
    expect(vocabulary.durationsMs).not.toContain(130);
  });

  it('permits only §11.6 curves plus the two named single-effect ones', () => {
    expect(vocabulary.timingFunctions).toContain('cubic-bezier(.2,.85,.2,1)');
    expect(vocabulary.timingFunctions).toContain('cubic-bezier(.2,.9,.2,1)');
    expect(vocabulary.timingFunctions).toContain('cubic-bezier(.2,1.4,.4,1)');
    expect(vocabulary.timingFunctions).toContain('cubic-bezier(.3,.7,.2,1)');
    expect(vocabulary.timingFunctions).toContain('cubic-bezier(.34,.6,.24,1)');
    expect(vocabulary.timingFunctions).toContain('linear');
    // TOKENS.md's overshoot curve is the level-up panel's; no phase-1 surface uses it.
    expect(vocabulary.timingFunctions).not.toContain('cubic-bezier(.2,1.5,.4,1)');
  });
});

describe('R35(b): the shared entry keyframes are declared once, here', () => {
  it('names the three the screens share', () => {
    expect(vocabulary.sharedKeyframes).toEqual(['viewIn', 'panelIn', 'turnIn']);
  });

  it('reads the real stylesheet rather than an empty stub', () => {
    // The assertions below are all `toContain`, and every one of them passes vacuously against
    // an empty string. This is the check that the source they scan actually arrived.
    expect(base.length).toBeGreaterThan(0);
    expect(base).toContain('.cdt-visually-hidden');
  });

  it('declares each of them exactly once in base.css', () => {
    for (const name of vocabulary.sharedKeyframes) {
      const declarations = base.match(new RegExp(`@keyframes\\s+${name}\\b`, 'g')) ?? [];
      expect(declarations, name).toHaveLength(1);
    }
  });

  it('carries turnIn, which no plan declared before this ruling', () => {
    // The tracking values are the assertion; their spelling is not. A CSS formatter writes the
    // leading zero back in, so matching `.4em` literally would fail on a reformat and say
    // nothing about §10.4a's turn line.
    expect(base).toContain('@keyframes turnIn');
    expect(base).toMatch(/letter-spacing:\s*0?\.4em/);
    expect(base).toMatch(/letter-spacing:\s*0?\.02em/);
  });
});
