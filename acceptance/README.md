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

## Four phases, one register

A phase-1 criterion is a §16 number — `14`, `45b`. A phase-2 criterion is `P2-<section>-<n>`
over sections 20 to 25, and its checks are `AC-P2-<section>-<n>[-slug]` where a slug segment
begins with a letter (`AC-P2-25-10-ddl`). **The id is the ownership map**: the phase is computed
from it and the owning section is read out of it, so neither can drift from a field beside it.
A `P2-26-*` id is refused by the id form — §26 is this contract and owns no criterion.

**There is no phase field.** `phaseOf` reads the phase out of the id's `P<n>-` prefix, and an id
naming a phase the register does not hold says exactly that rather than being graded as a
malformed phase-1 criterion.

A phase-2 criterion cites its own section (`P2-25-10` cites `§25.`), carries no `perf` runner,
no `measurement` and no `budget`, and is never `external`. Once a section holds **one** entry it
must hold **all** of them, contiguous, with no gaps: a section holding none is not yet registered
and is silent.

A phase-3 criterion is `P3-<section>-<n>` over sections 28 to 35, and its checks are
`AC-P3-<section>-<n>[-slug]` under the same slug rule. A `P3-36-*` id is refused by the id form —
§36 is this contract and owns no criterion. The same rules bind as for phase 2: it cites its own
section, states no performance figure, is never `external`, and its section is complete or silent.

**Two phase-3 ids carry a letter suffix: `P3-28-18a` and `P3-30-11a`.** Unlike phase 1's `45a`,
`45b`, `45c`, which have no bare form, each of these is an **additional** id beside a bare twin
that also exists — `P3-28-18` and `P3-30-11` are criteria of their own, and neither stands in for
the other. Both are present or neither is. They are held apart from the contiguity range for a
measured reason: `Number.parseInt('18a', 10)` is `18`, so a lettered id folded into that list
makes its bare twin report a duplicate that exists in no register.

A phase-4 criterion is `P4-<section>-<n>` over sections 38 to 48, and its checks are
`AC-P4-<section>-<n>[-slug]` under the same slug rule. `P4-37-*` and `P4-49-*` are refused by the
id form — §37 is the phase's scope and §49 this contract, and neither owns a criterion. **Phase 4
has no letter suffix**; one would need a ruling and a widening. The phase-2 rules bind — its own
section, no performance figure, never `external`, complete or silent — with one difference:
**a phase-4 check may be `deferred` to the plan that lands it.** Phase 4 registers every one of
its criteria before its lanes merge, so each lane's first test finds its check waiting under the name
its plan gives it; the release mode refuses every such deferral at the tag. A lane whose test
lands under a different name corrects the check's `test` in the same commit.

A phase-4 plan id owns a check the way `p2-20` does: `p4-38`, `p4-46b`, and the three Lane-0
plans `p4-L0a`, `p4-L0b`, `p4-L0c`, whose capital `L` a prefix alone does not admit. **Every
phase-4 check names its owner, a `manual` one included** — the lane whose merge makes that check
passable.

Three fields phase 4 adds, declared by the author and never inferred:

| Field | Shape | Means |
|---|---|---|
| `firstTag` | `true`, or absent | The check gates the first tag, `v0.9.0`. A phase-4 check carries it exactly when a Lane-0 plan lands it, and on no other phase's check. `--tag ft` reads it |
| `platforms` | `["linux"]`, `["windows"]` or both; absent means Linux | Which platform's results grade the check. No capture holds a Windows-native result — CI grades acceptance on Ubuntu and the gate copies the WSL cargo run — so **a check naming `windows` is `manual`, gate `WINDOWS-NATIVE`, `platforms: ["windows"]`**, recorded from a Windows-native run, beside the automated Linux check |
| `record` | `{ "recordedAt": "YYYY-MM-DD", "evidence": "…", "commit": "<full hex>" }` or `null` | A `manual` check's record: all three fields, or `null` until the gate runs. Only a `manual` check carries it, and every phase-4 `manual` check carries the key. An earlier phase's manual gate gains it when it is recorded |

**A record counts only while the tree has moved in the register alone since it was made.**
Committing a record makes a new commit, so the commit a record names can never be the one being
graded; the run lists `git diff --name-only <record.commit> <graded commit>`, and the record counts
when that names only `acceptance/criteria.json` and `acceptance/DISPOSITIONS.md`. A record whose
commit a shallow clone does not hold cannot be shown to count, and does not. A counting record
joins as **`recorded`**; a `manual` result is reported and never gated on an ordinary run. A live
observation's `verification` carries the `commit` it was observed against once it is dated.

The gates a phase-4 `manual` check may name are closed: `WINDOWS-NATIVE` (a clause graded on
Windows), `UNWIRED-AUDIT` (every producer the phase added has a production caller, audited against
the tagged commit), `PACKAGED-NOTIFICATION` (one per packaged build), `TRASH-QUOTA-PROBE`,
`EGRESS-CAPTURE`, and the release job's own steps, `RELEASE-VERSION-CHECK`, `RELEASE-PIPELINE`,
`RELEASE-LAUNCH`, `RELEASE-LAUNCH-HANDS` and `RELEASE-GLIBC-FLOOR`. No phase-4 check is a
`live-observation`.

Three optional markings, declared by the author and never inferred:

| Field | Shape | Means |
|---|---|---|
| `scanning` | `true` | The check walks a tree, greps a bundle or enumerates a schema. Its `assert` must say it **prints a count** and **fails at zero** — a passing run that scanned nothing is a failing gate |
| `mirror` | `{ "other": "<path>" }` | A cross-language constant, asserted from both sides. The path is the other language's file the test reads, and it must exist |
| `source` | `"§N.N"`, `"probe:<name>"` or `"schema"` | Where a figure in the `assert` comes from. A restatement of the number is not its source |
| `shares` | `"<the other check's id>"` | This check joins to a test another check already names. Exactly one check per test id may omit it — that one owns the key — and every other must name a check in the same group |

`shares` exists because two checks legitimately join to one run: a static rule narrowed in place
is claimed by the phase-1 criterion and the phase-2 one that narrowed it, and a criterion split
across two owners takes a check each. Both are deliberate and both look exactly like a
copy-pasted join key, which makes two criteria read as covered by one test. Declaring the share
is what tells them apart, and an undeclared duplicate fails.

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
without a record, is refused both ways. It is the ids named in `LIVE_OBSERVATION_CHECKS` and
nothing else: another needs a ruling, not a field.

**A `live-observation` deferral is discharged by setting `recordedAt` and `evidence` together,
and by nothing else.** Not by writing a test — a test here would be the assertion standing in for
the observation. Not by promoting the check to `automated`. Record what the response said, with
enough of it to be a record rather than a claim, and the check becomes `automated` in the same
change as the test that reads it.

| Check | What would discharge it |
|---|---|
| `AC-P2-20-13` | `X-OAuth-Scopes` read off a real token |
| `AC-P2-21-3-floor` | A real secondary-limit `Retry-After` from a forge |
| `AC-P3-32-3-header` | Whether a real advisories response carries a rate-limit header **at all** — with none, nothing is mirrored and the sweep runs with no brake |
| `AC-P3-32-3-resource` | What that header says the resource is, against a process-wide default that is a guess |
| `AC-P3-32-16-caps` | The 16 MB and 32-lockfile caps, which have exactly one datum behind them |

Each phase-3 one is a **second** check on a criterion whose first check is automated, so nothing
is registered as observed and nothing loses its fixture test — only the values are unobserved.

**A `plan` deferral is a promise to a plan that has not merged.** Once every plan of a phase has
merged, a deferral to one is a deferral to nobody, and `validatePhase2Complete` says so — that
is R46's *41 checks with an owner and no implementing task*, turned into a gate. The same audit
refuses a static rule still carrying the `pendingRegistryEntry` escape a registered check now
discharges.

## The test id is the join key

`test` is matched against the runner's own output, exactly:

- **cargo** — `<test binary>::<function>` for an integration test under `core/tests/`, taken from
  cargo's `Running tests/<binary>.rs` line and libtest's `test <function> ... ok`. A unit test
  inside `core/src` prints its own module path and is used unchanged.
- **vitest** — the reporter's `fullName`, which is every enclosing `describe` title and the test
  title joined by a single space. A test with no `describe` around it joins on its title alone.
- **e2e** — the Playwright spec title.
- **node** — `<file>::<full name>` for a `node:test` test in the harness or the protocol suite:
  the file relative to the repository, then every enclosing suite and the test joined by a single
  space, e.g. `scripts/check-motion-clamp.test.mjs::ac_p3_34_15 no acceptance assert …`.
- **script** — the check id the static gate emits, e.g. `check-forbidden:c44-forget-token`.

There is no name table, on purpose: a table mapping criterion to test name is a second copy of
the truth and rots on the first rename. A rename makes the gate report the check as **not run**,
which is the intended behaviour.

## Who writes the captures

`npm run acceptance` runs no suite: it grades whatever `acceptance/results/` holds. So **every
capture file the CLI parses has a gate step that writes it**, or the register is graded against a
tree nobody ran. The writers are named here and never counted — the count was stated four ways
and was wrong each time:

| Capture | Written by | Reached from the gate through |
|---|---|---|
| `cargo.txt` | `cargo test`, copied by the gate | the gate's own copy |
| `vitest.json` | vitest's JSON reporter | the gate's `--outputFile.json` |
| `e2e.json` | Playwright, copied by the gate | the gate's own copy |
| `node-harness.json` | `node:test`, through `scripts/acceptance/node-reporter.mjs` | the gate's `NODE_OPTIONS` on its harness step |
| `node-protocol.json` | `node:test`, through `scripts/acceptance/node-reporter.mjs` | the gate's `NODE_OPTIONS` on its protocol step |
| `script-bundle.json` | `scripts/check-bundle.mjs` | `npm run check:bundle` |
| `script-callsites.json` | `scripts/check-call-sites.mjs` | `npm run check:callsites` |
| `script-forbidden.json` | `scripts/check-forbidden.mjs` | `npm run check:forbidden` |
| `script-motionclamp.json` | `scripts/check-motion-clamp.mjs` | `npm run lint` → `lint:shell` → `lint:clamp` |
| `script-problems.json` | `scripts/check-problem-kinds.mjs` | `npm run check:problems` |

`scripts/acceptance/captures.mjs` holds the declared list. Its test derives the writers from the
tree and fails when the two disagree, and `captures.mjs --gate <gate script>` runs **in** the gate
and fails when a writer's npm script is not **reachable** from a step — through `package.json`'s
script graph, so a writer chained into `lint:shell` counts. A new writer is refused by name until
it is declared and given a step.

**The two `node:test` suites write a capture each**, one per gate step so neither overwrites the
other. The reporter runs on `node:test`'s own reporter API rather than the built-in `junit`
reporter, which records a test's name and not its file. The flags travel in `NODE_OPTIONS`
because a flag placed after the file patterns is **silently ignored**, and a `node --test`
started inside a test file must drop `NODE_TEST_CONTEXT` or it reports to its parent instead.

## Grading a tag

`npm run acceptance -- --tag ft` grades the first tag and `-- --tag 1.0` grades 1.0. Either
**refuses a dirty tree** — `git status --porcelain` names anything, and it exits 2 before grading —
then runs the ordinary gate, then applies the release rules on top of it:

- **`ft`**: every `firstTag` check is `automated` and passed, or `manual` and `recorded`.
- **`1.0`**: prints the set allowed not to run, which it **derives** rather than lists — the `perf`
  runner (by runner, not by group, so a behaviour check in a performance criterion is still
  graded), status `external`, status `unmeasurable`, and `RELEASE_NOT_RUN` — and fails on every
  other check that did not pass or is not recorded. **Any `plan` deferral outside that set fails,
  in any phase**: at the last tag every plan has merged, and a deferral to a merged plan is a
  deferral to nobody.
- **Both**: every *"`<n>` checks … `<m>` criteria"* figure `README.md` quotes must equal the
  register's own line, which the run prints for the comparison; a README quoting none fails.

`RELEASE_NOT_RUN` (`scripts/acceptance/release.mjs`) is closed like `LIVE_OBSERVATION_CHECKS`, and
every entry carries the ruling that exempts it. Another needs a ruling, not a field.

**The order that can pass.** A tag's own release job writes half the evidence — the version, the
pipeline, the launches and the glibc floor are recorded from its run — and that cannot exist before
the tag. So: tag, and keep the draft unpublished; commit the records against the **tagged** commit;
run `--tag` on that register-only descendant, where the records still count because only register
files moved; then publish.

## Dispositions today

Most criteria carry a mixture, and the honest statement is the generated one rather than a
sentence written here that can drift from it: run `npm run acceptance -- --report` for the
current roll-up, and read `DISPOSITIONS.md` for the committed one. `DISPOSITIONS.md` is a pure
function of `criteria.json` and is diff-gated, so editing the registry without regenerating it
fails the build.

The roll-up is per phase, four of them, and each is printed rather than restated here. A passing
run's first line is `renderRegistryLine`'s `<n> criteria / <m> checks validated — phase 1 <c>/<k>,
phase 2 <c>/<k>, phase 3 <c>/<k>, phase 4 <c>/<k>`, and `DISPOSITIONS.md` renders one table per
phase with its own disposition counts. Every phase-3 section, §28 to §35, is registered in full,
which `validatePhase3Complete` asserts against `PHASE3_SECTIONS`; every phase-4 section, §38 to
§48, likewise against `PHASE4_SECTIONS`, most of it deferred to the lanes that land it. A
criterion's disposition is its
weakest check's, so an audit of a register entry registered beside a deferred behaviour check —
`AC-42-register-audit` — leaves the criterion reading `deferred`.

`baseline.json` holds failing tests that are **known** red. It compares by identity, never by
count: a failure that is not listed fails the build, a listed test that now passes fails the
build as a stale entry, and a listed test that did not run at all fails the build as rot.
