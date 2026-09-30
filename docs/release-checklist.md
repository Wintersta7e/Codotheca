# Release checklist

The steps a person takes that no test covers. Work down it in order; each item says what to do
and, where it matters, why the obvious shortcut is wrong.

---

## 1. Before the tag

- [ ] **Write the release notes** at `docs/release-notes/<tag>.md` and commit them before tagging.
      The draft job refuses a tag without them: generated notes cannot carry the limits a person
      has to state (section 3).
- [ ] **Run the version check against the tag you are about to push:**
      `npm run check:version -- --tag <tag>`. It prints every version declaration the workspace
      holds and fails unless all of them equal the tag without its `v`. Quote its output.
- [ ] **Run the egress census:** `npm run check:egress`. It prints every network destination the
      code can reach and fails unless `README.md` and `SECURITY.md` name exactly those. Quote its
      output.
- [ ] **Re-run gitleaks and quote its own count:** `gitleaks detect --source .` — the commits it
      scanned and the leaks it found, as it printed them, not as a summary of them.

## 2. Building the artifacts

- [ ] **Build on the platform you are shipping for.** `scripts/build-dist.mjs` refuses to
      cross-compile and says so: an artifact is built on the platform it runs on, or it ships
      having never been executed.
- [ ] **Read `dist/logs/`, not the console.** Every step is redirected to a file and the file is
      read afterwards, because a piped command returns the pipe's exit status and a failed step
      reads as a successful one.
- [ ] `dist/logs/` holds build-machine paths. It is git-ignored, it is not shipped, and it
      should not be attached to a bug report.
- [ ] Confirm the tag is on the exact commit the artifacts were built from.

## 3. What the release notes must say

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
      `CODOTHECA_DATA_DIR` overrides it. **Write down what the release run's `launch` job
      printed**, not what the source says: it launches the installed build and the portable one
      with the default data directory and prints the directory each wrote and whether they match.
- [ ] **State that there is no updater.** No update channel, no update server, no background
      network traffic. A fix arrives when the user downloads the next release. `SECURITY.md`
      says the same thing about supported versions; keep the two consistent.
- [ ] **Say how to verify a download:** the `SHA256SUMS` file beside the artifacts, and
      `gh attestation verify <file> --repo <this repository>`.
- [ ] Link `SECURITY.md` for how to report a vulnerability privately.

## 4. The tag's release run, and the draft it made

- [ ] **Read every job's log, head first.** A green run is not the record: the run id, the
      tagged commit and the lines below are.
- [ ] The `version` job printed every declaration and `tag: <tag> — matches`. Quote it.
- [ ] The Linux build job's floor step printed each packed binary with its `GLIBC` need and what
      sits above the baseline. Quote the figures; a figure that differs from the notes is
      recorded as measured and the notes are corrected, never the other way round.
- [ ] The `launch` job printed each artifact it ran, with its SHA-256, and how it quit. Quote
      each line.
- [ ] **Download the draft — never rebuild it — and verify it:**
      `gh release download <tag> --dir <dir>`, then
      `node scripts/verify-release.mjs --dir <dir> --repo <owner>/<name>`. It prints one line per
      file and `files verified: <n>`, and exits non-zero on a missing `SHA256SUMS`, a file whose
      bytes do not match it, a file it does not list, or a failed attestation. A rebuild is not
      byte-identical, so a sum taken from one proves nothing about what a user downloads.
- [ ] **Launch what no runner launches, from the downloaded bytes,** and record the commit and
      what you saw: the `.rpm` installed and launched on an rpm-based system, and, while the
      floor step runs in report mode, the AppImage on a system at the glibc floor.

## 5. Repository settings to keep on

- [ ] **Private vulnerability reporting** — Settings → Code security → Private vulnerability
      reporting. `SECURITY.md` sends people to the Security tab, and if that button is off it
      sends them nowhere.
- [ ] **Delete a branch as soon as it is merged.**

## 6. After publishing

- [ ] Record the release's dependency posture with the release. A vulnerability database is a
      moving target, so "no known advisories" is a **dated** claim, not a permanent one.
