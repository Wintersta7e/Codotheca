/**
 * Performance samples, and the two things they are not allowed to be.
 *
 * A sample that cannot state its event pair is not a sample — v1 stated bare figures with no
 * event pair and two of them were arithmetically impossible. And a budget evaluated on hardware
 * that has not identified itself SKIPS: a number checked on unknown hardware is one nobody can
 * act on, and a green tick against it retires the question.
 */
import { readFileSync } from 'node:fs';
import { cpus, totalmem } from 'node:os';

export const SKIP_NO_MACHINE =
  'no reference machine resolved — a budget checked on unknown hardware is a number nobody can act on';

export function loadMachines(path) {
  return JSON.parse(readFileSync(path, 'utf8'));
}

export function machineFacts() {
  return {
    platform: process.platform,
    cores: cpus().length,
    memoryGb: Math.round(totalmem() / 1024 ** 3),
  };
}

export function resolveMachine(machines, env, facts) {
  const key = machines.identification.env;
  const claimed = env[key];
  if (claimed === undefined || claimed === '') {
    return { id: null, reason: `${key} is unset; ${SKIP_NO_MACHINE}` };
  }
  const machine = machines.machines.find((m) => m.id === claimed);
  if (machine === undefined) {
    return { id: null, reason: `${key}=${String(claimed)} names no machine in machines.json` };
  }

  for (const field of machines.identification.mustMatch) {
    const expected = machine.profile[field];
    const actual = facts[field];
    const tolerance = field === 'memoryGb' ? machines.identification.memoryToleranceGb : 0;
    const ok =
      typeof expected === 'number' ? Math.abs(expected - actual) <= tolerance : expected === actual;
    if (!ok) {
      return {
        id: null,
        reason: `${key}=${machine.id} but ${field} is ${String(actual)}, not ${String(expected)} — the claim is refused`,
      };
    }
  }
  return { id: machine.id, reason: `${key}=${machine.id}, fingerprint matches` };
}

export function newSample(spec) {
  const kind = spec.kind;
  if (!['latency', 'duration', 'rate', 'size'].includes(kind)) {
    throw new TypeError(`unknown measurement kind ${String(kind)}`);
  }
  if (['latency', 'duration'].includes(kind)) {
    if (typeof spec.from !== 'string' || spec.from.length === 0) {
      throw new TypeError(`${String(spec.check)}: a ${kind} states the event it measures from`);
    }
    if (typeof spec.to !== 'string' || spec.to.length === 0) {
      throw new TypeError(`${String(spec.check)}: a ${kind} states the event it measures to`);
    }
  }
  if (kind === 'rate' && (typeof spec.run !== 'string' || spec.run.length === 0)) {
    throw new TypeError(
      `${String(spec.check)}: a rate states its run definition instead of an event pair`,
    );
  }
  if (!['warm', 'cold', 'n/a'].includes(spec.thermal)) {
    throw new TypeError(`${String(spec.check)}: thermal is warm, cold or n/a`);
  }
  return { ...spec, observations: [] };
}

export function addObservation(sample, value) {
  if (!Number.isFinite(value)) {
    throw new TypeError(`${String(sample.check)}: an observation is a finite number`);
  }
  return { ...sample, observations: [...sample.observations, value] };
}

export function percentile(values, p) {
  if (values.length === 0)
    throw new RangeError('a percentile over no observations is not a number');
  const sorted = [...values].sort((a, b) => a - b);
  const rank = Math.ceil((p / 100) * sorted.length) - 1;
  return sorted[Math.min(Math.max(rank, 0), sorted.length - 1)];
}

export function summarize(sample) {
  const n = sample.observations.length;
  const p50 = n === 0 ? Number.NaN : percentile(sample.observations, 50);
  const p95 = n === 0 ? Number.NaN : percentile(sample.observations, 95);
  const pair =
    sample.kind === 'rate' ? sample.run : `${String(sample.from)} → ${String(sample.to)}`;
  return {
    check: sample.check,
    machine: sample.machine ?? 'unknown',
    n,
    p50,
    p95,
    summary: `${pair} · ${sample.thermal} · n=${String(n)} · p50 ${String(p50)} · p95 ${String(p95)}`,
  };
}

export function evaluateBudget(sample, budget, machineId) {
  if (machineId === null) return { status: 'skipped', reason: SKIP_NO_MACHINE };
  if (budget.machine !== undefined && budget.machine !== machineId) {
    return {
      status: 'skipped',
      reason: `budget is stated for ${String(budget.machine)}; this is ${machineId}`,
    };
  }
  if (sample.observations.length === 0) {
    return { status: 'skipped', reason: `${String(sample.check)} recorded no observations` };
  }
  const measured =
    budget.metric === 'p50'
      ? percentile(sample.observations, 50)
      : budget.metric === 'p95'
        ? percentile(sample.observations, 95)
        : Math.max(...sample.observations);
  const ok = budget.op === '<' ? measured < budget.value : measured <= budget.value;
  return {
    status: ok ? 'passed' : 'failed',
    reason: `${budget.metric} ${String(measured)} ${budget.op} ${String(budget.value)} on ${machineId}`,
  };
}
