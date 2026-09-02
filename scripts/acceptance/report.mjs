/**
 * Two renderings. `DISPOSITIONS.md` is a pure function of the registry and is committed and
 * diff-gated, so the prose statement of what is automated cannot drift from the JSON. The run
 * report carries results and is an artifact.
 */
import { STATUSES, rollUp } from './registry.mjs';

export function countByStatus(registry) {
  const counts = Object.fromEntries(STATUSES.map((s) => [s, 0]));
  for (const entry of registry.criteria)
    for (const check of entry.checks) counts[check.status] += 1;
  return counts;
}

function rolledUpCounts(registry) {
  const counts = Object.fromEntries(STATUSES.map((s) => [s, 0]));
  for (const entry of registry.criteria) counts[rollUp(entry)] += 1;
  return counts;
}

function evidence(check) {
  if (check.status === 'automated') return `runs now — \`${check.test}\``;
  if (check.status === 'deferred') return `plan ${check.owner} — \`${check.test}\``;
  if (check.status === 'manual') return `gate \`${check.gate}\``;
  return 'not gated';
}

export function renderDispositions(registry) {
  const lines = [];
  lines.push('# Acceptance dispositions');
  lines.push('');
  lines.push(
    '**Generated from `criteria.json` by `npm run acceptance -- --dispositions`. Do not edit.**',
  );
  lines.push('');
  lines.push('One row per criterion; the disposition is the weakest of its checks.');
  lines.push('');
  const rolled = rolledUpCounts(registry);
  const checks = countByStatus(registry);
  lines.push('| Disposition | Criteria | Checks |');
  lines.push('|---|---|---|');
  for (const status of STATUSES) {
    lines.push(`| ${status} | ${String(rolled[status])} | ${String(checks[status])} |`);
  }
  lines.push('');
  lines.push('| # | Disposition | Title | Checks |');
  lines.push('|---|---|---|---|');
  for (const entry of registry.criteria) {
    const detail = entry.checks.map((c) => `\`${c.id}\` ${c.status}, ${evidence(c)}`).join('<br>');
    lines.push(`| ${entry.id} | ${rollUp(entry)} | ${entry.title} | ${detail} |`);
  }
  lines.push('');
  lines.push('## Why a check is not automated, or is automated over less than it looks');
  lines.push('');
  // Every reason, not only the three statuses that require one. A deferral whose argument is in
  // the JSON and not in the table is an argument nobody reads.
  lines.push('| Check | Status | Reason |');
  lines.push('|---|---|---|');
  for (const entry of registry.criteria) {
    for (const check of entry.checks) {
      if (typeof check.reason !== 'string') continue;
      lines.push(`| \`${check.id}\` | ${check.status} | ${check.reason} |`);
    }
  }
  lines.push('');
  return `${lines.join('\n')}`;
}

export function renderRunReport(registry, join, diff, perf, absent = []) {
  const lines = [];
  lines.push('# Acceptance run');
  lines.push('');
  const byResult = { passed: 0, failed: 0, 'not-run': 0, skipped: 0 };
  for (const check of join.checks) byResult[check.result] = (byResult[check.result] ?? 0) + 1;
  lines.push(`Checks: ${String(join.checks.length)} — ${JSON.stringify(byResult)}`);
  lines.push('');
  if (absent.length > 0) {
    lines.push(
      `**Suites not run here: ${absent.join(', ')}.** Every automated check on those runners is` +
        ' reported as not-run and is not gated on. This is a partial run.',
    );
    lines.push('');
  }
  lines.push('## Failing checks, by name');
  lines.push('');
  const failing = join.checks.filter((c) => c.result === 'failed');
  if (failing.length === 0) lines.push('None.');
  for (const check of failing) {
    lines.push(`- \`${check.id}\` (criterion ${check.criterion}) — \`${String(check.test)}\``);
  }
  lines.push('');
  lines.push('## Automated checks that skipped');
  lines.push('');
  const skipped = join.checks.filter((c) => c.status === 'automated' && c.result === 'skipped');
  if (skipped.length === 0) lines.push('None.');
  for (const check of skipped) {
    lines.push(`- \`${check.id}\` (criterion ${check.criterion}) — \`${String(check.test)}\``);
  }
  lines.push('');
  lines.push('## Baseline diff');
  lines.push('');
  for (const [label, ids] of [
    ['new failures', diff.newFailures],
    ['stale baseline entries', diff.stale],
    ['baseline entries that did not run', diff.missing],
  ]) {
    lines.push(`- ${label}: ${ids.length === 0 ? 'none' : ids.map((i) => `\`${i}\``).join(', ')}`);
  }
  lines.push('');
  lines.push('## Performance');
  lines.push('');
  if (perf.length === 0) {
    lines.push('No performance samples were recorded on this machine. Performance criteria are');
    lines.push('measured on Reference A and Reference B only; a CI runner is neither.');
  }
  for (const row of perf) lines.push(`- \`${row.check}\` on ${row.machine}: ${row.summary}`);
  lines.push('');
  lines.push('## Not verified here');
  lines.push('');
  for (const entry of registry.criteria) {
    for (const check of entry.checks) {
      if (check.status !== 'external') continue;
      lines.push(`- Criterion ${entry.id}: ${check.reason}`);
    }
  }
  lines.push('');
  return `${lines.join('\n')}`;
}
