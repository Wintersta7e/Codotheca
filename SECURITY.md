# Security

## Reporting a vulnerability

Report privately through this repository's **Security** tab — *Report a vulnerability* — which
opens a private advisory visible only to the maintainers. Please do not open a public issue for
anything exploitable.

No email address is published here on purpose. The only address this project has is the
no-reply address its commits are authored under, which cannot receive mail, and a real one
would be a private address in a public file.

What helps in a report: what you did, what happened, the version from the release page or the
commit you built, and your operating system. If a diagnostics bundle is relevant, note that
`EXPORT EVERYTHING` writes one that is **anonymised by default** — it carries basenames and a
per-volume pseudonym rather than your directory tree, and it names the rolling log rather than
embedding it, so you attach that separately and deliberately.

Expect an acknowledgement within a week. This is a small project with one maintainer; there is
no paid triage rota and no bounty.

## Supported versions

**Phase 1 is a public beta.** Only the most recent release is supported. There is no update
channel, no auto-updater and no update server: a release is a page you download from by hand,
and a fix reaches you when you download the next one. Nothing older than the current release
gets a backport.

| Version | Supported |
| --- | --- |
| The most recent release | Yes |
| Anything earlier | No |

## What this application does and does not do

These are the properties a report can be measured against. Each is asserted by a test or a
gate in this repository, and several run over the **built** shell rather than the source,
because what ships is what matters: `npm run lint:updater`, `npm run check:bundle`,
`npm run check:forbidden`, `npm run check:callsites`, `app/src/shared/csp.test.ts`,
`core/tests/git_readonly.rs`, and the end-to-end spec that opens a real window and checks
the renderer can see no Node.

- **No account, no telemetry, no analytics.** The application makes no network request of its
  own. The Content-Security-Policy starts from `default-src 'none'` and names no remote origin.
- **The renderer is sandboxed.** `sandbox` and `contextIsolation` are on, `nodeIntegration` is
  off, navigation away from the app is blocked, new windows are denied, and every permission
  request is refused. The preload exposes exactly one bridge key and requires nothing but
  `electron`.
- **The renderer never originates a filesystem path or an executable.** A path enters only
  through a native dialog the shell owns.
- **No update channel is shipped.** `electron-updater` is not a dependency and a gate asserts
  its absence from the built shell.
- **Every git invocation is read-only.** Phase 1 has no delete, uninstall, clean, push or
  checkout.
- **One writer.** The core owns the only database connection; the renderer cannot open it.

## Releases are unsigned

There is no code-signing certificate. The Windows installer and the portable executable are
**unsigned**, and SmartScreen will warn about them. Verify a download against the SHA-256 sums
published beside it on the release page rather than against a signature that does not exist.
The packager reports `signing=none` rather than implying otherwise.

## Scope

In scope: anything that lets a repository on disk, a file it contains, or a crafted git object
run code, escape the renderer sandbox, or read a file outside the folders you added.

Out of scope: `git` itself, Electron and Chromium (report those upstream), and the consequences
of pointing the application at a repository you do not trust — it reads what you tell it to
read.

A scanned repository is nonetheless treated as untrusted data. Every `git` invocation is built
as an argv vector with no shell anywhere, and each one carries
`-c core.hooksPath=<an empty directory the app owns>`, `-c core.fsmonitor=false`,
`-c protocol.ext.allow=never`, `-c diff.external=`, `-c core.askPass=` and
`-c credential.helper=`, with `GIT_CONFIG_NOSYSTEM=1`, `GIT_TERMINAL_PROMPT=0` and thirteen
other `GIT_*` variables removed from the child's environment. `safe.directory` is added for one
exact path and only after you have marked that location trusted. Those are the settings a
repository's own configuration would otherwise use to run a program, so a report that gets past
them is exactly the kind this file is for.
