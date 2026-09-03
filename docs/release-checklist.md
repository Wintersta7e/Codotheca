# Release checklist

The steps a person takes that no test covers. Work down it in order; each item says what to do
and, where it matters, why the obvious shortcut is wrong.

---

## 1. Building the artifacts

- [ ] **Build on the platform you are shipping for.** `scripts/build-dist.mjs` refuses to
      cross-compile and says so: an artifact is built on the platform it runs on, or it ships
      having never been executed.
- [ ] **Read `dist/logs/`, not the console.** Every step is redirected to a file and the file is
      read afterwards, because a piped command returns the pipe's exit status and a failed step
      reads as a successful one.
- [ ] `dist/logs/` holds build-machine paths. It is git-ignored, it is not shipped, and it
      should not be attached to a bug report.
- [ ] Confirm the tag is on the exact commit the artifacts were built from.

## 2. What the release notes must say

- [ ] **The artifacts are unsigned.** There is no code-signing certificate; the packager reports
      `signing=none` rather than implying otherwise. Say so in the notes rather than letting a
      warning be the first a user hears of it.
- [ ] **Say what SmartScreen will show.** On Windows an unsigned installer raises
      *"Windows protected your PC"*, with a **More info → Run anyway** path. A user who has not
      been told will read it as malware; a user who has been told is not being trained to click
      through warnings in general.
- [ ] **State where the portable executable keeps its data.** The `portable` target unpacks to a
      temporary directory on each run, and the application's data directory is Electron's
      `userData` — the same per-user location the installer build uses — unless
      `CODOTHECA_DATA_DIR` overrides it. **Verify this on a real Windows machine before writing
      it down**: it is `resolveDataDir`'s behaviour read from the source, not a measurement, and
      the portable build has not been run.
- [ ] **State that there is no updater.** No update channel, no update server, no background
      network traffic. A fix arrives when the user downloads the next release. `SECURITY.md`
      says the same thing about supported versions; keep the two consistent.
- [ ] Link `SECURITY.md` for how to report a vulnerability privately.

## 3. Checksums

- [ ] **Publish SHA-256 sums beside the artifacts**, and take them from **the files you actually
      uploaded** — not from the build log, and not from a local copy you rebuilt afterwards. A
      rebuild is not byte-identical, and a sum that does not match what a user downloads is worse
      than no sum, because it reads as tampering.
      The order that gets this right: upload, download your own upload, hash that, publish those
      sums.
- [ ] `scripts/build-dist.mjs` prints a sha256 per artifact at the end of a build. Use it to
      check the upload round-tripped, not as the published value.

## 4. Before the repository is public

- [ ] **Enable private vulnerability reporting** — Settings → Code security → Private
      vulnerability reporting. `SECURITY.md` sends people to the Security tab, and if that
      button is off it sends them nowhere.
- [ ] **Delete every merged branch.** `git branch --merged main`. A stale branch is history you
      are about to publish.
- [ ] Confirm the repository has never been pushed with anything in it you would not publish.
      Before the first push a mistake is removable; after it, a leaked credential is rotated
      rather than removed.

## 5. After publishing

- [ ] Record the release's dependency posture with the release. A vulnerability database is a
      moving target, so "no known advisories" is a **dated** claim, not a permanent one.
