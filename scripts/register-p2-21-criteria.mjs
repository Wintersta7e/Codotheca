/**
 * Register §21's fifteen acceptance criteria, plus R56's live-observation floor.
 *
 * **Written as a script and not by hand, deliberately.** The Write/Edit auto-format hook reflows
 * `acceptance/criteria.json` — it is not prettier-clean and never has been — so a hand edit
 * rewrites the whole file and buries the fifteen rows this adds in three hundred lines of
 * whitespace. This appends, preserving the file's own formatting for everything it does not
 * touch, and refuses to run twice.
 *
 * Run once, from the repository root:
 *   node scripts/register-p2-21-criteria.mjs
 */
import { readFileSync, writeFileSync } from 'node:fs';

const PATH = 'acceptance/criteria.json';

/** §21.16's fifteen, in order, each with the test that carries it. */
const CRITERIA = [
  {
    id: 'P2-21-1',
    title: 'The sync and job vocabularies share no slug',
    spec: '§21.16',
    checks: [
      {
        id: 'AC-P2-21-1',
        status: 'automated',
        runner: 'cargo',
        owner: 'p2-21',
        test: 'sync_schema::the_sync_and_job_vocabularies_are_disjoint',
        assert:
          'Walks JobKind::ALL and SyncTaskKind::ALL, prints the count of slugs on each side and fails at zero on either, and asserts no slug appears in both.',
        scanning: true,
      },
    ],
  },
  {
    id: 'P2-21-2',
    title: 'Every stored slug is accepted by its column',
    spec: '§21.16',
    checks: [
      {
        id: 'AC-P2-21-2',
        status: 'automated',
        runner: 'cargo',
        owner: 'p2-21',
        test: 'sync_schema::every_sync_slug_is_accepted_by_its_column',
        assert:
          'Inserts every (task, state) pair into sync_task_state on a migrated database, prints the count inserted and fails at zero, so a CHECK that rejects a value the core emits is caught.',
        scanning: true,
      },
      {
        id: 'AC-P2-21-2-mirror',
        status: 'automated',
        runner: 'cargo',
        owner: 'p2-21',
        test: 'sync_schema::every_slug_round_trips_through_the_generated_enum',
        assert:
          'Each slug function agrees with the generated serde spelling in both directions, so the Rust and wire vocabularies are one value with one owner.',
        source: 'schema',
      },
    ],
  },
  {
    id: 'P2-21-3',
    title: 'The three 403 shapes settle three different ways',
    spec: '§21.16',
    checks: [
      {
        id: 'AC-P2-21-3-primary',
        status: 'automated',
        runner: 'cargo',
        owner: 'p2-21',
        test: 'sync_state::a_primary_yield_parks_to_the_reset_and_moves_neither_counter',
        assert:
          'A 403 carrying x-ratelimit-remaining: 0 parks to reset_at with fail_count and throttle_count both unchanged.',
      },
      {
        id: 'AC-P2-21-3-secondary',
        status: 'automated',
        runner: 'cargo',
        owner: 'p2-21',
        test: 'sync_state::a_secondary_limit_increments_its_own_counter_and_lengthens_the_park',
        assert:
          'A 403 carrying retry-after with remaining above zero parks and increments throttle_count, floored at 60 s and doubled on the second consecutive one.',
      },
      {
        id: 'AC-P2-21-3-terminal',
        status: 'automated',
        runner: 'cargo',
        owner: 'p2-21',
        test: 'sync_state::a_terminal_403_blocks_with_no_scheduled_retry',
        assert:
          'A 403 carrying neither header leaves the task blocked with no not_before and no scheduled retry, and no passage of time makes it runnable.',
      },
    ],
  },
  {
    id: 'P2-21-4',
    title: 'A 401 is terminal',
    spec: '§21.16',
    checks: [
      {
        id: 'AC-P2-21-4',
        status: 'automated',
        runner: 'cargo',
        owner: 'p2-21',
        test: 'sync_state::a_401_is_terminal_and_names_the_token',
        assert:
          'A 401 leaves the task blocked with no scheduled retry and records token_invalid, which is the failure the settled row actually names.',
      },
    ],
  },
  {
    id: 'P2-21-5',
    title: 'An unobserved budget is unknown, never zero and never the limit',
    spec: '§21.16',
    checks: [
      {
        id: 'AC-P2-21-5',
        status: 'automated',
        runner: 'cargo',
        owner: 'p2-21',
        test: 'sync_budget::an_unobserved_budget_is_unknown_and_a_task_holding_one_proceeds',
        assert:
          'With no sync_budget row the verdict is Unknown and the task proceeds; an observed row whose remaining header was absent reads as unknown rather than as zero or as the limit.',
      },
      {
        id: 'AC-P2-21-5-unsent',
        status: 'automated',
        runner: 'cargo',
        owner: 'p2-21',
        test: 'sync_budget::a_response_with_no_resource_header_writes_no_row_and_moves_no_clock',
        assert:
          'Rate headers arriving without x-ratelimit-resource cannot be keyed, so they write no row and move no observed_at.',
      },
    ],
  },
  {
    id: 'P2-21-6',
    title: 'The observation clock moves only for a call that observed something',
    spec: '§21.16',
    checks: [
      {
        id: 'AC-P2-21-6',
        status: 'automated',
        runner: 'cargo',
        owner: 'p2-21',
        test: 'sync_observation_clock::a_200_writes_a_304_confirms_and_every_refusal_dates_nothing',
        assert:
          'Against one fake in one test: a 200 writes the values and dates them, a 304 dates them without rewriting one, and a throttled, 401, 403, 404, rejected or skipped call dates nothing; the count of outcomes exercised is printed and zero fails.',
        scanning: true,
      },
    ],
  },
  {
    id: 'P2-21-7',
    title: 'The per-IP budget pool is one row',
    spec: '§21.16',
    checks: [
      {
        id: 'AC-P2-21-7',
        status: 'automated',
        runner: 'cargo',
        owner: 'p2-21',
        test: 'sync_schema::two_per_ip_budget_rows_for_one_resource_cannot_both_exist',
        assert:
          'Against a migrated database, two sync_budget rows with account_id IS NULL and one resource cannot both exist, while two accounts on that resource can.',
      },
    ],
  },
  {
    id: 'P2-21-8',
    title: "A park instant is derived from the response's own Date",
    spec: '§21.16',
    checks: [
      {
        id: 'AC-P2-21-8',
        status: 'automated',
        runner: 'cargo',
        owner: 'p2-21',
        test: 'sync_classify::the_park_instant_is_translated_onto_our_clock_from_the_responses_own_date',
        assert:
          'With a server clock offset by plus seven hours, by minus seven hours, and with the Date header absent, reset_at lands on this machine s clock rather than on the server s.',
      },
      {
        id: 'AC-P2-21-8-table',
        status: 'automated',
        runner: 'cargo',
        owner: 'p2-21',
        test: 'sync_classify::every_row_of_the_response_table_classifies_to_its_own_outcome',
        assert:
          'All ten rows of the response table, each asserted for what it produces, with the count of rows exercised printed so a row cannot be dropped and fail at zero unnoticed.',
        scanning: true,
      },
    ],
  },
  {
    id: 'P2-21-9',
    title: "The production provider reads through §21's decorator",
    spec: '§21.16',
    checks: [
      {
        id: 'AC-P2-21-9',
        status: 'automated',
        runner: 'cargo',
        owner: 'p2-21',
        test: 'sync_assembly::a_request_through_the_production_wiring_produces_an_observation',
        assert:
          'A provider call through assembly::sync::build_forge, which is the function the composition root calls, drains exactly one observation carrying the response s rate headers.',
      },
      {
        id: 'AC-P2-21-9-root',
        status: 'automated',
        runner: 'cargo',
        owner: 'p2-21',
        test: 'sync_assembly::the_composition_root_builds_its_forge_through_the_one_wiring_function',
        assert:
          'Prints the count of composition files scanned and fails at zero; the root calls build_forge once and constructs neither the provider nor the decorator by hand.',
        scanning: true,
      },
      {
        id: 'AC-P2-21-9-nofake',
        status: 'automated',
        runner: 'cargo',
        owner: 'p2-21',
        test: 'sync_assembly::no_test_double_appears_in_the_composition_root',
        assert:
          'Prints the count of composition files checked and fails at zero; no test double appears in the shipped composition root.',
        scanning: true,
      },
      {
        id: 'AC-P2-21-9-binary',
        status: 'automated',
        runner: 'cargo',
        owner: 'p2-21',
        test: 'sync_assembly::binary::the_real_binary_answers_sync_status_and_exits',
        assert:
          'The real binary completes the handshake, answers sync.status with empty arrays and two nulls, and exits cleanly after app.shutdown, which is the shutdown order stopping the pump.',
      },
    ],
  },
  {
    id: 'P2-21-10',
    title: 'Phase 1 is untouched',
    spec: '§21.16',
    checks: [
      {
        id: 'AC-P2-21-10',
        status: 'automated',
        runner: 'cargo',
        owner: 'p2-21',
        test: 'sync_phase1_untouched::a_full_sync_run_writes_no_job_row',
        assert:
          'After a run over all three task kinds the project_job_state count is unchanged, and the sync rows prove the run happened rather than passing on an empty queue.',
      },
      {
        id: 'AC-P2-21-10-kinds',
        status: 'automated',
        runner: 'cargo',
        owner: 'p2-21',
        test: 'sync_phase1_untouched::the_job_vocabulary_is_still_seven',
        assert: 'JobKind::ALL still holds seven members, so no j7 was added for symmetry.',
      },
      {
        id: 'AC-P2-21-10-partial',
        status: 'automated',
        runner: 'cargo',
        owner: 'p2-21',
        test: 'sync_phase1_untouched::job_outcome_partial_is_mentioned_by_one_production_file_once',
        assert:
          'Prints the count of files scanned and fails at zero; exactly one production file mentions JobOutcome::Partial, it is the state machine that must match on it, and it mentions it once.',
        scanning: true,
      },
    ],
  },
  {
    id: 'P2-21-11',
    title: 'Listing progress never guesses a denominator and never retreats',
    spec: '§21.16',
    checks: [
      {
        id: 'AC-P2-21-11',
        status: 'automated',
        runner: 'cargo',
        owner: 'p2-21',
        test: 'sync_runner::listing_progress_never_retreats_and_never_invents_a_denominator',
        assert:
          'Over a listing whose second page reports fewer entries, every listing_progress event carries a non-decreasing listed and a null total, and no payload contains a percent sign.',
      },
      {
        id: 'AC-P2-21-11-render',
        status: 'automated',
        runner: 'vitest',
        owner: 'p2-21',
        test: 'src/renderer/app/useSync.test.tsx',
        assert:
          'formatListingProgress renders "120 of 300" with a total and "120" without, never a percentage, and the hook renders what arrived rather than computing a figure of its own.',
      },
    ],
  },
  {
    id: 'P2-21-12',
    title: 'An interrupted task is re-queued, not surfaced as a failure',
    spec: '§21.16',
    checks: [
      {
        id: 'AC-P2-21-12',
        status: 'automated',
        runner: 'cargo',
        owner: 'p2-21',
        test: 'sync_runner::the_startup_sweep_requeues_a_running_row_and_moves_neither_counter',
        assert:
          'The start-up sweep moves a running row to queued with not_before zero and both counters untouched, and returns the count it moved.',
      },
      {
        id: 'AC-P2-21-12-quiet',
        status: 'automated',
        runner: 'cargo',
        owner: 'p2-21',
        test: 'sync_runner::an_interrupted_task_is_never_surfaced_as_a_failure',
        assert:
          'The pump sweeps the row and runs it, and no settled event reports the interrupted task as deferred or blocked.',
      },
    ],
  },
  {
    id: 'P2-21-13',
    title: 'A remote read is enqueued from exactly two visibility sites',
    spec: '§21.16',
    checks: [
      {
        id: 'AC-P2-21-13',
        status: 'automated',
        runner: 'cargo',
        owner: 'p2-21',
        test: 'sync_enqueue_sites::project_remote_is_enqueued_from_exactly_the_two_visibility_sites',
        assert:
          'Prints the count of files scanned and fails at zero; on_project_visible has exactly two call sites and they are detail/get.rs and projects/peek.rs.',
        scanning: true,
      },
      {
        id: 'AC-P2-21-13-nolocation',
        status: 'automated',
        runner: 'cargo',
        owner: 'p2-21',
        test: 'sync_enqueue_sites::a_project_with_no_location_still_enqueues_from_both_commands',
        assert:
          'A project with no location row still enqueues from projects.get and projects.peek, because the enqueue sits outside the location guard.',
      },
      {
        id: 'AC-P2-21-13-shelf',
        status: 'automated',
        runner: 'cargo',
        owner: 'p2-21',
        test: 'sync_enqueue_sites::the_two_commands_enqueue_once_each_and_the_shelf_enqueues_nothing',
        assert:
          'projects.get and projects.peek enqueue once each and projects.list enqueues nothing, so a shelf of a thousand rows queues no network task.',
      },
    ],
  },
  {
    id: 'P2-21-14',
    title: 'A scarce allowance is held for on-demand work',
    spec: '§21.16',
    checks: [
      {
        id: 'AC-P2-21-14',
        status: 'automated',
        runner: 'cargo',
        owner: 'p2-21',
        test: 'sync_budget::a_scarce_budget_yields_the_listing_and_spends_on_the_opened_page',
        assert:
          'With remaining at 150 and both kinds queued, exactly one request is issued and it is the on-demand one; the scheduled row is parked to the reset with both counters unchanged and reason reserve.',
      },
      {
        id: 'AC-P2-21-14-unknown',
        status: 'automated',
        runner: 'cargo',
        owner: 'p2-21',
        test: 'sync_budget::an_unknown_budget_lets_both_kinds_of_task_through',
        assert:
          'With no observation at all both kinds issue their request, because unknown is not below the reserve — it is unknown, and it spends.',
      },
    ],
  },
  {
    id: 'P2-21-15',
    title: 'An entry with no permission object is counted, never dropped',
    spec: '§21.16',
    checks: [
      {
        id: 'AC-P2-21-15',
        status: 'automated',
        runner: 'cargo',
        owner: 'p2-21',
        test: 'sync_listing::an_entry_with_no_permission_object_is_counted_and_admitted_nowhere',
        assert:
          'A listing entry with no permission object settles counted in skippedUnknownPermission, admitted nowhere, and dropped silently nowhere; the project set is exactly the admissible repository.',
      },
      {
        id: 'AC-P2-21-15-suppressed',
        status: 'automated',
        runner: 'cargo',
        owner: 'p2-21',
        test: 'sync_listing::a_suppression_names_the_project_that_blocked_it_once_per_entry',
        assert:
          'Two entries blocked by one project settle with suppressed equal to two and suppressedBy naming that project twice, and the list length equals the count in every case.',
      },
    ],
  },
];

/**
 * R56's floor. **A `live-observation` deferral carries no test id**, because a test standing in
 * for an observation is exactly the assertion the status exists to refuse. It is already named in
 * the harness's `LIVE_OBSERVATION_CHECKS`, which is what makes it registerable at all.
 */
const FLOOR = {
  id: 'AC-P2-21-3-floor',
  status: 'deferred',
  runner: 'cargo',
  owner: 'p2-21',
  assert:
    "That a forge's secondary rate limit is not usefully retried inside 60 seconds, which is what the floor in secondary_park_secs is set from.",
  deferral: 'live-observation',
  reason:
    '§21.4 states the 60 s floor as documentation knowledge, and no call has been made against a live forge by the decision round or by this lane. The implementation carries the floor; whether 60 s is the right number is the part that is unverified.',
  verification: { recordedAt: null, evidence: null },
};

const registry = JSON.parse(readFileSync(PATH, 'utf8'));
const existing = new Set(registry.criteria.map((c) => c.id));
if (existing.has('P2-21-1')) {
  console.error('acceptance/criteria.json already registers §21 — nothing to do');
  process.exit(1);
}

for (const entry of CRITERIA) {
  entry.group = 'functional';
  if (entry.id === 'P2-21-3') entry.checks.push(FLOOR);
}

// Appended after §20's block and before §22's, so the file reads in section order.
const at = registry.criteria.findIndex((c) => /^P2-22-/u.test(String(c.id)));
const before = at === -1 ? registry.criteria.length : at;
registry.criteria.splice(
  before,
  0,
  ...CRITERIA.map((entry) => ({
    id: entry.id,
    title: entry.title,
    group: entry.group,
    spec: entry.spec,
    checks: entry.checks,
  })),
);

writeFileSync(PATH, `${JSON.stringify(registry, null, 2)}\n`);
console.error(
  `registered ${String(CRITERIA.length)} §21 criteria and ` +
    `${String(CRITERIA.reduce((n, c) => n + c.checks.length, 0))} checks`,
);
