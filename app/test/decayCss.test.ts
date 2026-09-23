import { readdirSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { readScannedFile } from '../../scripts/lib/read-scanned.mjs';
import { DECAY_ALPHA_MAX } from '../src/renderer/decay/layers';

/**
 * **`AC-P3-33-10`** — the exemption withdrawal, priced.
 *
 * Criterion 46's exemption used to cover *material layers*, so the five colour literals a layer
 * author was about to hard-code were pre-exempted by the bar written to catch them. With the
 * clause struck, every colour in a `.cdt-decay` rule must resolve to one of five declared
 * tokens, and every alpha must sit under one ceiling whose owner is `decay/layers.ts`.
 *
 * **Never assert a CSS literal as a string.** The Write/Edit auto-format hook rewrites `.css` on
 * write and `fmt:check` does not cover it, so alphas are compared as **numbers** and colours by
 * **token name**.
 */
const RENDERER = fileURLToPath(new URL('../src/renderer/', import.meta.url));

/** The five §33.9 tokens and nothing else. */
const LAYER_TOKENS = new Set(['--dust', '--silver', '--rust', '--fail', '--growth']);

function cssFiles(dir: string): string[] {
  const out: string[] = [];
  for (const entry of readdirSync(dir)) {
    const full = join(dir, entry);
    // `readScannedFile` guards the read; the walk itself can still race a probe file another
    // gate planted, so a vanished directory entry is skipped here too.
    let isDir = false;
    try {
      isDir = statSync(full).isDirectory();
    } catch {
      continue;
    }
    if (isDir) out.push(...cssFiles(full));
    else if (entry.endsWith('.css')) out.push(full);
  }
  return out;
}

interface DecayRule {
  readonly file: string;
  readonly selector: string;
  readonly body: string;
}

function decayRules(): { rules: DecayRule[]; filesScanned: number } {
  const files = cssFiles(RENDERER);
  const rules: DecayRule[] = [];
  let filesScanned = 0;
  for (const file of files) {
    const text = readScannedFile(file);
    // A file that vanished mid-walk carries nothing to check, so it is skipped BEFORE being
    // counted — otherwise the "scanned nothing" guard below stops meaning what it says.
    if (text === null) continue;
    filesScanned += 1;
    const withoutComments = text.replace(/\/\*[\s\S]*?\*\//gu, '');
    for (const match of withoutComments.matchAll(/([^{}]+)\{([^{}]*)\}/gu)) {
      const selector = (match[1] ?? '').trim();
      if (!selector.includes('.cdt-decay')) continue;
      rules.push({ file, selector, body: match[2] ?? '' });
    }
  }
  return { rules, filesScanned };
}

describe('the decay stylesheet', () => {
  it('ac_p3_33_10 scans the renderer tree for .cdt-decay rules, failing at zero', () => {
    const { rules, filesScanned } = decayRules();
    console.error(
      `AC-P3-33-10: ${String(rules.length)} .cdt-decay rule(s) over ${String(filesScanned)} css file(s)`,
    );
    expect(filesScanned, 'the walk read no stylesheet at all').toBeGreaterThan(0);
    expect(rules.length, 'no .cdt-decay rule was found — the gate scanned nothing').toBeGreaterThan(
      0,
    );
  });

  it('ac_p3_33_10 keeps the paint in decay.css and the clamp in motion.css', () => {
    // Two files by design, not one. `decay.css` is the only stylesheet that PAINTS a layer;
    // `motion.css` carries the tier-clamp selector rows, which R112 makes p3-33's for phase 3.
    // A third file growing a `.cdt-decay` rule is what this catches.
    const { rules } = decayRules();
    // Forward slashes: on Windows the relative path reads `styles\decay.css`.
    const files = [
      ...new Set(rules.map((r) => r.file.replace(RENDERER, '').replaceAll('\\', '/'))),
    ].sort();
    expect(files).toEqual(['styles/decay.css', 'styles/motion.css']);

    for (const rule of rules.filter((r) => r.file.endsWith('motion.css'))) {
      expect(/color|background|var\(--(?!cdt-)/iu.test(rule.body), rule.selector).toBe(false);
    }
  });

  it('ac_p3_33_10 resolves every colour to one of the five §33.9 tokens', () => {
    const { rules } = decayRules();
    for (const rule of rules) {
      for (const found of rule.body.matchAll(/var\((--[a-z0-9-]+)/gu)) {
        const name = found[1] ?? '';
        // `--cdt-decay-drift` is geometry, not a colour, and is read by `transform`.
        if (name.startsWith('--cdt-')) continue;
        expect(LAYER_TOKENS.has(name), `${rule.selector}: ${name} is not a §33.9 layer token`).toBe(
          true,
        );
      }
      // No hex, no rgb(), no hsl() — the withdrawal has teeth only if this holds.
      expect(/#[0-9a-f]{3,8}\b/iu.test(rule.body), `${rule.selector} carries a hex literal`).toBe(
        false,
      );
      expect(/\b(?:rgba?|hsla?)\s*\(/iu.test(rule.body), `${rule.selector} carries rgb/hsl`).toBe(
        false,
      );
    }
  });

  it('ac_p3_33_10 keeps every alpha under the ceiling, compared as a number', () => {
    const { rules } = decayRules();
    let alphas = 0;
    for (const rule of rules) {
      for (const found of rule.body.matchAll(
        /color-mix\(\s*in\s+srgb\s*,\s*var\(--[a-z0-9-]+\)\s+([\d.]+)%/gu,
      )) {
        const percent = Number(found[1]);
        expect(Number.isFinite(percent), `${rule.selector}: unreadable alpha`).toBe(true);
        expect(
          percent / 100,
          `${rule.selector}: ${String(percent)}% exceeds DECAY_ALPHA_MAX`,
        ).toBeLessThanOrEqual(DECAY_ALPHA_MAX);
        alphas += 1;
      }
    }
    console.error(`AC-P3-33-10: ${String(alphas)} alpha stop(s) under ${String(DECAY_ALPHA_MAX)}`);
    expect(alphas, 'no alpha was read — the ceiling gated nothing').toBeGreaterThan(0);
  });
});
