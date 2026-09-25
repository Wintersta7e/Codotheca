import { afterEach, describe, expect, it } from 'vitest';
// `?raw` rather than node:fs: the renderer project carries no Node types by design, and under
// jsdom `import.meta.url` is not a file URL.
import motionCss from './motion.css?raw';
import css from './projectPage.css?raw';
import { required } from '../../shared/required';

/**
 * Both `?raw` imports, asserted non-empty before anything reads them. Vitest stubs CSS to an
 * empty module unless the file is processed, and that stubbing catches `?raw` too — an empty
 * read makes every assertion below vacuous.
 */
describe('the stylesheet under test actually arrived', () => {
  it('reads real text, because every assertion here is vacuous against ""', () => {
    expect(css.length).toBeGreaterThan(1000);
    expect(motionCss.length).toBeGreaterThan(500);
  });
});

const PAGE_FIXTURE = `
  <div class="cp-page">
    <div class="cp-bar"><span class="cp-bar-path"></span></div>
    <div class="cp-body">
      <div class="cp-col-left"></div>
      <div class="cp-col-right"><div class="cp-rise"></div></div>
    </div>
  </div>`;

/**
 * The `animation` shorthand, not `animationName`: jsdom does not expand a shorthand into its
 * longhands, so `animationName` reads `none` against a rule that really does animate. Measured —
 * an assertion on the longhand fails against correct CSS, which is the wrong half of R36.
 */
interface Resolved {
  readonly page: string;
  readonly rail: string;
  readonly rise: string;
}

/** Both stylesheets, in the order `main.tsx` imports them: the page first, motion last. */
function resolveAt(tier: 'full' | 'reduced' | 'off' | 'auto'): Resolved {
  const style = document.createElement('style');
  style.textContent = `${css}\n${motionCss}`;
  document.head.append(style);
  document.documentElement.setAttribute('data-effects-tier', tier);
  document.body.innerHTML = PAGE_FIXTURE;

  const at = (selector: string): CSSStyleDeclaration => {
    const node = document.querySelector(selector);
    // The fixture must actually carry the element, or every assertion reads a default and passes
    // against nothing — the failure mode this whole block exists to catch.
    if (node === null) throw new Error(`fixture has no ${selector}`);
    return getComputedStyle(node);
  };

  return {
    page: at('.cp-page').animation,
    rail: at('.cp-col-left').animation,
    rise: at('.cp-rise').animation,
  };
}

afterEach(() => {
  document.head.querySelectorAll('style').forEach((node) => {
    node.remove();
  });
  document.documentElement.removeAttribute('data-effects-tier');
  document.body.innerHTML = '';
});

describe('the entry runs at full and at nothing else', () => {
  it('unfolds the page, slides the rail and rises the column at `full`', () => {
    const r = resolveAt('full');
    expect(r.page).toBe('cp-crt-on 620ms cubic-bezier(0.25, 0.8, 0.3, 1) both');
    expect(r.rail).toBe('cp-slide-rail 400ms cubic-bezier(0.2, 0.85, 0.2, 1) both');
    expect(r.rise).toBe('cp-rise-in 420ms cubic-bezier(0.2, 0.85, 0.2, 1) both');
  });

  it('emits no entry frame at `reduced` — the page paints in its final state', () => {
    const r = resolveAt('reduced');
    for (const resolved of [r.page, r.rail, r.rise]) {
      expect(resolved).not.toMatch(/cp-crt-on|cp-slide-rail|cp-rise-in/);
    }
  });

  it('emits no entry frame at `off` either', () => {
    const r = resolveAt('off');
    for (const resolved of [r.page, r.rail, r.rise]) {
      expect(resolved).not.toMatch(/cp-crt-on|cp-slide-rail|cp-rise-in/);
    }
  });

  it('runs on the first frame, before the resolver has replaced `auto`', () => {
    // The shell writes the stored tier before the window exists, so the attribute really is
    // `auto` until the renderer has queried the compositor. A page that opted in at `full` only
    // would drop its own entry on every cold open.
    expect(resolveAt('auto').page).toContain('cp-crt-on');
  });
});

/**
 * jsdom does not evaluate `prefers-reduced-motion`, so the withdrawal at `auto` is asserted
 * structurally through the CSSOM rather than by resolving a style — a substring match on the
 * source would also match this comment.
 */
describe('the `auto` opt-in is withdrawn under a reduced-motion preference', () => {
  it('carries a media rule that stops all four animations', () => {
    const style = document.createElement('style');
    style.textContent = css;
    document.head.append(style);
    const sheet = style.sheet;
    if (sheet === null) throw new Error('the stylesheet did not parse');

    const media = [...sheet.cssRules].filter((r): r is CSSMediaRule => r instanceof CSSMediaRule);
    const reduce = media.find((r) => r.conditionText.includes('prefers-reduced-motion'));
    if (reduce === undefined) throw new Error('no reduced-motion media rule');

    const inner = [...reduce.cssRules].filter((r): r is CSSStyleRule => r instanceof CSSStyleRule);
    expect(inner.length).toBeGreaterThan(0);
    const covered = inner.flatMap((r) => r.selectorText.split(',').map((s) => s.trim()));
    for (const selector of ['.cp-page', '.cp-col-left', '.cp-rise', '.cp-opensin-menu']) {
      expect(covered.some((s) => s.endsWith(selector))).toBe(true);
    }
    for (const r of inner) expect(r.style.getPropertyValue('animation')).toBe('none');
  });
});

describe('what the page may not paint', () => {
  it('states no colour of its own — every one is a token from the palette block', () => {
    const body = css.replace(/\/\*[\s\S]*?\*\//g, '');
    // The gate proves this over the whole renderer; this proves it over the one file that
    // carries §8.5.1's four off-token grounds, which is where a hex would be re-imported.
    expect(body).not.toMatch(/#[0-9a-fA-F]{3,8}\b/);
    for (const snapped of ['#0b0e11', '#f4f7f9', '#e7ebef', '#9aa6b2', '#5f6b76', '#6c7885']) {
      expect(body).not.toContain(snapped);
    }
  });

  it('puts decision-carrying text at the contrast floor and never on an ornament step', () => {
    const body = css.replace(/\/\*[\s\S]*?\*\//g, '');
    // The path line, the roast line and the reroll readout are all decisions.
    expect(body).not.toContain('var(--text-4)');
    for (const rule of ['.cp-bar-path', '.cp-roast-line', '.cp-reroll-readout']) {
      const block = new RegExp(`\\${rule}\\s*\\{([^}]*)\\}`).exec(body)?.[1] ?? '';
      expect(block, `${rule} has no colour`).toContain('var(--text-3)');
    }
  });

  /**
   * Criterion 40's other half. `ActivityTab.test.tsx` proves the two lanes are the slot's own
   * children; this proves the slot lays them out side by side against its own floor, which lives
   * in the stylesheet and cannot be seen from the DOM shape.
   */
  it('lays the two activity lanes across the slot, each on the slot’s own floor', () => {
    const style = document.createElement('style');
    style.textContent = css;
    document.head.append(style);
    document.body.innerHTML = `
      <div class="cp-act-chart">
        <div class="cp-act-slot">
          <div class="cp-act-lane" data-lane="commitDays"></div>
          <div class="cp-act-lane" data-lane="sessions"></div>
        </div>
      </div>`;
    const slot = document.querySelector('.cp-act-slot');
    const lane = document.querySelector('[data-lane="sessions"]');
    if (slot === null || lane === null) throw new Error('fixture has no activity slot');
    const slotStyle = getComputedStyle(slot);
    // A row, not a column: a column is a stack, and a stack reads as a total.
    expect(slotStyle.flexDirection).not.toBe('column');
    expect(slotStyle.alignItems).toBe('flex-end');
    expect(getComputedStyle(lane).alignSelf).toBe('flex-end');
  });
});

/**
 * [p2] §24.8's block, resolved on real elements rather than matched as text. A rule that selects
 * nothing resolves to the default and every assertion about it passes against nothing, which is
 * why each `at()` below throws when the fixture does not carry the node.
 */
describe('the removal block is styled, and states its disposition without colour', () => {
  const REMOVAL_FIXTURE = `
    <div class="cp-rail">
      <div class="cp-uninstall" data-state="blocked">
        <p class="cp-uninstall__heading"></p>
        <ul class="cp-uninstall__reasons"><li></li></ul>
        <p class="cp-uninstall__fate"></p>
        <button class="cp-uninstall__go"></button>
      </div>
      <div class="cp-uninstall" data-state="closed">
        <p class="cp-uninstall__note"></p>
        <button class="cp-uninstall__open"></button>
      </div>
    </div>`;

  function mount(): (selector: string) => CSSStyleDeclaration {
    const style = document.createElement('style');
    style.textContent = css;
    document.head.append(style);
    document.body.innerHTML = REMOVAL_FIXTURE;
    return (selector: string): CSSStyleDeclaration => {
      const node = document.querySelector(selector);
      if (node === null) throw new Error(`fixture has no ${selector}`);
      return getComputedStyle(node);
    };
  }

  it('gives the block a surface, a padded box and an edge of its own', () => {
    const at = mount();
    const block = at('.cp-uninstall[data-state="blocked"]');
    expect(block.display).toBe('flex');
    expect(block.padding).toBe('9px 11px 10px');
    // Longhands resolve here where the `border` shorthand would not: this DOM drops a shorthand
    // carrying `var()` whole, and an assertion on the dropped form reads the initial value and
    // passes against a rule that is not there.
    expect(block.borderTopWidth).toBe('1px');
    expect(block.borderTopStyle).toBe('solid');
  });

  it('renders the opener and the removal as one shape, so the second is not a new control', () => {
    const at = mount();
    const open = at('.cp-uninstall__open');
    const go = at('.cp-uninstall__go');
    expect(open.height).toBe('28px');
    expect(open.height).toBe(go.height);
    expect(open.background).toBe(go.background);
    expect(open.borderTopWidth).toBe(go.borderTopWidth);
    // §24.5 puts the removal where the eye does not land first, so it does not take the fill
    // the page's primary control takes. Compared against that control, never against a literal.
    document.body.innerHTML += '<button class="cp-cta"></button>';
    const cta = getComputedStyle(required(document.querySelector('.cp-cta'), 'CTA'));
    expect(go.background).not.toBe(cta.background);
  });

  /**
   * §24.8: `unknown` renders distinctly from `blocked`, and neither is an error state. Compared
   * as resolved values, never as stylesheet text — a rule whose selector stopped matching would
   * still read correct in the source.
   */
  it('distinguishes unknown from blocked, and paints neither red', () => {
    const style = document.createElement('style');
    style.textContent = css;
    document.head.append(style);
    document.body.innerHTML = `
      <div class="cp-uninstall" id="u" data-state="unknown"></div>
      <div class="cp-uninstall" id="b" data-state="blocked"></div>`;
    const unknown = getComputedStyle(required(document.querySelector('#u'), '#u'));
    const blocked = getComputedStyle(required(document.querySelector('#b'), '#b'));
    expect(unknown.borderLeftWidth).not.toBe(blocked.borderLeftWidth);
    for (const resolved of [unknown, blocked]) {
      expect(resolved.borderLeftColor).not.toMatch(/\bred\b/i);
      expect(resolved.color).not.toMatch(/\bred\b/i);
    }
  });
});
