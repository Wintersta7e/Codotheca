import type { IgnoredReason, IgnoredTerm, QueryAst, QueryTerm } from './ast.js';
import { SIZE_UNIT_BYTES, TOUCHED_UNIT_DAYS } from './grammar.js';

function quoteIfNeeded(value: string, quoted: boolean): string {
  if (!quoted && !/[\s"]/.test(value)) return value;
  return `"${value.replace(/"/g, '\\"')}"`;
}

/** Largest unit that divides exactly, so `10mb` comes back as `10mb` and not `10240kb`. */
function scaleDown(total: number, table: Readonly<Record<string, number>>): string {
  const units = Object.entries(table).sort((a, b) => b[1] - a[1]);
  for (const [unit, size] of units) {
    if (total % size === 0) return `${String(total / size)}${unit}`;
  }
  return String(total);
}

export function renderTerm(term: QueryTerm): string {
  const dash = term.negated ? '-' : '';
  switch (term.kind) {
    case 'bare':
      return `${dash}${quoteIfNeeded(term.text, false)}`;
    case 'text':
      return `${dash}${term.field}:${quoteIfNeeded(term.value, term.quoted)}`;
    case 'flag':
      return `${dash}is:${term.flag}`;
    case 'has':
      return `${dash}has:${term.attribute}`;
    case 'size':
      return `${dash}size:${term.op === 'gt' ? '>' : '<'}${scaleDown(term.bytes, SIZE_UNIT_BYTES)}`;
    case 'touchedAge':
      return `${dash}touched:${term.op === 'gt' ? '>' : '<'}${scaleDown(term.days, TOUCHED_UNIT_DAYS)}`;
    case 'touchedYear':
      return `${dash}touched:${String(term.year)}`;
    // [p3] §31.1: a bare count, with no unit to scale down — the quantity is a number of checks.
    case 'completion':
      return `${dash}completion:${term.op === 'gt' ? '>' : '<'}${String(term.value)}`;
  }
}

export function effectiveQueryText(ast: QueryAst): string {
  return ast.terms.map(renderTerm).join(' ');
}

export function freeTextOf(ast: QueryAst): string {
  return ast.terms
    .filter((t) => t.kind === 'bare')
    .map(renderTerm)
    .join(' ');
}

export type PillState = 'accepted' | 'negated' | 'error';

export interface Pill {
  readonly key: string;
  readonly label: string;
  readonly state: PillState;
  readonly reason: string | null;
}

const REASON_LABELS: Readonly<Record<IgnoredReason, string>> = {
  unknownField: 'NOT A FIELD',
  notComputed: 'NOT COMPUTED IN THIS RELEASE',
  malformedValue: 'NOT A VALUE FOR THIS FIELD',
  notAvailable: 'NOT AVAILABLE IN THIS RELEASE',
};

export function ignoredReasonLabel(reason: IgnoredReason): string {
  return REASON_LABELS[reason];
}

/** §8.3a: the input holds free text only; a bare term is never pilled. */
export function pillsOf(ast: QueryAst, extraIgnored: readonly IgnoredTerm[]): readonly Pill[] {
  const pills: Pill[] = [];
  ast.terms.forEach((term, index) => {
    if (term.kind === 'bare') return;
    pills.push({
      key: `t${String(index)}`,
      label: renderTerm(term),
      state: term.negated ? 'negated' : 'accepted',
      reason: null,
    });
  });
  ast.ignored.forEach((entry, index) => {
    pills.push({
      key: `i${String(index)}`,
      label: entry.text,
      state: 'error',
      reason: ignoredReasonLabel(entry.reason),
    });
  });
  extraIgnored.forEach((entry, index) => {
    pills.push({
      key: `x${String(index)}`,
      label: entry.text,
      state: 'error',
      reason: ignoredReasonLabel(entry.reason),
    });
  });
  return pills;
}

export function ignoredClause(ignored: readonly IgnoredTerm[]): string {
  if (ignored.length === 0) return '';
  return ` · ${String(ignored.length)} ignored: ${ignored.map((i) => i.text).join(', ')}`;
}
