#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! **AC-P2-21-9** — assembly constructs the production provider **and** wraps p2-20's production
//! `ReqwestTransport` in §21's `ObservingTransport`, so the provider's transport is the decorated
//! one.
//!
//! §21.14's gate 8 says *"assembly constructs the real `GitHubApi`"*, and `GitHubApi` is declared
//! nowhere: R48 rules the typed seam is p2-20's `Provider` and R66 rules the raw seam is p2-20's
//! `HttpTransport`. **This plan's half is that the decorator is in the chain**, and the only check
//! that proves *that* is a **drained observation** — a reading check sees a constructor and cannot
//! see what the provider was handed.
//!
//! This is the shape that caught `NoDistros`: `core/src/main.rs` now wires
//! `SystemDistroProbe::system()` where a stand-in once stood.

use std::sync::Arc;

use codotheca_core::accounts::keychain::SecretToken;
use codotheca_core::http::{HttpResponse, HttpTransport};
use codotheca_core::testing::{FakeClock, FakeTransport};

const NOW: i64 = 1_800_000_000;

/// Every `.rs` the composition root spans. **Panics on an empty walk**: every assertion below is
/// *"this source says X"*, which zero files satisfies vacuously.
fn composition_sources() -> Vec<(std::path::PathBuf, String)> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut out = Vec::new();
    let mut stack = vec![root.join("assembly")];
    out.push((
        root.join("main.rs"),
        std::fs::read_to_string(root.join("main.rs")).expect("main.rs is readable"),
    ));
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("core/src/assembly is readable") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let text = std::fs::read_to_string(&path).expect("readable");
                out.push((path, text));
            }
        }
    }
    assert!(!out.is_empty(), "the walk found no composition sources");
    out
}

/// Source with every `//`-comment line removed, so a gate cannot be satisfied by the prose that
/// documents the rule it is checking.
fn strip_comments(source: &str) -> String {
    source
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// **AC-P2-21-9, the construction half.** The composition root builds its forge seam through the
/// **one** function that also builds the decorator, and constructs neither piece by hand.
///
/// This is deliberately not *"a decorator is constructed somewhere"*: a root that built one and
/// then handed the provider the bare transport would satisfy that, and the difference is the
/// whole risk. Routing both through `build_forge` is what makes the runtime assertion below cover
/// the production wiring rather than a chain the test assembled for itself (R88).
#[test]
fn the_composition_root_builds_its_forge_through_the_one_wiring_function() {
    let sources = composition_sources();
    let mut scanned = 0_usize;
    let mut hand_built: Vec<String> = Vec::new();
    let mut root_calls = 0_usize;
    for (path, text) in &sources {
        scanned += 1;
        let code = strip_comments(text);
        if path.ends_with("main.rs") {
            root_calls += code.matches("build_forge(").count();
            for needle in ["ObservingTransport::new(", "GitHubProvider::new("] {
                if code.contains(needle) {
                    hand_built.push(format!("{}: {needle}", path.display()));
                }
            }
        }
    }
    eprintln!(
        "sync_assembly: scanned {scanned} composition file(s), {root_calls} build_forge call(s)"
    );
    assert!(scanned > 0, "a gate that scanned nothing is a failing gate");
    assert_eq!(root_calls, 1, "one forge, assembled once");
    assert!(
        hand_built.is_empty(),
        "the root assembles a forge by hand, so the wiring below is not what ships: {hand_built:?}"
    );
}

/// No fake reaches the shipped binary. This is `NoDistros` from the other direction: a seam whose
/// production value is a stand-in compiles, passes, and is wrong only where nothing looks.
#[test]
fn no_test_double_appears_in_the_composition_root() {
    let sources = composition_sources();
    let mut scanned = 0_usize;
    let mut offenders: Vec<String> = Vec::new();
    for (path, text) in &sources {
        scanned += 1;
        // The inline `#[cfg(test)] mod tests` of a composition file is not the composition.
        let code = strip_comments(text);
        let production = code.split("#[cfg(test)]").next().unwrap_or(&code);
        for needle in [
            "FakeTransport",
            "FakeTokenStore",
            "FakeGitBackend",
            "FakeClock",
        ] {
            if production.contains(needle) {
                offenders.push(format!("{}: {needle}", path.display()));
            }
        }
    }
    eprintln!("sync_assembly: {scanned} composition file(s) checked for test doubles");
    assert!(scanned > 0);
    assert!(
        offenders.is_empty(),
        "a test double reached the composition root: {offenders:?}"
    );
}

/// **AC-P2-21-9, the half a reading check cannot make.** A request made through the **production
/// wiring** produces a drained observation.
///
/// `build_forge` is the function `core/src/main.rs` calls, so what is exercised here is what
/// ships. A gate that assembled its own provider over its own decorator would prove the decorator
/// works and say nothing about the composition root — which is R88's defect wearing a green tick.
#[test]
fn a_request_through_the_production_wiring_produces_an_observation() {
    let inner = Arc::new(FakeTransport::new());
    inner.push(HttpResponse {
        status: 200,
        headers: codotheca_core::http::normalise_headers([
            ("x-ratelimit-resource", "core"),
            ("x-ratelimit-remaining", "4321"),
        ]),
        body: br#"{"login":"someone"}"#.to_vec(),
    });
    let (provider, observing) = codotheca_core::assembly::sync::build_forge(
        Arc::clone(&inner) as Arc<dyn HttpTransport>,
        Arc::new(FakeClock::new(NOW)),
        "forge.example.invalid".to_owned(),
    );

    assert!(observing.drain().is_empty(), "nothing observed yet");
    provider
        .viewer(&SecretToken::new("t".to_owned()))
        .expect("the viewer read answers");
    let observed = observing.drain();
    eprintln!(
        "sync_assembly: {} observation(s) from one production-wired provider call",
        observed.len()
    );
    assert_eq!(observed.len(), 1, "the decorator is not in the chain");
    assert_eq!(observed[0].rate.remaining, Some(4321));
    assert_eq!(observed[0].rate.resource.as_deref(), Some("core"));
}

/// The **real binary** answers `sync.status` after the handshake, and answers it as *measured,
/// none*: empty arrays and two nulls, never a zero nobody observed.
///
/// A unit test over `CoreHandler` cannot see a composition root that forgot to start the pump or
/// to route the command; this is the shape `core/tests/assembly_startup.rs` established.
mod binary {
    use codotheca_core::proto::frame::{read_frame, write_frame};
    use std::io::Write as _;

    fn send(child: &mut std::process::Child, frame: &serde_json::Value) {
        let body = serde_json::to_vec(frame).expect("frame");
        let stdin = child.stdin.as_mut().expect("stdin");
        write_frame(stdin, &body).expect("write");
        stdin.flush().expect("flush");
    }

    fn recv(child: &mut std::process::Child) -> serde_json::Value {
        let stdout = child.stdout.as_mut().expect("stdout");
        let mut buf = Vec::new();
        read_frame(stdout, &mut buf).expect("read");
        serde_json::from_slice(&buf).expect("json")
    }

    #[test]
    fn the_real_binary_answers_sync_status_and_exits() {
        let dir = tempfile::tempdir().expect("tmp");
        let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_codotheca-core"))
            .arg(format!("--data-dir={}", dir.path().display()))
            .arg("--epoch=7")
            .arg(format!("--parent-pid={}", std::process::id()))
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("core spawns");

        let hello = recv(&mut child);
        assert_eq!(hello["t"], "hello", "{hello}");
        send(
            &mut child,
            &serde_json::json!({"t": "request", "id": 1, "command": "app.hello_ack", "args": {}}),
        );
        let _ack = recv(&mut child);

        send(
            &mut child,
            &serde_json::json!({"t": "request", "id": 2, "command": "sync.status", "args": {}}),
        );
        let reply = recv(&mut child);
        assert_eq!(reply["t"], "response", "{reply}");
        let status = &reply["ok"];
        assert_eq!(status["tasks"], serde_json::json!([]), "{reply}");
        assert_eq!(status["budgets"], serde_json::json!([]), "{reply}");
        assert!(status["listing"].is_null(), "{reply}");
        assert!(status["notice"].is_null(), "{reply}");

        // **The shutdown order.** `CoreHandler::shutdown` stops the sync pump beside the job
        // pump and before the publisher closes; a pump it did not stop would keep the process
        // alive past this wait.
        send(
            &mut child,
            &serde_json::json!({"t": "request", "id": 3, "command": "app.shutdown", "args": {}}),
        );
        let status = child.wait().expect("exits");
        assert!(status.success(), "{status:?}");
    }
}
