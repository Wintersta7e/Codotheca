import type { IgnoredTerm, QueryAst, QueryTerm, TextField } from './ast.js';
import {
  NEVER_EVALUATED,
  QUERY_GRAMMAR_VERSION,
  SIZE_UNIT_BYTES,
  TOUCHED_UNIT_DAYS,
  isHasAttribute,
  isIsFlag,
  isQueryField,
} from './grammar.js';
import { splitFieldValue, tokenizeQuery, unquote } from './tokenize.js';

const TEXT_FIELDS: readonly string[] = ['lang', 'owner', 'in', 'collection'];
const COMPARISON = /^([<>])(\d+)([a-z]+)$/;
const YEAR = /^\d{4}$/;
/** A prefix that reads as a field name: two or more letters. One letter is a drive letter. */
const FIELD_SHAPED = /^[A-Za-z]{2,}$/;

export function parseQuery(input: string): QueryAst {
  const terms: QueryTerm[] = [];
  const ignored: IgnoredTerm[] = [];

  for (const token of tokenizeQuery(input)) {
    const negated = token.text.startsWith('-');
    const body = negated ? token.text.slice(1) : token.text;
    if (body.length === 0) continue;

    const split = splitFieldValue(body);
    if (split === null) {
      terms.push({ kind: 'bare', negated, text: unquote(body).value.toLowerCase() });
      continue;
    }

    const field = split.field.toLowerCase();
    if (!isQueryField(field)) {
      if (FIELD_SHAPED.test(split.field) && !split.value.startsWith('//')) {
        ignored.push({ text: token.text, reason: 'unknownField' });
      } else {
        terms.push({ kind: 'bare', negated, text: unquote(body).value.toLowerCase() });
      }
      continue;
    }

    if (NEVER_EVALUATED.includes(field)) {
      ignored.push({ text: token.text, reason: 'notComputed' });
      continue;
    }

    const term = parseFieldTerm(field, split.value, split.quoted, negated);
    if (term === null) ignored.push({ text: token.text, reason: 'malformedValue' });
    else terms.push(term);
  }

  return { grammarVersion: QUERY_GRAMMAR_VERSION, terms, ignored };
}

function parseFieldTerm(
  field: string,
  raw: string,
  quoted: boolean,
  negated: boolean,
): QueryTerm | null {
  if (raw.length === 0) return null;

  if (TEXT_FIELDS.includes(field)) {
    const value = quoted ? raw : raw.toLowerCase();
    return { kind: 'text', negated, field: field as TextField, value, quoted };
  }

  if (field === 'is') {
    const flag = raw.toLowerCase();
    return isIsFlag(flag) ? { kind: 'flag', negated, flag } : null;
  }

  if (field === 'has') {
    const attribute = raw.toLowerCase();
    return isHasAttribute(attribute) ? { kind: 'has', negated, attribute } : null;
  }

  if (field === 'size') {
    const m = COMPARISON.exec(raw.toLowerCase());
    if (m === null) return null;
    const [, sign, digits, unit] = m;
    const scale = (SIZE_UNIT_BYTES as Record<string, number | undefined>)[unit ?? ''];
    if (scale === undefined || sign === undefined || digits === undefined) return null;
    return { kind: 'size', negated, op: sign === '>' ? 'gt' : 'lt', bytes: Number(digits) * scale };
  }

  // [p3] §31.1: a bare integer bound with no unit, because the quantity is a COUNT OF CHECKS —
  // `completion:>5kb` is not a thing to admit, so this parses its own two characters rather
  // than borrowing `COMPARISON`, which requires a unit.
  if (field === 'completion') {
    const m = /^([<>])(\d+)$/u.exec(raw.toLowerCase());
    const sign = m?.[1];
    const digits = m?.[2];
    if (sign === undefined || digits === undefined) return null;
    return { kind: 'completion', negated, op: sign === '>' ? 'gt' : 'lt', value: Number(digits) };
  }

  if (field === 'touched') {
    const lowered = raw.toLowerCase();
    if (YEAR.test(lowered)) return { kind: 'touchedYear', negated, year: Number(lowered) };
    const m = COMPARISON.exec(lowered);
    if (m === null) return null;
    const [, sign, digits, unit] = m;
    const scale = (TOUCHED_UNIT_DAYS as Record<string, number | undefined>)[unit ?? ''];
    if (scale === undefined || sign === undefined || digits === undefined) return null;
    return {
      kind: 'touchedAge',
      negated,
      op: sign === '>' ? 'gt' : 'lt',
      days: Number(digits) * scale,
    };
  }

  return null;
}
