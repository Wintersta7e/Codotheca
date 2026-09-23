/**
 * A `node:test` reporter that writes the acceptance capture: one row per test, `[{ id, status }]`,
 * where `id` is `<file relative to the repository>::<every enclosing suite and the test, joined by
 * a space>` — the file plus the name, the way cargo's rows are the binary plus the function.
 *
 * Built on the runner's own reporter API, because the built-in `junit` reporter records a test's
 * name and not its file, so two files holding one test name would join to one row.
 *
 *   node --test --test-reporter=<this file> --test-reporter-destination=<capture> ...
 */
import { relative, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../..', import.meta.url));

function fileKey(file) {
  if (typeof file !== 'string' || file.length === 0) return '<unknown>';
  const rel = relative(root, file);
  return (rel.startsWith('..') ? file : rel).split(sep).join('/');
}

export default async function* nodeCapture(source) {
  const stacks = new Map();
  const rows = [];
  for await (const event of source) {
    const data = event.data ?? {};
    const file = fileKey(data.file);
    if (event.type === 'test:start') {
      const stack = stacks.get(file) ?? [];
      stack.length = data.nesting;
      stack.push(data.name);
      stacks.set(file, stack);
      continue;
    }
    if (event.type !== 'test:pass' && event.type !== 'test:fail') continue;
    // A suite reports a pass or a failure of its own; it is not a test and carries no id.
    if (data.details?.type === 'suite') continue;
    const ancestors = (stacks.get(file) ?? []).slice(0, data.nesting);
    // `skip` and `todo` carry a reason string, possibly empty, or `true`; absent means neither.
    const marked = (v) => v !== undefined && v !== false;
    const status =
      event.type === 'test:fail'
        ? 'failed'
        : marked(data.skip) || marked(data.todo)
          ? 'skipped'
          : 'passed';
    rows.push({ id: `${file}::${[...ancestors, data.name].join(' ')}`, status });
  }
  yield `${JSON.stringify(rows, null, 2)}\n`;
}
