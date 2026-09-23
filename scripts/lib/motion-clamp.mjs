/**
 * §11.6's tier clamp, **read out of the stylesheets rather than enumerated anywhere**.
 *
 * `motion.css` names class names, and a name in no rule is in no clamp: `reduced` has no blanket
 * rule at all, so a new animated class is unclamped there by default and plays its full envelope
 * to a user who asked their operating system for reduced motion. `AC-50-zero-animation` once held
 * a hand-maintained list of the names and sat at nine against a real ten; this module is the one
 * parser both halves of the bar read — `app/test/tierClamp.test.ts` (which group a name joins)
 * and `scripts/check-motion-clamp.mjs` (that every animated class joins one).
 *
 * Durations are compared as numbers: the formatter rewrites `.css` and a duration written in `ms`
 * can come back in `s`.
 */

/** Every `/* … *\/` comment removed, so a selector in prose is never read as a rule. */
export function withoutCssComments(css) {
  return css.replace(/\/\*[\s\S]*?\*\//gu, '');
}

/**
 * The style rules of a stylesheet, with the at-rules around each.
 *
 * `@keyframes` bodies are skipped — their percentages are not selectors — and a rule nested in
 * `@media` keeps the condition in `context`, because a clamp inside a media query applies only
 * when the query does.
 */
export function cssRules(css) {
  const text = withoutCssComments(css);
  const rules = [];
  const walk = (from, to, context) => {
    let at = from;
    while (at < to) {
      const open = text.indexOf('{', at);
      if (open === -1 || open >= to) return;
      const prelude = text.slice(at, open).trim();
      const close = matching(text, open);
      if (close === -1 || close > to) return;
      if (prelude.startsWith('@')) {
        if (!/^@(-[a-z]+-)?keyframes\b/u.test(prelude))
          walk(open + 1, close, [...context, prelude]);
      } else if (prelude !== '') {
        rules.push({
          selectors: prelude
            .split(',')
            .map((s) => s.trim())
            .filter((s) => s !== ''),
          body: text.slice(open + 1, close),
          context,
        });
      }
      at = close + 1;
    }
  };
  walk(0, text.length, []);
  return rules;
}

function matching(text, open) {
  let depth = 0;
  for (let i = open; i < text.length; i += 1) {
    if (text[i] === '{') depth += 1;
    else if (text[i] === '}') {
      depth -= 1;
      if (depth === 0) return i;
    }
  }
  return -1;
}

/** `property: value` pairs of one rule body, in order. */
export function declarations(body) {
  const out = [];
  for (const part of body.split(';')) {
    const colon = part.indexOf(':');
    if (colon === -1) continue;
    const property = part.slice(0, colon).trim().toLowerCase();
    const value = part.slice(colon + 1).trim();
    if (property !== '') out.push({ property, value });
  }
  return out;
}

/** Every time value in `value`, in milliseconds. `.13s` and `130ms` are one value. */
export function durationsMs(value) {
  const out = [];
  for (const m of value.matchAll(/(?<![\w.-])(\d*\.?\d+)(ms|s)\b/giu)) {
    const amount = Number.parseFloat(m[1]);
    out.push(m[2].toLowerCase() === 's' ? amount * 1000 : amount);
  }
  return out;
}

/**
 * The durations a `transition` shorthand runs for: the **first** time value of each
 * comma-separated transition, because the second is its delay.
 */
export function transitionDurationsMs(value) {
  return splitTopLevel(value).map((item) => durationsMs(item)[0] ?? 0);
}

function splitTopLevel(value) {
  const out = [];
  let depth = 0;
  let start = 0;
  for (let i = 0; i < value.length; i += 1) {
    if (value[i] === '(') depth += 1;
    else if (value[i] === ')') depth -= 1;
    else if (value[i] === ',' && depth === 0) {
      out.push(value.slice(start, i));
      start = i + 1;
    }
  }
  out.push(value.slice(start));
  return out.map((s) => s.trim()).filter((s) => s !== '');
}

/** The tier a selector is scoped to, or `null` when it names none. */
export function tierOf(selector) {
  const m = /\[data-effects-tier\s*=\s*['"]?([a-z]+)['"]?\s*\]/u.exec(selector);
  return m === null ? null : m[1];
}

/**
 * The class names of the element a selector styles — its last compound, which is the element
 * that carries the effect. `.cdt-card[data-hovered='true'] .cdt-specular` animates
 * `.cdt-specular`, and clamping `.cdt-card` for it is exactly the `.cdt-plate`-for-`.cdt-specular`
 * mistake: a name in the set that is not the element carrying the effect.
 */
export function subjectClasses(selector) {
  const compounds = selector
    .replace(/\[[^\]]*\]/gu, (m) => m.replace(/[\s>+~]/gu, '_'))
    .split(/[\s>+~]+/u)
    .filter((c) => c !== '');
  const last = compounds.at(-1) ?? '';
  return [...last.matchAll(/\.([a-zA-Z_][\w-]*)/gu)].map((m) => m[1]);
}

/**
 * Every class name a `[data-effects-tier=…]` rule selects, bucketed by what the rule declares.
 *
 * `clamped` is a transition with a **duration**, which is what §11.6 means by clamping. The
 * `transition: none` rules declare the same property and clamp nothing, so they are deliberately
 * not in that bucket — landing a name there produces no transition at `reduced` at all, which
 * resolves as a plausible style and is exactly the mistake a line-range edit makes.
 */
export function clampClassNames(css) {
  const displayNone = new Set();
  const noTransform = new Set();
  const clamped = new Set();
  const all = new Set();

  for (const rule of cssRules(css)) {
    const names = new Set();
    for (const selector of rule.selectors) {
      if (!selector.includes('[data-effects-tier')) continue;
      for (const found of selector.matchAll(/\.([a-z][a-z0-9-]*)/gu)) names.add(found[1]);
    }
    if (names.size === 0) continue;
    const body = rule.body.trim();
    const declares = (property, value) =>
      new RegExp(`(^|;)\\s*${property}\\s*:\\s*${value}\\s*(;|$)`, 'u').test(body);
    const isClamped = /(^|;)\s*transition\s*:\s*[a-z-]+\s+\d/u.test(body);
    for (const name of names) {
      all.add(name);
      if (declares('display', 'none')) displayNone.add(name);
      if (declares('transform', 'none')) noTransform.add(name);
      if (isClamped) clamped.add(name);
    }
  }

  return { displayNone, noTransform, clamped, all };
}

/** A pseudo-class whose weight is its argument's. None occurs in the tree; one is refused. */
export const UNWEIGHED_PSEUDO = /:(is|not|where|has)\(/u;

/**
 * One selector's specificity, `[ids, classes, types]` — attribute selectors and pseudo-classes
 * weigh as classes, pseudo-elements as types, `*` as nothing. **Per selector, never per rule**: a
 * browser weighs the selector in a comma list that matched, not the heaviest one beside it.
 */
export function specificity(selector) {
  let ids = 0;
  let classes = 0;
  let types = 0;
  let rest = selector.replace(/\[[^\]]*\]/gu, () => {
    classes += 1;
    return ' ';
  });
  rest = rest.replace(/#[\w-]+/gu, () => {
    ids += 1;
    return ' ';
  });
  rest = rest.replace(/\.[\w-]+/gu, () => {
    classes += 1;
    return ' ';
  });
  rest = rest.replace(/::[\w-]+/gu, () => {
    types += 1;
    return ' ';
  });
  rest = rest.replace(/:[\w-]+(\([^)]*\))?/gu, () => {
    classes += 1;
    return ' ';
  });
  types += [...rest.matchAll(/(?:^|[\s>+~])[a-zA-Z][\w-]*/gu)].length;
  return [ids, classes, types];
}

/** Negative, zero or positive, as `a` is weaker than, equal to or stronger than `b`. */
export function compareSpecificity(a, b) {
  for (let i = 0; i < 3; i += 1) {
    if ((a[i] ?? 0) !== (b[i] ?? 0)) return (a[i] ?? 0) - (b[i] ?? 0);
  }
  return 0;
}

/**
 * What a rule sets moving: `transform` (anything but `none`), `animation` (anything but `none`),
 * or `transition` (a duration over the clamp).
 */
export function motionFamilies(body, clampMs) {
  const out = new Set();
  for (const { property, value } of declarations(body)) {
    const none = /^none\b/iu.test(value);
    if (property === 'transform' && !none) out.add('transform');
    else if ((property === 'animation' || property === 'animation-name') && !none) {
      out.add('animation');
    } else if (
      property === 'transition' &&
      transitionDurationsMs(value).some((ms) => ms > clampMs)
    ) {
      out.add('transition');
    } else if (
      property === 'transition-duration' &&
      durationsMs(value).some((ms) => ms > clampMs)
    ) {
      out.add('transition');
    }
  }
  return out;
}

/**
 * What a rule scoped to `tier` clamps: `transform: none`; an animation that is `none`, or at
 * `reduced` a finite one within the clamp; a transition that is `none`, or at `reduced` within the
 * clamp. `display: none` clamps everything, since nothing on an absent element moves.
 */
export function clampFamilies(body, tier, clampMs) {
  const out = new Set();
  for (const { property, value } of declarations(body)) {
    const none = /^none\b/iu.test(value);
    if (property === 'display' && none) out.add('all');
    else if (property === 'transform' && none) out.add('transform');
    else if (property === 'animation' || property === 'animation-name') {
      const clamped =
        tier === 'reduced' &&
        property === 'animation' &&
        !/\binfinite\b/iu.test(value) &&
        (durationsMs(value)[0] ?? Infinity) <= clampMs;
      if (none || clamped) out.add('animation');
    } else if (property === 'transition') {
      const clamped =
        tier === 'reduced' && transitionDurationsMs(value).every((ms) => ms <= clampMs);
      if (none || clamped) out.add('transition');
    }
  }
  return out;
}
