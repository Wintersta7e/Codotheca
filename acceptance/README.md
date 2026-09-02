# The acceptance registry

`criteria.json` is the machine-readable form of §16. One entry per criterion; one or more
**checks** per entry. A criterion's disposition is the weakest of its checks and is computed,
never stored.

| Status | Requires | Means |
|---|---|---|
| `automated` | `runner`, `test` | Runs in CI today and gates the build |
| `deferred` | `owner`, `test` | The named plan lands the named test |
| `manual` | `gate`, `reason` | A named review gate; the reason says why a machine cannot |
| `unmeasurable` | `reason`, no budget | No honest event pair, or no budget to gate on |
| `external` | `reason` | Criterion 29 only — it comes from beta users |

Adding a criterion: give it an id from §16, a `spec` citation, a group, and at least one check.
Adding a check: the id is `AC-<criterion>[-slug]` in lower kebab, unique across the file.

Any check may also carry a `reason`. It is required on `manual`, `unmeasurable` and `external`,
and optional elsewhere — a `deferred` check whose deferral needs an argument states it there, and
the disposition table prints every reason it finds. A deferral with no argument is just an owner
and a date that has not arrived; a deferral that had a reason and lost it reads the same way.

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
