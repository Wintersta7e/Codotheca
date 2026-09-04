# The acceptance registry

`criteria.json` is the machine-readable form of the acceptance bar. One entry per criterion; one
or more **checks** per entry. A criterion's disposition is the weakest of its checks and is
computed, never stored.

| Status | Requires | Means |
|---|---|---|
| `automated` | `runner`, `test` | Runs in CI today and gates the build |
| `deferred` | `owner`, `test` | The named plan lands the named test |
| `manual` | `gate`, `reason` | A named review gate; the reason says why a machine cannot |
| `unmeasurable` | `reason`, no budget | No honest event pair, or no budget to gate on |
| `external` | `reason` | Criterion 29 only — it comes from beta users |

Adding a criterion: give it an id, a `spec` citation, a group, and at least one check.
Adding a check: the id is `AC-<criterion>[-slug]` in lower kebab, unique across the file.

Any check may also carry a `reason`. It is required on `manual`, `unmeasurable` and `external`,
and optional elsewhere — a `deferred` check whose deferral needs an argument states it there, and
the disposition table prints every reason it finds. A deferral with no argument is just an owner
and a date that has not arrived; a deferral that had a reason and lost it reads the same way.

## Two phases, one register

A phase-1 criterion is a §16 number — `14`, `45b`. A phase-2 criterion is `P2-<section>-<n>`
over sections 20 to 25, and its checks are `AC-P2-<section>-<n>[-slug]` where a slug segment
begins with a letter (`AC-P2-25-10-ddl`). **The id is the ownership map**: the phase is computed
from it and the owning section is read out of it, so neither can drift from a field beside it.
A `P2-26-*` id is refused by the id form — §26 is this contract and owns no criterion.

A phase-2 criterion cites its own section (`P2-25-10` cites `§25.`), carries no `perf` runner,
no `measurement` and no `budget`, and is never `external`. Once a section holds **one** entry it
must hold **all** of them, contiguous, with no gaps: a section holding none is not yet registered
and is silent.

Three optional markings, declared by the author and never inferred:

| Field | Shape | Means |
|---|---|---|
| `scanning` | `true` | The check walks a tree, greps a bundle or enumerates a schema. Its `assert` must say it **prints a count** and **fails at zero** — a passing run that scanned nothing is a failing gate |
| `mirror` | `{ "other": "<path>" }` | A cross-language constant, asserted from both sides. The path is the other language's file the test reads, and it must exist |
| `source` | `"§N.N"`, `"probe:<name>"` or `"schema"` | Where a figure in the `assert` comes from. A restatement of the number is not its source |

## Two shapes of deferral

`deferred` gains a `deferral` discriminator. It defaults to `plan`, so every phase-1 deferral
means exactly what it did.

| `deferral` | Requires | Refuses | Means |
|---|---|---|---|
| `plan` (default) | `owner`, `test` | — | The named plan lands the named test |
| `live-observation` | `owner`, `reason`, `verification: { recordedAt, evidence }` | `test` | Documentation knowledge until it is checked against a live response |

`live-observation` **refuses a test id**, because a test standing in for an observation is a
plausible assertion passing for a measured one — the thing the register exists to refuse.
`recordedAt` stays `null` until the observation happens; evidence without a date, or a date
without a record, is refused both ways. It is the two ids named in `LIVE_OBSERVATION_CHECKS` and
nothing else: a third needs a ruling, not a field.

## The test id is the join key

`test` is matched against the runner's own output, exactly:

- **cargo** — `<test binary>::<function>` for an integration test under `core/tests/`, taken from
  cargo's `Running tests/<binary>.rs` line and libtest's `test <function> ... ok`. A unit test
  inside `core/src` prints its own module path and is used unchanged.
- **vitest** — the reporter's `fullName`, which is every enclosing `describe` title and the test
  title joined by a single space. A test with no `describe` around it joins on its title alone.
- **e2e** — the Playwright spec title.
- **script** — the check id the static gate emits, e.g. `check-forbidden:c44-forget-token`.

There is no name table, on purpose: a table mapping criterion to test name is a second copy of
the truth and rots on the first rename. A rename makes the gate report the check as **not run**,
which is the intended behaviour.

## Dispositions today

Most criteria carry a mixture, and the honest statement is the generated one rather than a
sentence written here that can drift from it: run `npm run acceptance -- --report` for the
current roll-up, and read `DISPOSITIONS.md` for the committed one. `DISPOSITIONS.md` is a pure
function of `criteria.json` and is diff-gated, so editing the registry without regenerating it
fails the build.

`baseline.json` holds failing tests that are **known** red. It compares by identity, never by
count: a failure that is not listed fails the build, a listed test that now passes fails the
build as a stale entry, and a listed test that did not run at all fails the build as rot.
