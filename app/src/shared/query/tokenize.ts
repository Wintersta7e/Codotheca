export interface RawToken {
  readonly text: string;
  readonly start: number;
  readonly end: number;
}

const QUOTE = '"';
const BACKSLASH = '\\';

/** §8.3: quoting is `"…"` with `\"` as the only escape. Whitespace inside quotes does not split. */
export function tokenizeQuery(input: string): readonly RawToken[] {
  const tokens: RawToken[] = [];
  let index = 0;
  while (index < input.length) {
    while (index < input.length && /\s/.test(input.charAt(index))) index += 1;
    if (index >= input.length) break;
    const start = index;
    let inQuotes = false;
    while (index < input.length) {
      const ch = input.charAt(index);
      if (inQuotes && ch === BACKSLASH && index + 1 < input.length) {
        index += 2;
        continue;
      }
      if (ch === QUOTE) {
        inQuotes = !inQuotes;
        index += 1;
        continue;
      }
      if (!inQuotes && /\s/.test(ch)) break;
      index += 1;
    }
    tokens.push({ text: input.slice(start, index), start, end: index });
  }
  return tokens;
}

export function unquote(value: string): { readonly value: string; readonly quoted: boolean } {
  if (value.length >= 2 && value.startsWith(QUOTE) && value.endsWith(QUOTE)) {
    return { value: value.slice(1, -1).replace(/\\"/g, QUOTE), quoted: true };
  }
  if (value.startsWith(QUOTE)) {
    // Unterminated: take what is there rather than dropping the term silently.
    return { value: value.slice(1).replace(/\\"/g, QUOTE), quoted: true };
  }
  return { value, quoted: false };
}

/** Splits at the **first** colon at index > 0, so `in:wsl:ubuntu` keeps its second colon. */
export function splitFieldValue(
  token: string,
): { readonly field: string; readonly value: string; readonly quoted: boolean } | null {
  const colon = token.indexOf(':');
  if (colon <= 0) return null;
  const { value, quoted } = unquote(token.slice(colon + 1));
  return { field: token.slice(0, colon), value, quoted };
}
