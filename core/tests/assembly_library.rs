#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! The binary builds a library.
//!
//! `assembly_startup.rs` proved the core *answers*; this proves it *works*. The real binary is
//! spawned against a temporary index and a generated corpus, told to add a root and scan, and
//! then asked for its library. Four things are asserted, and each one fails for its own reason:
//!
//! 1. `projects.list` returns one row per repository — the hand-off wrote `project` and
//!    `location` rows at all. Read through **two** queries, because §8.0b keeps Reference off
//!    the default shelf and a generated corpus is entirely Reference; see `library`.
//! 2. At least one row carries a value **only a job produces**. `refstateObservedAt` is NULL
//!    until J1 has run and `upsert_location` never touches it, so a non-null value cannot come
//!    from the walk. Present is not enough: it must be *computed*.
//! 3. A second scan over an unchanged tree adds no duplicate row.
//! 4. The process exits when told to, within a bounded wait — a worker pool holding a publisher
//!    clone hangs the transport join, and nothing else would say so.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use codotheca_core::corpus::{ensure, CorpusOptions};
use codotheca_core::proto::frame::{read_frame, write_frame};

/// The fixtures this test scans. All four land on volume A, all four have distinct identities,
/// and none of them is one of the slow ones — the point here is assembly, not scale.
const WANTED: [&str; 4] = ["upstream", "other-upstream", "zero-commit", "multi-root"];

/// A walk over four small repositories plus the jobs they queue. Generous, because a red gate
/// from a busy machine is worse than a slow one.
const SCAN_DEADLINE: Duration = Duration::from_secs(240);
/// Long enough for J1 to be scheduled, run and persisted on a loaded machine.
const COMPUTED_DEADLINE: Duration = Duration::from_secs(240);
/// `app.shutdown` to a dead process.
const EXIT_DEADLINE: Duration = Duration::from_secs(60);

struct Core {
    child: std::process::Child,
    next_id: u64,
}

impl Core {
    fn spawn(dir: &Path) -> Core {
        let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_codotheca-core"))
            .arg(format!("--data-dir={}", dir.display()))
            .arg("--epoch=11")
            .arg(format!("--parent-pid={}", std::process::id()))
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("core spawns");

        let hello = read_json(&mut child);
        assert_eq!(hello["t"], "hello", "{hello}");
        let mut core = Core { child, next_id: 1 };
        core.request("app.hello_ack", &serde_json::json!({}));
        core
    }

    /// Send one request and read frames until **its** response arrives.
    ///
    /// Events share the stream, so matching on the id is the whole of the correlation: taking
    /// the next frame would read a `scan/progress` and call it the answer.
    fn request(&mut self, command: &str, args: &serde_json::Value) -> serde_json::Value {
        let id = self.next_id;
        self.next_id += 1;
        let frame = serde_json::json!({"t": "request", "id": id, "command": command, "args": args});
        let body = serde_json::to_vec(&frame).expect("frame");
        {
            let stdin = self.child.stdin.as_mut().expect("stdin");
            write_frame(stdin, &body).expect("write");
            stdin.flush().expect("flush");
        }
        loop {
            let reply = read_json(&mut self.child);
            if reply["id"] == serde_json::json!(id) {
                assert_eq!(
                    reply["t"], "response",
                    "{command} was refused rather than answered: {reply}"
                );
                return reply["ok"].clone();
            }
        }
    }

    /// Fire and forget: `app.shutdown` may or may not be answered before the process goes.
    fn send(&mut self, command: &str, args: &serde_json::Value) {
        let id = self.next_id;
        self.next_id += 1;
        let frame = serde_json::json!({"t": "request", "id": id, "command": command, "args": args});
        let body = serde_json::to_vec(&frame).expect("frame");
        let stdin = self.child.stdin.as_mut().expect("stdin");
        write_frame(stdin, &body).expect("write");
        stdin.flush().expect("flush");
    }
}

fn read_json(child: &mut std::process::Child) -> serde_json::Value {
    let stdout = child.stdout.as_mut().expect("stdout");
    let mut buf = Vec::new();
    read_frame(stdout, &mut buf).expect("the core is still speaking");
    serde_json::from_slice(&buf).expect("json")
}

/// Build the four fixtures in a directory of this test's own.
///
/// Not `shared_corpus`: that one is the whole 25-fixture tree, and this test walks its scan root
/// for real. A private root is what keeps the expected project set exactly the four below.
fn corpus_volume(root: &Path) -> PathBuf {
    let mut options = CorpusOptions::new(root);
    options.only = Some(WANTED.iter().map(|n| (*n).to_owned()).collect());
    let manifest = ensure(&options).expect("corpus builds");
    let volume = manifest
        .volume("vol-a")
        .expect("the corpus lays these fixtures on volume A");
    for name in WANTED {
        let fixture = manifest.require(name).expect("fixture built");
        assert!(
            fixture.materialised,
            "{name} was skipped ({:?}); this test would then assert a smaller library than it \
             claims to",
            fixture.skip_reason
        );
    }
    volume.path.clone()
}

fn rows_of(page: &serde_json::Value) -> Vec<serde_json::Value> {
    page["rows"].as_array().expect("rows is an array").clone()
}

/// Every row in the library, which is **two** queries and not one.
///
/// §8.0b keeps `is:reference` off the default shelf, and every repository in a generated corpus
/// is Reference: its commits carry a fixture identity, not this machine's, so J1.5 answers
/// `authored_by_user = 0`. Asking only the plain query would make this test read the shelf's
/// *policy* as the library's *contents* — and would go green or red depending on how far the
/// scheduler had got.
fn library(core: &mut Core) -> Vec<serde_json::Value> {
    let mut rows = rows_of(&core.request("projects.list", &serde_json::json!({})));
    rows.extend(rows_of(&core.request(
        "projects.list",
        &serde_json::json!({ "query": "is:reference" }),
    )));
    rows.sort_by(|a, b| a["id"].as_i64().cmp(&b["id"].as_i64()));
    rows.dedup_by_key(|row| row["id"].as_i64());
    rows
}

fn names_of(rows: &[serde_json::Value]) -> Vec<String> {
    let mut names: Vec<String> = rows
        .iter()
        .map(|row| row["name"].as_str().expect("a name").to_owned())
        .collect();
    names.sort();
    names
}

/// Run one scan to completion. `scan.status` is polled rather than `scan.finished` subscribed
/// to, so the wait cannot outlive the deadline waiting for an event that will not come.
fn scan_to_completion(core: &mut Core) -> serde_json::Value {
    core.request("scan.start", &serde_json::json!({"full": true}));
    let until = Instant::now() + SCAN_DEADLINE;
    loop {
        let status = core.request("scan.status", &serde_json::json!({}));
        if status["running"] == serde_json::json!(false) && !status["endedAt"].is_null() {
            return status;
        }
        assert!(
            Instant::now() < until,
            "the scan did not finish within {SCAN_DEADLINE:?}: {status}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// The first row carrying a value the walk cannot have written, or `None` on timeout.
///
/// `refstateObservedAt` is the discriminator on purpose: `upsert_location` writes none of the
/// observation columns on insert or on update, precisely so that "not computed" survives a
/// rescan. A non-null value here therefore means a job ran.
fn wait_for_a_computed_row(core: &mut Core) -> Option<serde_json::Value> {
    let until = Instant::now() + COMPUTED_DEADLINE;
    loop {
        let hit = library(core)
            .into_iter()
            .find(|row| !row["refstateObservedAt"].is_null());
        if hit.is_some() {
            return hit;
        }
        if Instant::now() >= until {
            return None;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

#[test]
fn the_binary_scans_a_corpus_and_answers_with_a_library() {
    let corpus_dir = tempfile::tempdir().expect("tmp");
    let volume = corpus_volume(corpus_dir.path());
    let data_dir = tempfile::tempdir().expect("tmp");
    let mut core = Core::spawn(data_dir.path());

    let added = core.request(
        "roots.add",
        &serde_json::json!({
            "pathBytes": {
                "b64": base64_of(volume.to_str().expect("the corpus root is UTF-8").as_bytes())
            },
            // The root is this test's own temporary tree, not a folder a user picked, so the
            // directory estimate would only be a slow way to learn what the walk is about to.
            "confirmLarge": true,
        }),
    );
    assert!(
        added["refusedBecause"].is_null() && !added["root"].is_null(),
        "the scan root was refused, so nothing below is about scanning: {added}"
    );

    // The walk itself, before anything about rows. A run that found nothing would make every
    // assertion below fail for the wrong reason — and a green one prove nothing at all.
    let scanned = scan_to_completion(&mut core);
    assert_eq!(
        scanned["foundRepos"],
        serde_json::json!(WANTED.len()),
        "the walk did not find the corpus, so nothing below is about the hand-off: {scanned}"
    );

    // 1. One row per repository.
    let rows = library(&mut core);
    let mut expected: Vec<String> = WANTED.iter().map(|n| (*n).to_owned()).collect();
    expected.sort();
    assert_eq!(
        names_of(&rows),
        expected,
        "a scan that discovers repositories and persists nothing answers with zero rows"
    );

    // 2. A value only a job produces.
    let computed = wait_for_a_computed_row(&mut core).unwrap_or_else(|| {
        panic!(
            "no row carried a computed observation within {COMPUTED_DEADLINE:?}; every project's \
             `refstateObservedAt` is still NULL, which is what a library with no scheduler looks \
             like"
        )
    });
    assert!(
        computed["branch"].is_string(),
        "J1 wrote its timestamp but no branch, which is a half-written reading: {computed}"
    );

    // 3. A second scan over an unchanged tree duplicates nothing.
    scan_to_completion(&mut core);
    let again = library(&mut core);
    assert_eq!(
        names_of(&again),
        expected,
        "the second scan added rows for repositories that were already indexed"
    );

    // 4. It exits.
    core.send("app.shutdown", &serde_json::json!({}));
    let until = Instant::now() + EXIT_DEADLINE;
    let status = loop {
        if let Some(status) = core.child.try_wait().expect("wait") {
            break status;
        }
        assert!(
            Instant::now() < until,
            "the core did not exit within {EXIT_DEADLINE:?}; a worker holding a publisher clone \
             keeps the transport join blocked"
        );
        std::thread::sleep(Duration::from_millis(100));
    };
    assert!(status.success(), "exited badly: {status:?}");
}

/// `Bytes` crosses as `{"b64": …}` (§2.5). Encoded here rather than through the core's own
/// serializer so the test states the wire form the shell would send.
fn base64_of(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}
